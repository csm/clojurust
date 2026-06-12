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
            return call_jit_native(f, fn_ptr, epoch, arity, args, caller_env);
        }

        // 2. IR interpreter (also bumps invocation counter).
        if let Some(result) = try_ir_path(f, arity, args, caller_env) {
            return result;
        }

        // 3. No IR yet — count the tree-walked call; crossing the warm
        //    threshold requests background lowering (Phase 10.7).
        maybe_request_lowering(f, arity_id, caller_env);
    }

    // 4. Tree-walking interpreter.
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
/// Only executes IR that is already cached — published by the background
/// lowering worker once the function crossed the warm threshold (Phase 10.7),
/// eagerly lowered at definition time (`CLJRS_EAGER_LOWER`), or loaded from a
/// pre-built bundle.
fn try_ir_path(
    f: &CljxFn,
    arity: &cljrs_value::CljxFnArity,
    args: &[Value],
    caller_env: &mut Env,
) -> Option<EvalResult> {
    let arity_id = arity.ir_arity_id;

    // Cross-defn invalidation: if a defn this arity's lowering specialized
    // against was rebound, the cached IR was dropped and the arity marked
    // for re-lowering — request a background re-lower (one relaxed atomic
    // load on the fast path).  The mark is only *peeked* here; the lowering
    // worker consumes it, so a mark can never be lost between the worker
    // publishing IR and validating it against concurrent rebinds.
    // `lower_queued` (cleared when the rebind dropped the JitEntry)
    // deduplicates the request across calls.
    if crate::defn_registry::relower_pending()
        && crate::defn_registry::relower_marked(arity_id)
        && !IR_LOWERING_ACTIVE.get()
        && !crate::jit_state::lower_queued(arity_id)
    {
        request_background_lower(f, caller_env);
    }

    // Only use IR if already cached.
    let ir_func = crate::ir_cache::get_cached(arity_id)?;

    // Async IR functions fall back to tree-walking.
    if ir_func.is_async {
        return None;
    }

    // Bump invocation counter and the argument-type profile (drives
    // specialized compilation, Phase 10.6); enqueue JIT compilation when
    // hot.  Variadic arities profile only the fixed prefix — the rest-list
    // parameter is synthesized and must never be specialized.
    let profile_args = if arity.rest_param.is_some() {
        &args[..arity.params.len().min(args.len())]
    } else {
        args
    };
    crate::jit_state::record_call(arity_id, Arc::clone(&ir_func), profile_args);

    Some(execute_ir(f, arity, &ir_func, args, caller_env))
}

/// Whether all IR functionality is disabled (`CLJRS_NO_IR`).  Cached: the
/// warm path checks this on every tree-walked call.
fn no_ir() -> bool {
    static NO_IR: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NO_IR.get_or_init(|| std::env::var("CLJRS_NO_IR").is_ok())
}

/// Tier-0 warm-up accounting (Phase 10.7).
///
/// Counts a tree-walked call to `arity_id`; when the function gets warm
/// (`jit_state::ir_threshold`, default 50) its arity bodies are macro-expanded
/// here on the calling thread (macros need the interpreter) and shipped to
/// the background lowering worker.  Once the worker publishes the IR, step 2
/// of `call_cljrs_fn` takes over and the JIT pipeline proceeds as before.
fn maybe_request_lowering(f: &CljxFn, arity_id: u64, caller_env: &mut Env) {
    // Cheap gates first; all of these make the arity permanently
    // un-lowerable (or expansion unsafe), so skip before touching counters.
    //
    // - Inside a lowering/expansion already: don't recurse.
    // - Async fns: the IR interpreter refuses them (`try_ir_path`), so
    //   lowering would be wasted work.
    // - Capturing closures: lowering cannot see captures (every reference
    //   would mis-resolve as a global), same restriction as eager lowering.
    // - Bootstrap-era fns (defined before the compiler was ready): never
    //   lowered under eager lowering either; they stay at tree-walk.
    if IR_LOWERING_ACTIVE.get()
        || f.is_async
        || !f.closed_over_names.is_empty()
        || crate::jit_state::is_bootstrap_arity(arity_id)
        || no_ir()
        || !caller_env
            .globals
            .compiler_ready
            .load(std::sync::atomic::Ordering::Acquire)
    {
        return;
    }

    if !crate::jit_state::record_interp_call(arity_id) {
        return;
    }

    // Threshold crossed.  Shipped (builtin-source) namespaces — clojure.test,
    // clojure.string, the compiler namespaces, … — keep their historical
    // behavior and are never background-lowered: like the bootstrap, they
    // only ever reached the IR tiers under opt-in eager lowering, and some
    // of their patterns are known to miscompile (TODO.md Phase 10.7 notes).
    // Pin the queued flag so this is a one-time check per arity.
    if caller_env.globals.builtin_source(&f.defining_ns).is_some() {
        crate::jit_state::mark_lower_queued(arity_id);
        return;
    }

    // A previous lowering attempt may have marked this arity Unsupported
    // (e.g. an eager-lowering failure); pin the queued flag so the per-call
    // gates above stay the steady-state cost.
    if !crate::ir_cache::should_attempt(arity_id) {
        crate::jit_state::mark_lower_queued(arity_id);
        return;
    }

    request_background_lower(f, caller_env);
}

