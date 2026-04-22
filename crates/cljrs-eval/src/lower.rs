//! Clojure-based IR lowering orchestration.
//!
//! This module bridges the Clojure compiler front-end (`cljrs.compiler.anf`)
//! with the Rust IR interpreter.  It calls the Clojure `lower-fn-body`
//! function to lower macro-expanded Form ASTs to IR data, then converts
//! the result to a Rust `IrFunction` via `ir_convert`.

use std::sync::Arc;

use cljrs_ir::IrFunction;
use cljrs_reader::Form;
use cljrs_value::Value;

use cljrs_env::env::Env;

// ── Error type ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LowerError {
    /// The compiler namespaces are not loaded yet (still bootstrapping).
    NotReady,
    /// The Clojure lowering function returned an error.
    LowerFailed(String),
    /// IR conversion from Clojure data to Rust types failed.
    ConvertFailed(String),
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::NotReady => write!(f, "compiler not ready"),
            LowerError::LowerFailed(msg) => write!(f, "lowering failed: {msg}"),
            LowerError::ConvertFailed(msg) => write!(f, "IR conversion failed: {msg}"),
        }
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Lower a function arity's body to IR via the Clojure compiler front-end.
///
/// Returns `Err(NotReady)` if the compiler namespaces haven't been loaded.
/// Returns `Err(LowerFailed)` if the Clojure lowering function fails.
/// Returns `Err(ConvertFailed)` if the Clojure data → Rust IR conversion fails.
pub fn lower_arity(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
) -> Result<IrFunction, LowerError> {
    // Check if compiler is ready.
    if !env
        .globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(LowerError::NotReady);
    }

    lower_arity_inner(name, params, rest_param, body, ns, env)
}

fn lower_arity_inner(
    name: Option<&str>,
    params: &[Arc<str>],
    rest_param: Option<&Arc<str>>,
    body: &[Form],
    ns: &Arc<str>,
    env: &mut Env,
) -> Result<IrFunction, LowerError> {
    use cljrs_gc::GcPtr;
    use cljrs_value::collections::vector::PersistentVector;

    let globals = &env.globals;

    // Look up the lower-fn-body function.
    let lower_fn = globals
        .lookup_var_in_ns("cljrs.compiler.anf", "lower-fn-body")
        .ok_or_else(|| {
            LowerError::LowerFailed("cljrs.compiler.anf/lower-fn-body not found".to_string())
        })?;
    let lower_fn_val = lower_fn.get().deref().unwrap_or(Value::Nil);

    // Macro-expand the body forms before lowering to IR.
    // The ANF compiler does not expand macros; we must do it here so that
    // macro calls (e.g. `cond`, `when`, `and`) become their `if`-chain
    // expansions rather than unresolvable function calls at runtime.
    let expanded_body: Vec<Form> = body
        .iter()
        .map(|f| cljrs_interp::macros::macroexpand_all(f, env).unwrap_or_else(|_| f.clone()))
        .collect();
    let body = expanded_body.as_slice();

    // Build arguments:
    // 1. fname (string or nil)
    let fname_val = match name {
        Some(n) => Value::string(n.to_string()),
        None => Value::Nil,
    };

    // 2. ns (string)
    let ns_val = Value::string(ns.to_string());

    // 3. params (vector of strings) — includes rest param if present
    let mut param_strs: Vec<Value> = params
        .iter()
        .map(|p| Value::string(p.to_string()))
        .collect();
    if let Some(rest) = rest_param {
        // The Clojure lowerer expects all params including rest as a flat list.
        // The last param in a variadic arity is the rest param.
        param_strs.push(Value::string(rest.to_string()));
    }
    let params_val = Value::Vector(GcPtr::new(PersistentVector::from_iter(param_strs)));

    // 4. body-forms (vector of form values)
    let body_forms_val = Value::Vector(GcPtr::new(PersistentVector::from_iter(
        body.iter().map(cljrs_builtins::form::form_to_value),
    )));

    // Push eval context so callback::invoke can work.
    cljrs_env::callback::push_eval_context(env);

    // Set IR_LOWERING_ACTIVE to prevent eager lowering of closures
    // created inside the Clojure compiler during this lowering call.
    use crate::apply::IR_LOWERING_ACTIVE;
    let was_active = IR_LOWERING_ACTIVE.get();
    IR_LOWERING_ACTIVE.set(true);

    // Call the Clojure lowering function.
    let ir_data = cljrs_env::callback::invoke(
        &lower_fn_val,
        vec![fname_val, ns_val, params_val, body_forms_val],
    );

    // Restore lowering flag and pop eval context.
    IR_LOWERING_ACTIVE.with(|c| c.set(was_active));
    cljrs_env::callback::pop_eval_context();

    let ir_data = ir_data.map_err(|e| LowerError::LowerFailed(format!("{e:?}")))?;

    // Convert the result Value → IrFunction.
    crate::ir_convert::value_to_ir_function(&ir_data)
        .map_err(|e| LowerError::ConvertFailed(format!("{e}")))
}
