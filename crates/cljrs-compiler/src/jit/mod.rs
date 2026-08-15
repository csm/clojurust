//! In-process JIT compiler for clojurust — Phase 10.1.
//!
//! ## How it fits into the execution tiers
//!
//! ```text
//! call_cljrs_fn (cljrs-runtime/src/tiered/apply.rs)
//!     ↓ JIT-native   ← this module publishes compiled function pointers
//!     ↓ Tier-1 IR    ← invocation counter bumped here; enqueue when hot
//!     ↓ Tree-walk    ← universal fallback
//! ```
//!
//! The JIT shares `typeinfer`, `codegen`, and `rt_abi` with the AOT backends
//! directly — they are sibling modules of this one, not a separate package.
//!
//! ## Usage
//!
//! Attach a JIT to a runtime once it is built:
//!
//! ```rust,ignore
//! let runtime = Runtime::builder()
//!     .execution_mode(ExecutionMode::Tiered)
//!     .build()?;
//! cljrs_compiler::jit::install(&runtime);
//! ```
//!
//! [`install`] registers a [`Jit`] backend on that runtime — the object the
//! runtime's dispatch path calls to enqueue compiles, mark superseded code
//! stale, take a pending exception, and recognise a deoptimization.  All the
//! *state* those operations read and write (invocation counters, argument
//! profiles, published pointers, OSR entries) belongs to the runtime, in
//! `cljrs_runtime::tiered::jit_state::JitState`; this module owns only the
//! compiled machine code, in the process-wide [`code_cache`].
//!
//! Two runtimes in one process can therefore each have a JIT (or not) without
//! seeing each other's counters or promoting each other's functions; they
//! share one background worker thread and one code cache, which is what the
//! executable-memory accounting is about in the first place.
//!
//! Functions reach IR via warm-threshold background lowering (Phase 10.7,
//! owned by `cljrs-runtime`): once a function's tree-walked call count exceeds
//! `CLJRS_IR_THRESHOLD` (default 50) it is lowered to optimized IR in the
//! background and dispatch switches to the Tier-1 IR interpreter.  Hot
//! functions (Tier-1 call count exceeding `CLJRS_JIT_THRESHOLD`, default
//! 1000) are then compiled in the background; subsequent calls dispatch
//! directly to native code.  `CLJRS_EAGER_LOWER=1` restores eager lowering
//! at definition time.

use std::collections::HashSet;
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, OnceLock, Weak, mpsc};

use cljrs_ir::IrFunction;
use cljrs_runtime::Runtime;
use cljrs_runtime::env::env::Env;
use cljrs_runtime::tiered::backend::JitBackend;
use cljrs_runtime::tiered::tiers::Tiers;
use cljrs_value::Value;

mod async_jit;
pub mod code_cache;
mod jit_compiler;
mod jit_worker;
#[cfg(all(test, not(feature = "no-gc")))]
mod osr_integration;

/// The JIT backend attached to a runtime.
///
/// Cheap and stateless: the compile queue and the code cache it talks to are
/// process-wide (one worker thread, one executable-memory budget), and every
/// per-runtime table lives in the runtime.
pub struct Jit {
    queue: SyncSender<jit_worker::CompileRequest>,
}

impl JitBackend for Jit {
    fn enqueue_function(
        &self,
        tiers: Weak<Tiers>,
        arity_id: u64,
        ir_func: Arc<IrFunction>,
    ) -> bool {
        // Non-blocking: if the queue is full, skip this compile request.
        // The function keeps running at Tier 1 until the queue drains.
        self.queue
            .try_send(jit_worker::CompileRequest::Function {
                tiers,
                arity_id,
                ir_func,
            })
            .is_ok()
    }

    fn enqueue_osr(
        &self,
        tiers: Weak<Tiers>,
        arity_id: u64,
        header: u32,
        ir_func: Arc<IrFunction>,
    ) -> bool {
        self.queue
            .try_send(jit_worker::CompileRequest::Osr {
                tiers,
                arity_id,
                header,
                ir_func,
            })
            .is_ok()
    }

    fn mark_stale(&self, epoch: u64) {
        code_cache::mark_stale(epoch);
    }

    fn take_pending_exception(&self) -> Option<Value> {
        crate::rt_abi::take_pending_exception_value()
    }

    fn deopt_sentinel(&self) -> usize {
        crate::rt_abi::deopt_sentinel_addr()
    }