/// Snapshot `f` (macro-expanding every arity body on this thread) and enqueue
/// it for background lowering.  On acceptance, marks every arity of `f` as
/// queued; a full queue leaves the flags unset so a later call retries.
fn request_background_lower(f: &CljxFn, caller_env: &mut Env) {
    let arities: Vec<crate::lower_worker::LowerArityRequest> = f
        .arities
        .iter()
        .map(|a| crate::lower_worker::LowerArityRequest {
            arity_id: a.ir_arity_id,
            params: a.params.clone(),
            rest_param: a.rest_param.clone(),
            destructure_params: a.destructure_params.clone(),
            destructure_rest: a.destructure_rest.clone(),
            expanded_body: crate::lower::macroexpand_body(&a.body, caller_env),
        })
        .collect();
    let arity_ids: Vec<u64> = arities.iter().map(|a| a.arity_id).collect();

    let accepted = crate::lower_worker::enqueue(crate::lower_worker::LowerRequest {
        globals_id: crate::lower::globals_id(caller_env),
        name: f.name.clone(),
        ns: f.defining_ns.clone(),
        is_async: f.is_async,
        arities,
    });
    if accepted {
        for id in arity_ids {
            crate::jit_state::mark_lower_queued(id);
        }
    }
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
    f: &CljxFn,
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

    // Native code resolves globals (rt_load_global) and calls function values
    // (rt_call, the HOF bridges) through rt_abi, which dispatches via the
    // thread-local eval context — exactly as `execute_ir` pushes one for the
    // Tier-1 interpreter.  Without it every such bridge fails and silently
    // yields nil.
    let _eval_ctx = cljrs_env::callback::install_eval_context_guard(
        caller_env.globals.clone(),
        f.defining_ns.clone(),
    );

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

    // Deoptimization (Phase 10.6): a specialized compilation whose entry
    // type guard failed returns the deopt sentinel.  Guards run before any
    // side effect, so re-executing the whole call at Tier 1 is sound.  The
    // failure is counted; repeated violations discard the specialization
    // (record_deopt), after which dispatch returns to Tier 1 until a generic
    // recompile is published.
    if crate::jit_state::is_deopt_result(result_ptr) {
        crate::jit_state::record_deopt(arity.ir_arity_id);
        if let Some(ir_func) = crate::ir_cache::get_cached(arity.ir_arity_id) {
            return execute_ir(f, arity, &ir_func, args, caller_env);
        }
        return cljrs_interp::apply::call_cljrs_fn(f, args, caller_env);
    }

    // SAFETY: result_ptr was returned by rt_abi; it points to a live Value
    // in ALLOC_ROOTS.  Clone it before the alloc frame drops.
    let result = unsafe { (*result_ptr).clone() };

    // An uncaught `(throw …)` inside native code stashes the thrown value in a
    // thread-local and returns the nil sentinel.  Surface it as an error here
    // (while the alloc frame still roots it), exactly as Tier-1 would have
    // propagated it — and so a stale slot cannot misfire a later `rt_try`.
    if let Some(thrown) = crate::jit_state::take_pending_exception() {
        return Err(cljrs_env::error::EvalError::Thrown(thrown));
    }
    Ok(result)
}
