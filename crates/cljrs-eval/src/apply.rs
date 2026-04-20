///! Extended apply routines, tries IR evaluation and falls back to tree-walking.

use cljrs_env::env::Env;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_interp::apply::select_arity;
use cljrs_value::{CljxFn, Value};

/// Whether eager IR lowering at function definition time is enabled.
///
/// Eager lowering calls the Clojure compiler for every `fn*` definition,
/// which is expensive.  Disabled by default; set `CLJRS_EAGER_LOWER=1` to
/// enable.  (Or set `CLJRS_NO_IR=1` to disable all IR functionality.)
pub(crate) fn eager_lower_enabled() -> bool {
    thread_local! {
        static ENABLED: bool = std::env::var("CLJRS_EAGER_LOWER").is_ok()
            && std::env::var("CLJRS_NO_IR").is_err();
    }
    ENABLED.with(|e| *e)
}

pub fn call_cljrs_fn(f: &CljxFn, args: &[Value], caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    // Try IR path if this isn't a macro and IR is cached (e.g. prebuilt).
    if !f.is_macro
        && let Some(result) = try_ir_path(f, arity, &args, caller_env)
    {
        return result;
    }

    // Tree-walking interpreter.
    cljrs_interp::apply::call_cljrs_fn(f, args, caller_env)
}

// Thread-local guard to prevent recursive IR lowering.
// When we're inside a lowering call (which invokes the Clojure compiler),
// we must not try to lower functions called by the compiler itself.
thread_local! {
    pub static IR_LOWERING_ACTIVE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Attempt to execute via the IR interpreter.
/// Returns `None` if IR is not available (not cached, lowering failed, etc.).
/// Returns `Some(result)` if IR execution was attempted.
///
/// Only uses the IR path for functions that were eagerly lowered at definition
/// time (in `eval_fn`).  Lazy on-call lowering is not done here because the
/// IR interpreter doesn't yet handle all patterns (e.g., complex compiler code).
fn try_ir_path(
    f: &CljxFn,
    arity: &cljrs_value::CljxFnArity,
    args: &[Value],
    caller_env: &mut Env,
) -> Option<EvalResult> {
    let arity_id = arity.ir_arity_id;

    // Only use IR if already cached (eagerly lowered at definition time).
    let ir_func = crate::ir_cache::get_cached(arity_id)?;
    Some(execute_ir(f, arity, &ir_func, args, caller_env))
}

/// Execute an IR function with the given arguments.
fn execute_ir(
    f: &CljxFn,
    arity: &cljrs_value::CljxFnArity,
    ir_func: &cljrs_ir::IrFunction,
    args: &[Value],
    caller_env: &mut Env,
) -> EvalResult {
    let _caller_root = cljrs_env::gc_roots::push_env_root(caller_env);
    let mut env = Env::with_closure(caller_env.globals.clone(), &f.defining_ns, f);

    // Bind params (including destructuring) into the env so LoadLocal can find them.
    env.push_frame();
    cljrs_interp::apply::bind_fn_params(arity, args, &mut env)?;

    // Self-reference for named functions.
    if let Some(ref name) = f.name {
        let self_val = Value::Fn(GcPtr::new(f.clone()));
        env.bind(name.clone(), self_val);
    }

    // Push eval context so IR closures (which use with_eval_context) can
    // call back into the interpreter.
    cljrs_env::callback::push_eval_context(&env);

    // Build IR args: the IR function's params are the positional params
    // (and rest param if variadic). These map to VarIds in the IR.
    let ir_args = args.to_vec();

    let result = crate::ir_interp::interpret_ir(
        ir_func,
        ir_args,
        &caller_env.globals,
        &f.defining_ns,
        &mut env,
    );

    cljrs_env::callback::pop_eval_context();
    env.pop_frame();
    result
}
