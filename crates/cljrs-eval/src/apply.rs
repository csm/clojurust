//! Extended apply routines, tries IR evaluation and falls back to tree-walking.
use std::sync::Arc;

use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_gc::GcPtr;
use cljrs_interp::apply::select_arity;
use cljrs_ir::IrFunction;
use cljrs_value::{CljxFn, CljxFnArity, PersistentList, Value};

use crate::jit_state;

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

    if !f.is_macro {
        // Tier 2 — JIT-native.  When enabled, the hottest arities run native
        // code; this also drives invocation counting and background queueing.
        if jit_state::enabled()
            && let Some(result) = try_jit_path(f, arity, args, caller_env)
        {
            return result;
        }

        // Tier 1 — IR interpreter (cached / prebuilt IR).
        if let Some(result) = try_ir_path(f, arity, args, caller_env) {
            return result;
        }
    }

    // Tier 0 — tree-walking interpreter.
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

    // Async IR functions fall back to tree-walking (eval_async in cljrs-async).
    // The IR interpreter is synchronous and cannot yield to the Tokio executor.
    if ir_func.is_async {
        return None;
    }

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

    // Build IR args: positional params map 1:1, but the rest param (if any)
    // must receive a list of the remaining args, not individual values.
    let ir_args = if arity.rest_param.is_some() {
        let n = arity.params.len();
        let mut ir_args = args[..n.min(args.len())].to_vec();
        let rest_items: Vec<Value> = args[n.min(args.len())..].to_vec();
        let rest_val = if rest_items.is_empty() {
            Value::Nil
        } else {
            Value::List(GcPtr::new(PersistentList::from_iter(rest_items)))
        };
        ir_args.push(rest_val);
        ir_args
    } else {
        args.to_vec()
    };

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

// ── Tier 2 — JIT-native dispatch ─────────────────────────────────────────────

/// Whether an arity is in the set the JIT can compile in Phase 10.1: the same
/// set Tier-1 already handles — no captures, no destructuring, no rest param.
fn jit_eligible_arity(f: &CljxFn, arity: &CljxFnArity) -> bool {
    !f.is_async
        && f.closed_over_names.is_empty()
        && arity.rest_param.is_none()
        && arity.destructure_params.is_empty()
        && arity.destructure_rest.is_none()
}

/// Whether a lowered IR function is JIT-able in Phase 10.1: a flat function
/// with no nested closures (subfunctions are deferred to Phase 10.3) and no
/// async yield points.
fn jit_eligible_ir(ir: &IrFunction) -> bool {
    ir.subfunctions.is_empty() && !ir.is_async
}

/// Attempt the JIT-native execution path.
///
/// - If native code is ready for this arity, runs it and returns the result.
/// - Otherwise bumps the invocation counter and, on crossing the threshold,
///   lowers the arity (if needed) and queues it for background compilation.
///
/// Returns `Some(result)` only when native code actually ran; `None` means the
/// caller should fall through to Tier 1 / Tier 0 for this call.
fn try_jit_path(
    f: &CljxFn,
    arity: &CljxFnArity,
    args: &[Value],
    caller_env: &mut Env,
) -> Option<EvalResult> {
    if !jit_eligible_arity(f, arity) {
        return None;
    }

    let arity_id = arity.ir_arity_id;
    let entry = jit_state::get_or_create(arity_id);

    // Already compiled → run native code.
    if let Some((code, _n_params)) = entry.ready_code() {
        return Some(call_native(f, code, args, caller_env));
    }

    // Count this invocation; queue for compilation on threshold crossing.
    let count = entry.bump();
    if count == jit_state::threshold() && entry.try_begin_queue(arity.params.len() as u8) {
        match lower_for_jit(f, arity, caller_env) {
            Some(ir) if jit_eligible_ir(&ir) => {
                jit_state::enqueue(arity_id, ir, arity.params.len() as u8);
            }
            _ => jit_state::mark_failed(arity_id),
        }
    }

    None
}

