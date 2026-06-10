//! Extended apply routines, tries IR evaluation and falls back to tree-walking.
use std::sync::Arc;

use cljrs_env::env::Env;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_interp::apply::select_arity;
use cljrs_value::{CljxFn, PersistentList, Value};

static EAGER_LOWER_FORCED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Force eager IR lowering to be enabled for all threads, regardless of
/// the `CLJRS_EAGER_LOWER` environment variable.  Called by `cljrs_jit::init`.
pub fn force_eager_lowering() {
    EAGER_LOWER_FORCED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Whether eager IR lowering at function definition time is enabled.
///
/// Eager lowering calls the Clojure compiler for every `fn*` definition,
/// which is expensive.  Disabled by default; set `CLJRS_EAGER_LOWER=1` to
/// enable.  (Or set `CLJRS_NO_IR=1` to disable all IR functionality.)
pub(crate) fn eager_lower_enabled() -> bool {
    if std::env::var("CLJRS_NO_IR").is_ok() {
        return false;
    }
    if EAGER_LOWER_FORCED.load(std::sync::atomic::Ordering::Relaxed) {
        return true;
    }
    std::env::var("CLJRS_EAGER_LOWER").is_ok()
}

pub fn call_cljrs_fn(f: &CljxFn, args: &[Value], caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    if !f.is_macro {
        let arity_id = arity.ir_arity_id;

        // 1. JIT-native: fastest path — skip interpreter entirely.
        if let Some((fn_ptr, epoch)) = crate::jit_state::get_native_fn(arity_id) {
            return call_jit_native(fn_ptr, epoch, arity, args, caller_env);
        }

        // 2. IR interpreter (also bumps invocation counter).
        if let Some(result) = try_ir_path(f, arity, args, caller_env) {
            return result;
        }
    }

    // 3. Tree-walking interpreter.
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

    // Async IR functions fall back to tree-walking.
    if ir_func.is_async {
        return None;
    }

    // Bump invocation counter; enqueue JIT compilation when hot.
    crate::jit_state::record_call(arity_id, Arc::clone(&ir_func));

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

    // OSR (Phase 10.4) is enabled here — this path has a stable arity
    // identity, so a hot loop inside a single call can promote to native
    // code mid-run.
    let result = crate::ir_interp::interpret_ir_with_osr(
        ir_func,
        ir_args,
        &caller_env.globals,
        &f.defining_ns,
        &mut env,
        Some(arity.ir_arity_id),
    );

    cljrs_env::callback::pop_eval_context();
    env.pop_frame();
    result
}

/// Invoke a JIT-compiled native function.
///
/// Roots the caller env and the argument slice on the GC shadow stack before
/// entering native code, so GC safepoints inside the JIT frame can find them.
///
/// For a variadic arity the compiled function's signature is
/// `(fixed…, rest_list)` — the IR lowers the rest parameter to a single value
/// that receives a list of the trailing arguments.  We therefore pack the
/// trailing args into a list here, exactly as [`execute_ir`] does for the IR
/// interpreter, so the native call receives the `arity.params.len() + 1`
/// arguments it was compiled for instead of the raw call argument count.
fn call_jit_native(
    fn_ptr: *const (),
    epoch: u64,
    arity: &cljrs_value::CljxFnArity,
    args: &[Value],
    caller_env: &mut Env,
) -> EvalResult {
    // Register this native frame's code epoch so code unloading cannot free the
    // backing module while it executes.  Pushed *before* entering native code
    // and *before* any safepoint can occur, and popped on return/unwind.
    let _jit_frame = crate::jit_state::push_jit_frame(epoch);
    // Register the caller env so its GcPtrs survive any GC triggered inside
    // the JIT frame (at rt_safepoint calls).
    let _caller_root = cljrs_env::gc_roots::push_env_root(caller_env);

    // Build the argument list the native code expects.  Fixed arities pass args
    // through unchanged; variadic arities pack the trailing args into the rest
    // list so the native arg count matches the compiled signature.
    let call_args: Vec<Value> = if arity.rest_param.is_some() {
        let n = arity.params.len();
        let split = n.min(args.len());
        let mut v = args[..split].to_vec();
        let rest_items = &args[split..];
        let rest_val = if rest_items.is_empty() {
            Value::Nil
        } else {
            Value::List(GcPtr::new(PersistentList::from_iter(rest_items.to_vec())))
        };
        v.push(rest_val);
        v
    } else {
        args.to_vec()
    };

    // Register the (owned) arg values on the shadow stack — including the freshly
    // built rest list — so they survive any GC triggered inside the JIT frame.
    let _arg_roots = cljrs_env::gc_roots::root_values(&call_args);
    // Track all allocations made inside the JIT frame.
    let _alloc_frame = cljrs_gc::push_alloc_frame();

    // Pass raw pointers to the args.  These are valid for the duration of this
    // call because `call_args` outlives the call and `_arg_roots` roots the
    // underlying Values.
    let arg_ptrs: Vec<*const Value> = call_args.iter().map(|v| v as *const Value).collect();

    // SAFETY: fn_ptr was produced by Cranelift JIT with SystemV ABI and the
    // correct number of *const Value params; all arg pointers are live.
    let result_ptr = unsafe { crate::jit_state::dispatch_jit_call(fn_ptr, &arg_ptrs) };

    // SAFETY: result_ptr was returned by rt_abi; it points to a live Value
    // in ALLOC_ROOTS.  Clone it before the alloc frame drops.
    let result = unsafe { (*result_ptr).clone() };
    Ok(result)
}
