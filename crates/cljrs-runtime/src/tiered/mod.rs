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
pub mod backend;
pub mod defn_registry;
pub mod ir_cache;
pub mod ir_interp;
pub mod jit_state;
pub mod lower;
mod lower_worker;
pub mod tiers;

pub use crate::env::callback::invoke;
pub use crate::env::env::{Env, GlobalEnv};
pub use crate::env::error::{EvalError, EvalResult};
pub use crate::env::gc_roots::{force_collect, set_stw_reclaim_hook};
pub use crate::env::loader::load_ns;
pub use crate::interp::eval::{eval, eval_with_gas};

pub use apply::force_eager_lowering;
pub use backend::JitBackend;
pub use jit_state::{set_ir_threshold, set_jit_threshold, set_osr_threshold};
pub use tiers::Tiers;

use std::sync::Arc;

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
                    globals
                        .ir_cache()
                        .store(arity.ir_arity_id, Arc::new(ir_func.clone()));
                    loaded += 1;
                }
            }
        }
    }

    loaded
}