/// Obtain IR for an arity to hand to the JIT, lowering it on demand if needed.
///
/// Lowering runs on the calling (mutator) thread because it drives the
/// Clojure-implemented compiler front-end, which needs an eval context; the
/// background JIT thread only runs pure IR → native codegen.
///
/// This deliberately **never mutates the shared `ir_cache`**: the JIT must not
/// change which tier the interpreter chooses for a function.  It only *reads*
/// the cache so an already-lowered (eager/prebuilt) IR can be reused; otherwise
/// it lowers a private copy for the JIT.  `try_begin_queue` guarantees this runs
/// at most once per arity, so there is no repeated-lowering cost.
fn lower_for_jit(f: &CljxFn, arity: &CljxFnArity, caller_env: &mut Env) -> Option<Arc<IrFunction>> {
    let arity_id = arity.ir_arity_id;

    // Reuse IR the interpreter already lowered (eagerly or prebuilt).
    if let Some(ir) = crate::ir_cache::get_cached(arity_id) {
        return Some(ir);
    }
    // A prior interpreter attempt marked it unsupported — don't bother.
    if !crate::ir_cache::should_attempt(arity_id) {
        return None;
    }

    // Don't nest lowering calls (the compiler front-end is itself Clojure code).
    if IR_LOWERING_ACTIVE.get() {
        return None;
    }

    // Make sure the Clojure compiler namespaces are loaded.
    let globals = caller_env.globals.clone();
    if !crate::ensure_compiler_loaded(&globals, caller_env) {
        return None;
    }

    IR_LOWERING_ACTIVE.set(true);
    let result = crate::lower::lower_and_optimize_arity(
        f.name.as_deref(),
        &arity.params,
        arity.rest_param.as_ref(),
        &arity.body,
        &f.defining_ns,
        caller_env,
        f.is_async,
    );
    IR_LOWERING_ACTIVE.set(false);

    result.ok().map(Arc::new)
}

/// Execute finalized JIT-native code for one arity.
///
/// Mirrors the runtime setup the AOT harness performs: it installs an eval
/// context so the `rt_*` runtime bridge can resolve globals and dispatch
/// callbacks, roots the argument values for the GC, then invokes the native
/// `extern "C" fn(*const Value, ...) -> *const Value` through the registered
/// invoke hook.
fn call_native(f: &CljxFn, code: *const u8, args: &[Value], caller_env: &mut Env) -> EvalResult {
    // Root the caller's environment for the duration of any nested callbacks.
    let _caller_root = cljrs_env::gc_roots::push_env_root(caller_env);

    // Install the eval context (globals + defining namespace) the runtime
    // bridge reads via `capture_eval_context` / `invoke`.
    let env = Env::new(caller_env.globals.clone(), &f.defining_ns);
    cljrs_env::callback::push_eval_context(&env);

    // Root the arguments and hand the native code stable pointers into the
    // rooted Vec.  `rt_*` always clones values out of these pointers, so they
    // need only stay valid (and marked) for the duration of the call.
    let arg_values: Vec<Value> = args.to_vec();
    let _arg_root = cljrs_env::gc_roots::root_values(&arg_values);
    let arg_ptrs: Vec<*const Value> = arg_values.iter().map(|v| v as *const Value).collect();

    // Bound the in-flight allocations the native call registers as roots.
    let _alloc_frame = cljrs_gc::push_alloc_frame();

    let outcome = jit_state::invoke_native(code, &arg_ptrs);

    cljrs_env::callback::pop_eval_context();

    match outcome {
        Some(Ok(v)) => Ok(v),
        Some(Err(thrown)) => Err(EvalError::Thrown(thrown)),
        // No invoke hook registered: should not happen once `ready_code` is
        // set, but fall back to the interpreter rather than panicking.
        None => {
            let arity = select_arity(f, args.len())?;
            try_ir_path(f, arity, args, caller_env)
                .unwrap_or_else(|| cljrs_interp::apply::call_cljrs_fn(f, args, caller_env))
        }
    }
}
