#![allow(clippy::arc_with_non_send_sync)]
//! IR-accelerated evaluation for clojurust.
//!
//! Wraps the tree-walking interpreter (`cljrs-interp`) with IR lowering and
//! interpretation.  When a function has been lowered to IR (eagerly at
//! definition time or from a pre-built cache), calls are dispatched to the
//! tier-1 IR interpreter; otherwise they fall back to tree-walking.
//!
//! Key components:
//! - `ir_interp` — tier-1 IR interpreter (register-file execution of `IrFunction`)
//! - `ir_cache` — thread-safe cache of lowered IR keyed by arity ID
//! - `lower` — orchestrates the pure-Rust `cljrs_ir::lower` pipeline to produce IR
//! - `apply` — IR-aware function dispatch with tree-walk fallback

// EvalError::Thrown wraps a full Value; boxing would require pervasive changes.
#![allow(clippy::result_large_err)]
// Namespace/GlobalEnv use Mutex<HashMap<Arc<str>, GcPtr<Var>>> — intentionally verbose for clarity.
#![allow(clippy::type_complexity)]

pub mod apply;
pub mod defn_registry;
pub mod ir_cache;
pub mod ir_interp;
pub mod jit_state;
pub mod lower;
mod lower_worker;

pub use cljrs_env::callback::invoke;
pub use cljrs_env::env::{Env, GlobalEnv};
pub use cljrs_env::error::{EvalError, EvalResult};
pub use cljrs_env::gc_roots::{force_collect, set_stw_reclaim_hook};
pub use cljrs_env::loader::load_ns;
pub use cljrs_interp::eval::eval;

pub use apply::force_eager_lowering;
pub use jit_state::{
    set_enqueue_hook, set_ir_threshold, set_jit_threshold, set_osr_threshold, store_native_fn,
};

use crate::ir_interp::eager_lower_fn;
use std::sync::Arc;

/// Mark the IR compiler as ready and snapshot the bootstrap arity watermark.
///
/// IR lowering is pure Rust (`cljrs_ir::lower`, orchestrated by the `lower`
/// module) — there is nothing to load.  This flips `compiler_ready`, the gate
/// for eager and background lowering, and records the bootstrap watermark so
/// functions defined before this point (the clojure.core bootstrap) stay
/// excluded from background lowering (Phase 10.7).
///
/// Returns `false` (leaving lowering disabled) when `CLJRS_NO_IR` is set.
pub fn mark_compiler_ready(globals: &Arc<GlobalEnv>) -> bool {
    if globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return true;
    }

    if std::env::var("CLJRS_NO_IR").is_ok() {
        return false;
    }

    globals
        .compiler_ready
        .store(true, std::sync::atomic::Ordering::Release);
    jit_state::set_bootstrap_arity_watermark(cljrs_interp::arity::next_arity_id());
    true
}

pub fn standard_env_minimal() -> Arc<GlobalEnv> {
    cljrs_interp::standard_env_minimal(Some(eval), Some(apply::call_cljrs_fn), Some(eager_lower_fn))
}

/// Like `standard_env_minimal` but without IR lowering.  Use this when IR
/// generation is not needed (e.g. the AOT test harness) to avoid populating
/// the IR cache with entries that will never be evicted.
pub fn standard_env_minimal_no_ir() -> Arc<GlobalEnv> {
    cljrs_interp::standard_env_minimal(Some(eval), Some(apply::call_cljrs_fn), None)
}

pub fn standard_env() -> Arc<GlobalEnv> {
    standard_env_minimal()
}

pub fn standard_env_with_paths(source_paths: Vec<std::path::PathBuf>) -> Arc<GlobalEnv> {
    let globals = standard_env();
    globals.set_source_paths(source_paths);
    globals
}

/// Load pre-built IR from a serialized bundle into the IR cache.
///
/// Walks all namespaces in the `GlobalEnv`, and for each function var whose
/// arity matches a bundle key (`"ns/name:param_count"` or `"ns/name:param_count+"`
/// for variadic), stores the pre-built IR in the cache keyed by the runtime
/// `ir_arity_id`.
///
/// Returns the number of arities successfully loaded.
pub fn load_prebuilt_ir(globals: &Arc<GlobalEnv>, bundle: &cljrs_ir::IrBundle) -> usize {
    use cljrs_value::Value;

    let ns_map = globals.namespaces.read().unwrap();
    let mut loaded = 0usize;

    for (ns_name, ns_ptr) in ns_map.iter() {
        let interns = ns_ptr.get().interns.lock().unwrap();
        for (var_name, var) in interns.iter() {
            let val = var.get().deref().unwrap_or(Value::Nil);
            let f = match &val {
                Value::Fn(gc_fn) => gc_fn.get(),
                _ => continue,
            };
            if f.is_macro {
                continue;
            }

            for arity in &f.arities {
                let key = if arity.rest_param.is_some() {
                    format!("{ns_name}/{var_name}:{}+", arity.params.len())
                } else {
                    format!("{ns_name}/{var_name}:{}", arity.params.len())
                };

                if let Some(ir_func) = bundle.get(&key) {
                    ir_cache::store_cached(arity.ir_arity_id, Arc::new(ir_func.clone()));
                    loaded += 1;
                }
            }
        }
    }

    loaded
}
