//! In-process JIT compiler for clojurust — Phase 10.1.
//!
//! ## How it fits into the execution tiers
//!
//! ```text
//! call_cljrs_fn (cljrs-eval/src/apply.rs)
//!     ↓ JIT-native   ← this crate publishes compiled function pointers
//!     ↓ Tier-1 IR    ← invocation counter bumped here; enqueue when hot
//!     ↓ Tree-walk    ← universal fallback
//! ```
//!
//! ## Usage
//!
//! Call [`init`] once at process startup (before any Clojure code runs):
//!
//! ```rust,ignore
//! cljrs_jit::init();
//! ```
//!
//! This:
//! 1. Installs an enqueue hook in `cljrs_eval::jit_state`.
//! 2. Spawns the background JIT worker thread.
//!
//! Functions reach IR via warm-threshold background lowering (Phase 10.7,
//! owned by `cljrs-eval`): once a function's tree-walked call count exceeds
//! `CLJRS_IR_THRESHOLD` (default 50) it is lowered to optimized IR in the
//! background and dispatch switches to the Tier-1 IR interpreter.  Hot
//! functions (Tier-1 call count exceeding `CLJRS_JIT_THRESHOLD`, default
//! 1000) are then compiled in the background; subsequent calls dispatch
//! directly to native code.  `CLJRS_EAGER_LOWER=1` restores eager lowering
//! at definition time.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use cljrs_ir::IrFunction;
use cljrs_value::Value;

mod async_jit;
pub mod code_cache;
mod jit_compiler;
mod jit_worker;
#[cfg(all(test, not(feature = "no-gc")))]
mod osr_integration;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialise the JIT tier.
///
/// Idempotent: safe to call multiple times, only initialises once.
///
/// Sets the JIT threshold to `CLJRS_JIT_THRESHOLD` (env) or 1000 (default).
/// Override the threshold before calling this with
/// [`cljrs_eval::jit_state::set_jit_threshold`].
pub fn init() {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }

    let (tx, rx) = mpsc::sync_channel::<jit_worker::CompileRequest>(256);

    // Register the enqueue hook so the IR dispatch path can hand us hot
    // functions.
    let fn_tx = tx.clone();
    cljrs_eval::set_enqueue_hook(move |arity_id, ir_func: Arc<IrFunction>| {
        // Non-blocking: if the queue is full, skip this compile request.
        // The function will keep running at Tier 1 until the queue drains.
        let _ = fn_tx.try_send(jit_worker::CompileRequest::Function { arity_id, ir_func });
    });

    // Register the OSR enqueue hook (Phase 10.4) so a hot loop back-edge can
    // request compilation of an OSR-entry variant mid-call.
    cljrs_eval::jit_state::set_osr_enqueue_hook(
        move |arity_id, header, ir_func: Arc<IrFunction>| {
            let _ = tx.try_send(jit_worker::CompileRequest::Osr {
                arity_id,
                header,
                ir_func,
            });
        },
    );

    // Exception propagation: let the dispatch seam (call_jit_native / OSR
    // entry) take an uncaught `(throw …)` stashed by native code and re-raise
    // it as `EvalError::Thrown`.  cljrs-eval cannot depend on cljrs-compiler,
    // so the taker is threaded through as a hook here.
    cljrs_eval::jit_state::set_pending_exception_hook(
        cljrs_compiler::rt_abi::take_pending_exception_value,
    );

    // Deoptimization (Phase 10.6): a specialized compilation's failed entry
    // guard returns rt_abi's sentinel pointer; the dispatch seam compares
    // result addresses against it via this hook and re-runs the call at
    // Tier 1.
    cljrs_eval::jit_state::set_deopt_sentinel_hook(cljrs_compiler::rt_abi::deopt_sentinel_addr);

    // Closure escape: when JIT code materializes a closure via `rt_make_fn*`,
    // the resulting GC-managed value captures a raw pointer into the executing
    // module.  The frame scan cannot see such values, so pin the module's
    // epoch — it is never unloaded (bounded leak, sound).
    cljrs_compiler::rt_abi::set_closure_escape_hook(|| {
        if let Some(epoch) = cljrs_eval::jit_state::current_jit_epoch() {
            code_cache::pin_epoch(epoch);
        }
    });

    // Code unloading (Phase 10.2):
    //
    // 1. When a var holding a function is redefined, mark the old definition's
    //    compiled arities stale and stop dispatching to them.
    cljrs_value::set_var_rebind_hook(on_var_rebind);
    //    Cross-defn invalidation (Phase 10.5) also stales native code of
    //    *dependents* of a rebound defn; it runs in cljrs-eval, which routes
    //    the staled epochs here through this hook.
    cljrs_eval::jit_state::set_stale_epoch_hook(code_cache::mark_stale);
    // 2. At each stop-the-world GC safepoint, reclaim stale modules that no
    //    frame is executing.
    cljrs_eval::set_stw_reclaim_hook(|| {
        code_cache::reclaim_at_stw();
    });

    // Async-JIT activation (Phase H): compile `^:async` arities to native poll
    // functions on first dispatch.  The async dispatcher (cljrs-async) calls
    // this hook because it cannot reach the compiler itself.
    cljrs_env::async_hook::set_async_compile_hook(async_jit::compile_async_arity);

    // Spawn the background compilation thread.
    jit_worker::start_worker(rx);
}

/// Var-rebind hook: stale the native code of any arity that the *old* function
/// value carried but the *new* value does not.
///
/// Nulling the dispatch pointer ([`jit_state::take_native_epoch`]) makes future
/// calls fall back to the interpreter immediately; the returned epoch is handed
/// to the code cache for reclamation once no frame is executing it.
fn on_var_rebind(old: &Value, new: &Value) {
    let old_fn = match old {
        Value::Fn(f) => f,
        _ => return,
    };
    // Arities still present in the new binding must not be staled (e.g. when a
    // var is rebound to the same function object).
    let new_ids: HashSet<u64> = arity_ids(new);
    for arity in &old_fn.get().arities {
        let id = arity.ir_arity_id;
        if new_ids.contains(&id) {
            continue;
        }
        if let Some(epoch) = cljrs_eval::jit_state::take_native_epoch(id) {
            code_cache::mark_stale(epoch);
        }
        // OSR-entry code for loops inside the old definition is superseded by
        // the rebind just like its whole-function code.
        for epoch in cljrs_eval::jit_state::take_osr_epochs(id) {
            code_cache::mark_stale(epoch);
        }
    }
}

/// Collect the set of `ir_arity_id`s carried by a function value (empty for
/// non-function values).
fn arity_ids(value: &Value) -> HashSet<u64> {
    match value {
        Value::Fn(f) => f.get().arities.iter().map(|a| a.ir_arity_id).collect(),
        _ => HashSet::new(),
    }
}