    fn compile_async_arity(&self, callee: &Value, nargs: usize, env: &mut Env) {
        async_jit::compile_async_arity(callee, nargs, env);
    }
}

/// The shared compile queue, and the process-wide seams that go with it.
///
/// Created on the first [`install`] call: one worker thread, one var-rebind
/// hook, one stop-the-world reclaim hook — all of which are about the
/// process's executable memory rather than any one runtime.
fn shared_queue() -> SyncSender<jit_worker::CompileRequest> {
    static QUEUE: OnceLock<SyncSender<jit_worker::CompileRequest>> = OnceLock::new();
    QUEUE
        .get_or_init(|| {
            let (tx, rx) = mpsc::sync_channel::<jit_worker::CompileRequest>(256);

            // Code unloading (Phase 10.2):
            //
            // 1. When a var holding a function is redefined, mark the old
            //    definition's compiled arities stale and stop dispatching to
            //    them.  `Var::bind` has no runtime handle, so this is
            //    necessarily process-wide.
            cljrs_value::set_var_rebind_hook(on_var_rebind);
            // 2. At each stop-the-world GC safepoint, reclaim stale modules
            //    that no frame is executing.  The frame scan is across
            //    threads, not runtimes.
            cljrs_runtime::tiered::set_stw_reclaim_hook(|| {
                code_cache::reclaim_at_stw();
            });

            jit_worker::start_worker(rx);
            tx
        })
        .clone()
}

/// Attach a JIT to `runtime`, so its hot arities compile to native code.
///
/// Idempotent per runtime: the first backend installed wins.  Compilation
/// only actually happens in a runtime whose execution mode reaches
/// [`TierState::Jit`](cljrs_runtime::TierState) — `ExecutionMode::TieredNoJit`
/// and `CLJRS_NO_IR` keep dispatch below Tier 2 with a backend installed.
pub fn install(runtime: &Runtime) {
    install_on(runtime.globals());
}

/// [`install`], for a caller holding the environment rather than the
/// [`Runtime`] handle (an AOT harness, an embedding host).
pub fn install_on(globals: &Arc<cljrs_runtime::env::env::GlobalEnv>) {
    globals.jit().install_backend(Arc::new(Jit {
        queue: shared_queue(),
    }));
}

/// Var-rebind hook: stale the native code of any arity that the *old* function
/// value carried but the *new* value does not.
///
/// Nulling the dispatch pointer (`JitState::take_native_epoch`) makes future
/// calls fall back to the interpreter immediately; the returned epoch is handed
/// to the code cache for reclamation once no frame is executing it.
///
/// `Var::bind` sees only the two values, so this runs over every live runtime.
/// Arity ids are process-unique, so at most one of them holds published code
/// for a given id and the rest are no-ops.
fn on_var_rebind(old: &Value, new: &Value) {
    let old_fn = match old {
        Value::Fn(f) => f,
        _ => return,
    };
    // Arities still present in the new binding must not be staled (e.g. when a
    // var is rebound to the same function object).
    let new_ids: HashSet<u64> = arity_ids(new);
    let live = cljrs_runtime::tiered::tiers::live();
    for arity in &old_fn.get().arities {
        let id = arity.ir_arity_id;
        if new_ids.contains(&id) {
            continue;
        }
        for tiers in &live {
            if let Some(epoch) = tiers.jit().take_native_epoch(id) {
                code_cache::mark_stale(epoch);
            }
            // OSR-entry code for loops inside the old definition is superseded
            // by the rebind just like its whole-function code.
            for epoch in tiers.jit().take_osr_epochs(id) {
                code_cache::mark_stale(epoch);
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturated_queue_rejects_function_and_osr_requests() {
        let (queue, _receiver) = mpsc::sync_channel(1);
        let tiers = Tiers::new(0xF400_0001);
        let ir = Arc::new(IrFunction::new(None, None));
        assert!(
            queue
                .try_send(jit_worker::CompileRequest::Function {
                    tiers: tiers.handle(),
                    arity_id: 1,
                    ir_func: ir.clone(),
                })
                .is_ok(),
            "first request fills the queue"
        );
        let jit = Jit { queue };

        assert!(!jit.enqueue_function(tiers.handle(), 2, ir.clone()));
        assert!(!jit.enqueue_osr(tiers.handle(), 3, 0, ir));
    }
}
