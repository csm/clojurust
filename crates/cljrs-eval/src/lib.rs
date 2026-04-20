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
//! - `ir_convert` — converts Clojure Value data → Rust `IrFunction`
//! - `lower` — bridges the Clojure compiler front-end to produce IR
//! - `apply` — IR-aware function dispatch with tree-walk fallback

// EvalError::Thrown wraps a full Value; boxing would require pervasive changes.
#![allow(clippy::result_large_err)]
// Namespace/GlobalEnv use Mutex<HashMap<Arc<str>, GcPtr<Var>>> — intentionally verbose for clarity.
#![allow(clippy::type_complexity)]

pub mod ir_cache;
pub mod ir_convert;
pub mod ir_interp;
pub mod lower;
pub mod apply;

pub use cljrs_env::callback::invoke;
pub use cljrs_env::env::{Env, GlobalEnv};
pub use cljrs_env::error::{EvalError, EvalResult};
pub use cljrs_interp::eval::eval;
pub use cljrs_env::loader::load_ns;

use std::sync::Arc;
use crate::ir_interp::eager_lower_fn;

pub fn register_compiler_sources(globals: &Arc<GlobalEnv>) {
    globals.register_builtin_source("cljrs.compiler.ir", cljrs_ir::COMPILER_IR_SOURCE);
    globals.register_builtin_source("cljrs.compiler.known", cljrs_ir::COMPILER_KNOWN_SOURCE);
    globals.register_builtin_source("cljrs.compiler.anf", cljrs_ir::COMPILER_ANF_SOURCE);
    globals.register_builtin_source("cljrs.compiler.escape", cljrs_ir::COMPILER_ESCAPE_SOURCE);
    globals.register_builtin_source("cljrs.compiler.optimize", cljrs_ir::COMPILER_OPTIMIZE_SOURCE);
}

/// Load the Clojure compiler namespaces and mark the compiler as ready
/// for IR lowering.  Called lazily on first lowering attempt.
pub fn ensure_compiler_loaded(globals: &Arc<GlobalEnv>, env: &mut Env) -> bool {
    // Already loaded?
    if globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return true;
    }

    // Don't load if CLJRS_NO_IR is set.
    if std::env::var("CLJRS_NO_IR").is_ok() {
        return false;
    }

    // Prevent concurrent loading attempts.
    static LOADING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if LOADING.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return false; // Another thread is loading.
    }

    let span = || cljrs_types::span::Span::new(Arc::new("<compiler-load>".to_string()), 0, 0, 1, 1);
    for ns_name in &[
        "cljrs.compiler.ir",
        "cljrs.compiler.known",
        "cljrs.compiler.anf",
    ] {
        let require_form = cljrs_reader::Form::new(
            cljrs_reader::form::FormKind::List(vec![
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol("require".into()),
                    span(),
                ),
                cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                        cljrs_reader::form::FormKind::Symbol((*ns_name).into()),
                        span(),
                    ))),
                    span(),
                ),
            ]),
            span(),
        );
        if let Err(e) = eval(&require_form, env) {
            eprintln!("[compiler-load warning] failed to load {ns_name}: {e:?}");
            LOADING.store(false, std::sync::atomic::Ordering::Release);
            return false;
        }
    }

    globals
        .compiler_ready
        .store(true, std::sync::atomic::Ordering::Release);
    LOADING.store(false, std::sync::atomic::Ordering::Release);
    true
}

pub fn standard_env_minimal() -> Arc<GlobalEnv> {
    cljrs_interp::standard_env_minimal(Some(eval), Some(apply::call_cljrs_fn), Some(eager_lower_fn))
}

pub fn standard_env() -> Arc<GlobalEnv> {
    let globals = standard_env_minimal();
    register_compiler_sources(&globals);
    globals
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