// `EvalResult`'s error is `cljrs_runtime`'s `EvalError`, returned by value from
// every evaluator entry point; the synchronous side allows this lint crate-wide
// (`cljrs-runtime/src/lib.rs`) and `runtime.rs` allows it per function.  Clippy
// only began flagging `async fn`s for it in 1.98 — the code below is unchanged
// from when it passed under 1.94.
#![allow(clippy::result_large_err)]

//! Asynchronous tree-walking evaluation for `^:async` function bodies.
//!
//! [`eval_async`] mirrors the synchronous [`cljrs_runtime::interp::eval::eval`] for the
//! forms where an `await` can legitimately appear — `await` itself, `do`, `if`,
//! `let`/`let*`, `loop`/`loop*`, `recur`, `try`, `def`, `defonce`, `and`, `or`,
//! `throw`, `set!`, `letfn`, `binding`, `with-out-str`, collection literals, and
//! function-call arguments — and delegates every other form to the synchronous
//! evaluator. Any sub-expression a delegated form evaluates therefore takes the
//! blocking `await` path, which on the single-threaded `LocalSet` deadlocks
//! whenever the awaited future is not already settled: a form that can carry an
//! `await` needs an arm here, not a fallthrough. When it reaches an `(await x)` it
//! cooperatively yields to the Tokio `LocalSet` executor until the awaited
//! `Future`/`Promise` resolves, instead of blocking the OS thread the way the
//! sync `await` fallback does.
//!
//! Forms that the sync evaluator macro-expands (`when`, `cond`, `->`, …) are
//! expanded here first via [`cljrs_runtime::interp::macros::macroexpand`] so their
//! desugared `if`/`do`/`let` shapes are handled with proper yielding.

use std::future::Future;

use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_runtime::builtins::form::{expand_pairs, expand_reader_conds_cow};
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::error::{EvalError, EvalResult, value_error_to_eval_error};
use cljrs_runtime::interp::apply::{bind_fn_params, select_arity};
use cljrs_runtime::interp::destructure::{bind_pattern, value_to_seq_vec};
use cljrs_runtime::interp::eval::{eval, is_special_form};
use cljrs_runtime::interp::macros::macroexpand;
use cljrs_value::value::SetValue;
use cljrs_value::{
    CljxFnArity, CljxFuture, FutureState, MapValue, PersistentHashSet, PersistentList,
    PersistentVector, Value,
};

// Managed captures must be visible before the first task poll. Keep their
// shadow roots in a detached state so dropping a task uses the same registry.
#[cfg(not(feature = "no-gc"))]
struct CapturedRoots {
    _values: Box<[Value]>,
    state: cljrs_gc::mutator::MutatorState,
    guard: Option<cljrs_runtime::env::gc_roots::ValueRootGuard>,
}
#[cfg(not(feature = "no-gc"))]
impl CapturedRoots {
    fn new(values: Vec<Value>) -> Self {
        let values = values.into_boxed_slice();
        let mut state = cljrs_gc::mutator::MutatorState::fresh();
        state.swap_with_current();
        let guard = Some(cljrs_runtime::env::gc_roots::root_values(&values));
        state.swap_with_current();
        Self {
            _values: values,
            state,
            guard,
        }
    }
}
#[cfg(not(feature = "no-gc"))]
impl Drop for CapturedRoots {
    fn drop(&mut self) {
        self.state.swap_with_current();
        self.guard.take();
        self.state.swap_with_current();
    }
}

/// Spawn `task` on the current `LocalSet` and return a `Value::Future` that the
/// task settles on completion. The single delivery point for every async
/// primitive (`^:async` calls, `timeout`, `alts`).
///
/// Public so other native crates (e.g. `cljrs-io`) can drive their own async
/// work onto the shared executor and deliver results through the same `Future`
/// machinery.
///
/// Must be called from within a Tokio `LocalSet` context.
pub fn spawn_future<F>(task: F) -> Value
where
    F: Future<Output = EvalResult> + 'static,
{
    spawn_future_with_roots(Vec::new(), task)
}

/// Spawn a task with explicit managed captures. Use this for borrowed values
/// that can outlive their current cell binding or evaluation read scope.
pub fn spawn_future_with_roots<F>(roots: Vec<Value>, task: F) -> Value
where
    F: Future<Output = EvalResult> + 'static,
{
    // GC builds: heap-promotion fallback — the task's captured environment is
    // opaque to the publish-barrier scan, and the task may run after any
    // bump-region scope active right now has closed.  Poison the active
    // regions so they are retired (kept alive) instead of reset; a no-op (one
    // thread-local read) when no region is open, which is the common case.
    #[cfg(not(feature = "no-gc"))]
    cljrs_gc::region::poison_active_regions();
    #[cfg(feature = "no-gc")]
    let lease = {
        let mut lease = cljrs_gc::region::RegionLease::capture_active();
        lease.extend(&cljrs_gc::region::RegionLease::for_values(&roots));
        lease
    };
    #[cfg(feature = "no-gc")]
    let allocation = cljrs_gc::region::AllocationSnapshot::capture();
    let future = GcPtr::new(CljxFuture::new_local());
    let task_future = future.clone();
    #[cfg(not(feature = "no-gc"))]
    let captures = {
        let mut roots = roots;
        roots.push(Value::Future(task_future.clone()));
        CapturedRoots::new(roots)
    };
    #[cfg(feature = "no-gc")]
    let _ = roots;
    let gas_meters = cljrs_runtime::env::gas::active_meters();
    let interrupts = cljrs_runtime::env::gas::active_interrupts();
    tokio::task::spawn_local(async move {
        #[cfg(not(feature = "no-gc"))]
        let _captures = captures;
        #[cfg(feature = "no-gc")]
        let mut reads = cljrs_gc::region::ReadContext::default();
        let mut task = Box::pin(task);
        let result = std::future::poll_fn(|cx| {
            // LocalSet tasks share an OS thread, so TLS state must be scoped
            // to one poll and removed before another task can run. Meters
            // stack on the driver's; interrupt flags replace them, so this
            // task answers only to an interrupt of the eval that spawned it.
            let _interrupts = cljrs_runtime::env::gas::InterruptScope::enter(&interrupts);
            let _gas_guards = cljrs_runtime::env::gas::install_meters(&gas_meters);
            let _monitors = cljrs_runtime::env::monitor::AsyncMonitorScope::enter();
            #[cfg(feature = "no-gc")]
            let _lease_scope = lease.enter();
            #[cfg(feature = "no-gc")]
            let _allocation_scope = allocation.enter();
            #[cfg(feature = "no-gc")]
            let _reads = reads.enter();
            task.as_mut().poll(cx)
        })
        .await;
        #[cfg(feature = "no-gc")]
        let _lease_scope = lease.enter();
        #[cfg(feature = "no-gc")]
        let _allocation_scope = allocation.enter();
        settle_future(&task_future, result);
    });
    Value::Future(future)
}

/// Write a completed result into a future and wake blocking `deref` waiters.
pub(crate) fn settle_future(future: &GcPtr<CljxFuture>, result: EvalResult) {
    let state = future.get().state.lock().unwrap();
    // Cancellation is sticky. `future-cancel` does not interrupt the task, so a
    // task cancelled mid-body still runs to completion and lands here; letting
    // it write its result would flip `future-cancelled?` back to false and hand
    // `await` a value it had already promised to raise on. Cancel wins: the
    // side effects happened, the result is discarded.
    if matches!(&*state, FutureState::Cancelled) {
        return;
    }
    drop(state);
    let settled = match result {
        Ok(v) => FutureState::Done(cljrs_value::publish::publish_for_owner(
            v,
            future.get() as *const _ as usize,
        )),
        // Preserve the thrown value (and any non-Thrown error as a fresh
        // Value::Error) so `await` can re-throw it with ex-data/ex-cause intact.
        Err(EvalError::GasExhausted) => FutureState::GasExhausted,
        Err(EvalError::MemoryLimitExceeded(fault)) => FutureState::MemoryLimitExceeded(fault),
        Err(e) => FutureState::Failed(cljrs_value::publish::publish_for_owner(
            e.to_error_value(),
            future.get() as *const _ as usize,
        )),
    };
    let mut state = future.get().state.lock().unwrap();
    if matches!(&*state, FutureState::Cancelled) {
        return;
    }
    *state = settled;
    drop(state);
    future.get().cond.notify_all();
    future.get().ready.notify_all();
}

/// Run the body of an `^:async` function to completion, yielding at every
/// `await`. Returns the value of the last body form.
///
/// `callee` must be a `Value::Fn`; `args` are the already-evaluated call
/// arguments. A fresh environment is built from the function's closure with
/// `is_async = true` so nested `await`s take the yielding path.
pub async fn run_async_fn(callee: Value, args: Vec<Value>, base: &Env) -> EvalResult {
    let f = match &callee {
        Value::Fn(f) => f.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "async dispatch expected a fn, got {}",
                other.type_name()
            )));
        }
    };
    let arity = select_arity(&f, args.len())?.clone();
    let mut env = Env::with_closure(base.globals.clone(), &f.defining_ns, &f);
    env.is_async = true;

    // Keep callee and the local env alive across GC cycles at async yield points.
    let _callee_root = cljrs_runtime::env::gc_roots::root_value(&callee);
    let _env_root = cljrs_runtime::env::gc_roots::push_env_root(&env);

    let mut current_args = args;
    loop {
        env.push_frame();
        bind_fn_params(&arity, &current_args, &mut env)?;
        if let Some(name) = &f.name {
            env.bind(name.clone(), callee.clone());
        }

        let mut result = Ok(Value::Nil);
        for form in &arity.body {
            result = Box::pin(eval_async(form, &mut env)).await;
            if result.is_err() {
                break;
            }
        }
        env.pop_frame();

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_args)) => {
                current_args = flatten_recur_args(&arity, new_args)?;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Asynchronously evaluate a single form.
pub async fn eval_async(form: &Form, env: &mut Env) -> EvalResult {
    #[cfg(feature = "no-gc")]
    {
        let mut reads = cljrs_gc::region::ReadContext::default();
        let mut evaluation = Box::pin(eval_async_inner(form, env));
        let result = std::future::poll_fn(|cx| {
            let _reads = reads.enter();
            evaluation.as_mut().poll(cx)
        })
        .await;
        cljrs_runtime::interp::eval::publish_result(result, None)
    }
    #[cfg(not(feature = "no-gc"))]
    eval_async_inner(form, env).await
}

async fn eval_async_inner(form: &Form, env: &mut Env) -> EvalResult {
    // Collection literals may contain `await` sub-expressions (e.g. the
    // return value of a ^:async fn is `[(await x) (await y)]`). Evaluate
    // each element with eval_async so awaits yield cooperatively instead of
    // blocking the LocalSet thread via the sync condvar path.
    match &form.kind {
        FormKind::Vector(elems) => {
            let elems = expand_reader_conds_cow(elems).into_owned();
            let mut vals: Vec<Value> = Vec::with_capacity(elems.len());
            for f in &elems {
                vals.push(Box::pin(eval_async_inner(f, env)).await?);
            }
            return Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(vals))));
        }
        FormKind::Map(elems) => {
            let elems = expand_pairs(elems)
                .map_err(|_| {
                    EvalError::Runtime("map literal must have an even number of forms".into())
                })?
                .into_owned();
            let mut pairs: Vec<Value> = Vec::with_capacity(elems.len());
            for f in &elems {
                pairs.push(Box::pin(eval_async_inner(f, env)).await?);
            }
            let kv_pairs: Vec<(Value, Value)> = pairs
                .chunks(2)
                .map(|pair| (pair[0].clone(), pair[1].clone()))
                .collect();
            return Ok(Value::Map(MapValue::from_pairs(kv_pairs)));
        }
        FormKind::Set(elems) => {
            let elems = expand_reader_conds_cow(elems).into_owned();
            let mut vals: Vec<Value> = Vec::with_capacity(elems.len());
            for f in &elems {
                vals.push(Box::pin(eval_async_inner(f, env)).await?);
            }
            return Ok(Value::Set(SetValue::Hash(GcPtr::new(
                PersistentHashSet::from_iter(vals),
            ))));
        }
        FormKind::List(_) => {} // fall through to list handling below
        _ => return eval(form, env),
    }

    // Reduce control-flow macros (when, cond, ->, …) to their special-form core
    // so awaits nested inside them take the yielding path.
    let expanded = macroexpand(form, env)?;
    let forms = match &expanded.kind {
        FormKind::List(forms) if !forms.is_empty() => forms,
        _ => return eval(&expanded, env),
    };

    let forms_cow = expand_reader_conds_cow(forms);
    if forms_cow.is_empty() {
        return Ok(Value::List(GcPtr::new(PersistentList::empty())));
    }
    let forms: &[Form] = &forms_cow;

    if let FormKind::Symbol(s) = &forms[0].kind {
        match s.as_str() {
            "await" => return eval_await_async(&forms[1..], env).await,
            "do" => return eval_body_async(&forms[1..], env).await,
            "if" => return eval_if_async(&forms[1..], env).await,
            "let*" | "let" => return eval_let_async(&forms[1..], env).await,
            // loop/loop* needs an async handler so that `await` inside the body
            // yields correctly instead of falling back to blocking deref.
            "loop*" | "loop" => return eval_loop_async(&forms[1..], env).await,
            // try/catch/finally must yield so `await`/`<?` inside the body (and
            // inside catch bodies) cooperate with the executor instead of taking
            // the blocking sync path.
            "try" => return eval_try_async(&forms[1..], env).await,
            // `recur` needs an async handler because its *arguments* may await
            // (`(recur (conj acc (<? ch)) (inc i))`). The sync `eval_recur`
            // would evaluate them on the blocking deref path, parking the
            // single LocalSet thread forever.
            "recur" => return eval_recur_async(&forms[1..], env).await,
            // The remaining forms that evaluate a sub-expression in place (as
            // opposed to registering a body to run later, like `fn`/`defn`)
            // each need their own arm for the same reason as `recur`.
            "def" => return eval_def_async(&forms[1..], env).await,
            "defonce" => return eval_defonce_async(&forms[1..], env).await,
            "and" => return eval_and_async(&forms[1..], env).await,
            "or" => return eval_or_async(&forms[1..], env).await,
            "throw" => return eval_throw_async(&forms[1..], env).await,
            "set!" => return eval_set_bang_async(&forms[1..], env).await,
            "letfn" => return eval_letfn_async(&forms[1..], env).await,
            "binding" => return eval_binding_async(&forms[1..], env).await,
            "with-out-str" => return eval_with_out_str_async(&forms[1..], env).await,
            // What is left either evaluates nothing an `await` could sit in
            // (`quote`, `var`, `fn`, `defn`, `ns`, …) or is `.`, which the
            // sync evaluator rejects outright. Run them synchronously.
            other if is_special_form(other) => return eval(&expanded, env),
            _ => {}
        }
        return eval_call_async(&forms[0], &forms[1..], &expanded, env).await;
    }

    // Non-symbol head (e.g. `((f) args)`): no yielding in Phase B.
    eval(&expanded, env)
}

/// `(await x)` — evaluate `x`, then yield until the resulting future/promise
/// resolves.
async fn eval_await_async(args: &[Form], env: &mut Env) -> EvalResult {
    let Some(arg) = args.first() else {
        return Err(EvalError::Runtime("await requires one argument".into()));
    };
    let val = Box::pin(eval_async_inner(arg, env)).await?;
    await_value(val).await
}

/// Cooperatively await a Clojure value. Futures and promises yield to the
/// executor until resolved; any other value is returned as-is.
pub async fn await_value(val: Value) -> EvalResult {
    #[cfg(feature = "no-gc")]
    {
        let mut reads = cljrs_gc::region::ReadContext::default();
        let mut wait = Box::pin(await_value_inner(val));
        let result = std::future::poll_fn(|cx| {
            let _reads = reads.enter();
            wait.as_mut().poll(cx)
        })
        .await;
        cljrs_runtime::interp::eval::publish_result(result, None)
    }
    #[cfg(not(feature = "no-gc"))]
    await_value_inner(val).await
}

async fn await_value_inner(val: Value) -> EvalResult {
    match val {
        Value::Future(f) => {
            // Root the future across GC cycles: the alloc frame of the scope that
            // produced it may have dropped before this task reached a yield point.
            let anchor = Value::Future(f.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                let listener = f.get().ready.listen();
                if let Some(result) = f.get().settled_value() {
                    return result.map_err(cljrs_runtime::env::error::value_error_to_eval_error);
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                listener.await;
            }
        }
        Value::Promise(p) => {
            let anchor = Value::Promise(p.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                let listener = p.get().ready.listen();
                let value = p.get().value.lock().unwrap().clone();
                if let Some(value) = value {
                    cljrs_value::publish::read_slot(p.get() as *const _ as usize, 0);
                    return Ok(value);
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                listener.await;
            }
        }
        other => Ok(other),
    }
}

/// Evaluate a sequence of body forms, returning the value of the last.
async fn eval_body_async(forms: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in forms {
        result = Box::pin(eval_async_inner(form, env)).await?;
    }
    Ok(result)
}

/// `(try body... (catch Type e handler...)... (finally cleanup...))` with
/// yielding bodies. Mirrors the synchronous `eval_try` (`cljrs_runtime::interp`) exactly,
/// but evaluates the body, catch handlers, and finally block with `eval_async`
/// so an `await`/`<?` inside any of them cooperates with the executor instead of
/// falling back to the blocking sync path.
async fn eval_try_async(args: &[Form], env: &mut Env) -> EvalResult {
    let (body, catches, fin_body) = cljrs_runtime::interp::special::parse_try_args(args);

    let mut result = eval_body_async(body, env).await;

    // Handle catch: never intercept Recur (loop/fn trampoline signal).
    let err_opt = match std::mem::replace(&mut result, Ok(Value::Nil)) {
        Ok(v) => {
            result = Ok(v);
            None
        }
        Err(EvalError::Recur(recur_args)) => {
            result = Err(EvalError::Recur(recur_args));
            None
        }
        Err(EvalError::GasExhausted) => {
            result = Err(EvalError::GasExhausted);
            None
        }
        Err(other) => Some(other),
    };

    if let Some(err) = err_opt {
        let thrown_val = match err {
            EvalError::Thrown(v) => v,
            ref other => cljrs_runtime::interp::special::eval_error_to_value(other),
        };
        let mut handled = false;
        for c in &catches {
            if cljrs_runtime::interp::special::catch_type_matches(c.type_sym, &thrown_val) {
                env.push_frame();
                env.bind(std::sync::Arc::from(c.binding), thrown_val.clone());
                result = eval_body_async(c.body, env).await;
                env.pop_frame();
                handled = true;
                break;
            }
        }
        if !handled {
            // No matching catch — re-throw.
            result = Err(EvalError::Thrown(thrown_val));
        }
    }

    // Always run finally (its value is discarded).
    if !fin_body.is_empty() {
        let _ = eval_body_async(fin_body, env).await;
    }

    result
}

/// `(if test then else?)` with a yielding test and selected branch.
async fn eval_if_async(args: &[Form], env: &mut Env) -> EvalResult {
    let Some(test_form) = args.first() else {
        return Err(EvalError::Runtime("if requires a test".into()));
    };
    let test = Box::pin(eval_async_inner(test_form, env)).await?;
    let truthy = !matches!(test, Value::Nil | Value::Bool(false));
    if truthy {
        match args.get(1) {
            Some(then) => Box::pin(eval_async_inner(then, env)).await,
            None => Ok(Value::Nil),
        }
    } else {
        match args.get(2) {
            Some(els) => Box::pin(eval_async_inner(els, env)).await,
            None => Ok(Value::Nil),
        }
    }
}

/// `(let* [bindings] body…)` with yielding binding inits and body. Destructuring
/// patterns are bound via the shared [`bind_pattern`] helper.
async fn eval_let_async(args: &[Form], env: &mut Env) -> EvalResult {
    let bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => expand_pairs(v)
            .map_err(|_| EvalError::Runtime("let* binding vector must have even length".into()))?
            .into_owned(),
        _ => return Err(EvalError::Runtime("let* requires a binding vector".into())),
    };

    env.push_frame();
    for pair in bindings.chunks(2) {
        let val = match Box::pin(eval_async_inner(&pair[1], env)).await {
            Ok(v) => v,
            Err(e) => {
                env.pop_frame();
                return Err(e);
            }
        };
        if let Err(e) = bind_pattern(&pair[0], val, env) {
            env.pop_frame();
            return Err(e);
        }
    }
    let result = eval_body_async(&args[1..], env).await;
    env.pop_frame();
    result
}

/// `(loop* [bindings] body…)` / `(loop [bindings] body…)` — an async-aware
/// loop: binding inits are evaluated with [`eval_async`], the body runs with
/// [`eval_body_async`], and `recur` restarts the iteration.
async fn eval_loop_async(args: &[Form], env: &mut Env) -> EvalResult {
    let bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => expand_pairs(v)
            .map_err(|_| EvalError::Runtime("loop binding vector must have even length".into()))?
            .into_owned(),
        _ => return Err(EvalError::Runtime("loop requires a binding vector".into())),
    };

    let patterns: Vec<Form> = bindings.iter().step_by(2).cloned().collect();
    let init_forms: Vec<Form> = bindings.iter().skip(1).step_by(2).cloned().collect();

    // Evaluate initial binding values.
    let mut current_vals: Vec<Value> = Vec::with_capacity(patterns.len());
    for form in &init_forms {
        current_vals.push(Box::pin(eval_async_inner(form, env)).await?);
    }

    let body = &args[1..];
    loop {
        // Root current_vals so they survive GC — they're not yet bound in env.
        let _vals_root = cljrs_runtime::env::gc_roots::root_values(&current_vals);

        // GC safepoint on every iteration, as in the synchronous `eval_loop`,
        // so a tight recur loop that never awaits still lets the collector run.
        cljrs_runtime::env::gc_roots::gc_safepoint(env)?;

        // Scope this iteration's allocations in a fresh alloc frame so the
        // intermediates and the previous iteration's recur values become
        // collectable once the iteration ends (see `eval_loop`).
        let iter_frame = cljrs_gc::push_alloc_frame();

        env.push_frame();
        for (pat, val) in patterns.iter().zip(current_vals.iter()) {
            if let Err(e) = bind_pattern(pat, val.clone(), env) {
                env.pop_frame();
                return Err(e);
            }
        }

        let (result, suspended) = poll_tracking_suspension(eval_body_async(body, env)).await;
        env.pop_frame();

        match result {
            Err(EvalError::Recur(new_vals)) => {
                // Dropping the frame truncates ALLOC_ROOTS back to where this
                // iteration started.  That is only sound when nothing else ran
                // in between: if the body yielded, other tasks on this thread's
                // `LocalSet` may have pushed roots of their own above ours, so
                // keep every entry and let the enclosing frame release them.
                if suspended {
                    std::mem::forget(iter_frame);
                }
                if new_vals.len() != patterns.len() {
                    return Err(EvalError::Arity {
                        name: "recur".into(),
                        expected: patterns.len().to_string(),
                        got: new_vals.len(),
                    });
                }
                // new_vals are moved out before the frame drops and are
                // re-rooted at the top of the next iteration; no safepoint
                // runs in between.
                current_vals = new_vals;
            }
            // The loop's result (or error payload) keeps its alloc-frame roots
            // until the enclosing frame drops: the async caller may yield
            // before it roots the value itself.
            other => {
                std::mem::forget(iter_frame);
                return other;
            }
        }
    }
}

/// Drive `fut` to completion and report whether it ever returned `Pending`,
/// i.e. whether it yielded and let other tasks on the `LocalSet` run.
async fn poll_tracking_suspension<F: Future>(fut: F) -> (F::Output, bool) {
    let mut fut = std::pin::pin!(fut);
    let mut suspended = false;
    let out = std::future::poll_fn(|cx| {
        let poll = fut.as_mut().poll(cx);
        suspended |= poll.is_pending();
        poll
    })
    .await;
    (out, suspended)
}

/// `(recur args…)` — evaluate every argument with [`eval_async`] so an `await`
/// in a recur position yields, then raise the `EvalError::Recur` trampoline
/// signal that [`eval_loop_async`] (loop target) or [`run_async_fn`] (fn target)
/// catches. Mirrors the synchronous `eval_recur` (`cljrs_runtime::interp::special`)
/// apart from the evaluator used for the arguments.
async fn eval_recur_async(args: &[Form], env: &mut Env) -> EvalResult {
    let mut vals: Vec<Value> = Vec::with_capacity(args.len());
    for form in args {
        // Root the arguments already evaluated: each remaining `await` is a
        // yield point at which a GC cycle may run. The root records a raw
        // (ptr, len), so it is dropped before `push` can move the storage.
        let val = {
            let _vals_root = cljrs_runtime::env::gc_roots::root_values(&vals);
            Box::pin(eval_async_inner(form, env)).await?
        };
        vals.push(val);
    }
    Err(EvalError::Recur(vals))
}

/// `(def name "doc"? value?)` with a yielding value expression. Name, metadata
/// and interning are shared with the synchronous `def`.
async fn eval_def_async(args: &[Form], env: &mut Env) -> EvalResult {
    let target = cljrs_runtime::interp::special::parse_def(args, env)?;
    let val = match target.value_form {
        Some(form) => {
            // `^{...}` metadata was evaluated by `parse_def`; keep it alive
            // across the value's yield points.
            let _meta_root = target
                .meta
                .as_ref()
                .map(cljrs_runtime::env::gc_roots::root_value);
            eval_def_value_async(form, env).await?
        }
        None => Value::Nil,
    };
    cljrs_runtime::interp::special::intern_def(target, val, env)
}

/// Evaluate a `def` value expression. Under no-gc the sync `def` allocates it
/// in the StaticArena, because the var outlives every scratch region; the
/// allocation context is thread-local, so here it is installed per poll.
async fn eval_def_value_async(form: &Form, env: &mut Env) -> EvalResult {
    #[cfg(feature = "no-gc")]
    {
        let mut ctx: Option<cljrs_gc::alloc_ctx::StaticCtxGuard> = None;
        poll_scoped(
            Box::pin(eval_async_inner(form, env)),
            &mut ctx,
            |ctx| *ctx = Some(cljrs_gc::alloc_ctx::StaticCtxGuard::new()),
            |ctx| *ctx = None,
        )
        .await
    }
    #[cfg(not(feature = "no-gc"))]
    {
        Box::pin(eval_async_inner(form, env)).await
    }
}

/// `(defonce name value)`: the value expression is evaluated, yieldingly, only
/// when the var is not already bound.
async fn eval_defonce_async(args: &[Form], env: &mut Env) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Runtime("defonce requires a name".into()));
    }
    let target = cljrs_runtime::interp::special::parse_def(args, env)?;
    if let Some(var) = cljrs_runtime::interp::special::defonce_existing(&target.name, env) {
        return Ok(var);
    }
    Box::pin(eval_def_async(args, env)).await
}

/// `(and forms…)`, short-circuiting, with yielding operands.
async fn eval_and_async(args: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Bool(true);
    for form in args {
        result = Box::pin(eval_async_inner(form, env)).await?;
        if matches!(result, Value::Nil | Value::Bool(false)) {
            return Ok(result);
        }
    }
    Ok(result)
}

/// `(or forms…)`, short-circuiting, with yielding operands.
async fn eval_or_async(args: &[Form], env: &mut Env) -> EvalResult {
    let mut last = Value::Nil;
    for form in args {
        last = Box::pin(eval_async_inner(form, env)).await?;
        if !matches!(last, Value::Nil | Value::Bool(false)) {
            return Ok(last);
        }
    }
    Ok(last)
}

/// `(throw x)` with a yielding `x`.
async fn eval_throw_async(args: &[Form], env: &mut Env) -> EvalResult {
    let val = match args.first() {
        Some(f) => Box::pin(eval_async_inner(f, env)).await?,
        None => Value::Nil,
    };
    Err(cljrs_runtime::interp::special::throw_value(val))
}

/// `(set! target value)` with a yielding value and, for a `(.-field inst)`
/// target, a yielding `inst`. Evaluation order matches the sync `set!`.
async fn eval_set_bang_async(args: &[Form], env: &mut Env) -> EvalResult {
    use cljrs_runtime::interp::special as sp;
    let target = args
        .first()
        .ok_or_else(|| EvalError::Runtime("set! requires a target".into()))?;
    let val = match args.get(1) {
        Some(f) => Box::pin(eval_async_inner(f, env)).await?,
        None => Value::Nil,
    };
    match &target.kind {
        FormKind::Symbol(sym) => sp::set_bang_symbol(sym, val, env),
        _ => match sp::set_bang_field_target(target) {
            Some((field, inst_form)) => {
                let _val_root = cljrs_runtime::env::gc_roots::root_value(&val);
                let inst = Box::pin(eval_async_inner(inst_form, env)).await?;
                sp::set_type_instance_field(&inst, field, val.clone())
            }
            None => Err(sp::set_bang_target_error()),
        },
    }
}

/// `(letfn [fns…] body…)`: the fns are built by the shared sync helper (building
/// a closure evaluates nothing), and the body yields.
async fn eval_letfn_async(args: &[Form], env: &mut Env) -> EvalResult {
    cljrs_runtime::interp::special::push_letfn_frame(args, env)?;
    let result = eval_body_async(&args[1..], env).await;
    env.pop_frame();
    result
}

/// `(binding [var val …] body…)` with yielding inits and body.
///
/// Dynamic bindings live on a thread-local stack, and every task on the
/// `LocalSet` shares the thread. Leaving the frame pushed across a yield would
/// let other tasks see it, and would let them pop it (or have theirs popped).
/// So the frame is pushed at the start of each poll of the body and taken back
/// off at the end; between polls its values sit in a rooted slice, which picks
/// up any `set!` the body made.
async fn eval_binding_async(args: &[Form], env: &mut Env) -> EvalResult {
    use cljrs_runtime::env::dynamics;
    let pairs = match args.first().and_then(|f| f.as_vector()) {
        Some(v) => expand_pairs(v)
            .map_err(|_| EvalError::Runtime("binding vector must have even count".into()))?
            .into_owned(),
        None => return Err(EvalError::Runtime("binding requires a vector".into())),
    };

    let mut keys: Vec<dynamics::VarKey> = Vec::with_capacity(pairs.len() / 2);
    let mut vals: Vec<Value> = Vec::with_capacity(pairs.len() / 2);
    for pair in pairs.chunks(2) {
        let Some(sym_str) = pair[0].as_symbol() else {
            return Err(EvalError::Runtime("binding targets must be symbols".into()));
        };
        let parsed = cljrs_value::Symbol::parse(sym_str);
        let ns_part = env.resolve_ns_or_current(parsed.namespace.as_deref());
        let var_ptr = env
            .globals
            .lookup_var_in_ns(&ns_part, &parsed.name)
            .ok_or_else(|| EvalError::UnboundSymbol(sym_str.to_string()))?;
        let val = {
            let _vals_root = cljrs_runtime::env::gc_roots::root_values(&vals);
            Box::pin(eval_async_inner(&pair[1], env)).await?
        };
        keys.push(dynamics::var_key_of(&var_ptr));
        vals.push(val);
    }

    // `vals` is not resized from here on, so its storage is stable while rooted.
    let _vals_root = cljrs_runtime::env::gc_roots::root_values(&vals);
    let mut state = (keys, vals, None::<dynamics::BindingGuard>);
    poll_scoped(
        Box::pin(eval_body_async(&args[1..], env)),
        &mut state,
        |(keys, vals, guard)| {
            // Built in binding order, so a repeated var keeps its last value,
            // as in the sync `binding`.
            let frame = keys.iter().copied().zip(vals.iter().cloned()).collect();
            *guard = Some(dynamics::push_frame(frame));
        },
        |(keys, vals, guard)| {
            if let Some(guard) = guard.take() {
                let frame = dynamics::take_frame(guard);
                for (key, slot) in keys.iter().zip(vals.iter_mut()) {
                    if let Some(v) = frame.get(key) {
                        *slot = v.clone();
                    }
                }
            }
        },
    )
    .await
}

/// `(with-out-str body…)` with a yielding body. The capture buffer is
/// thread-local, so, as with `binding`, it is installed only while this task is
/// being polled and held here in between; output printed by other tasks while
/// this one is suspended is not captured.
async fn eval_with_out_str_async(body: &[Form], env: &mut Env) -> EvalResult {
    use cljrs_runtime::builtins::builtins::{pop_output_capture, resume_output_capture};
    let mut buf = String::new();
    let result = poll_scoped(
        Box::pin(eval_body_async(body, env)),
        &mut buf,
        |buf| resume_output_capture(std::mem::take(buf)),
        |buf| *buf = pop_output_capture().unwrap_or_default(),
    )
    .await;
    result?;
    Ok(Value::string(buf))
}

/// Drive `fut` to completion with thread-local state installed only for the
/// duration of each poll: `enter` runs before every poll and `exit` after it,
/// including when the poll unwinds. Tasks on the `LocalSet` share one thread,
/// so thread-local state a body relies on must not stay installed while it is
/// suspended and another task runs.
async fn poll_scoped<F, S>(
    mut fut: std::pin::Pin<Box<F>>,
    state: &mut S,
    enter: impl Fn(&mut S),
    exit: impl Fn(&mut S),
) -> F::Output
where
    F: Future + ?Sized,
{
    struct Exit<'a, S, X: Fn(&mut S)> {
        state: &'a mut S,
        exit: &'a X,
    }
    impl<S, X: Fn(&mut S)> Drop for Exit<'_, S, X> {
        fn drop(&mut self) {
            (self.exit)(self.state);
        }
    }

    std::future::poll_fn(|cx| {
        enter(state);
        let _exit = Exit {
            state: &mut *state,
            exit: &exit,
        };
        fut.as_mut().poll(cx)
    })
    .await
}

/// A function call whose arguments may contain `await`s. Arguments are
/// evaluated with [`eval_async`] (so awaits yield), then the callee is applied.
///
/// Calls whose head resolves to a form-intercepted native fn (`apply`, `swap!`,
/// …) are delegated wholesale to the synchronous evaluator, which performs the
/// special spreading/atom handling those builtins require. Such calls do not
/// yield on awaits inside their arguments in Phase B.
async fn eval_call_async(head: &Form, args: &[Form], whole: &Form, env: &mut Env) -> EvalResult {
    // Interop: `(.method target args…)` / `(.-field target)`. The head names a
    // method, not a value, so it must be routed before the callee is
    // evaluated — `eval` on it raises `UnboundSymbol`. Mirrors `eval_call`;
    // the target and arguments still take the yielding path, so an `await`
    // inside either cooperates.
    if let FormKind::Symbol(s) = &head.kind
        && cljrs_runtime::interp::apply::is_method_sugar(s)
    {
        let Some((target_form, arg_forms)) = args.split_first() else {
            return Err(EvalError::Runtime(format!("{s} requires a target object")));
        };
        let target = Box::pin(eval_async_inner(target_form, env)).await?;
        let _target_root = cljrs_runtime::env::gc_roots::root_value(&target);
        let mut argv: Vec<Value> = Vec::with_capacity(arg_forms.len());
        for a in arg_forms {
            let val = {
                let _args_root = cljrs_runtime::env::gc_roots::root_values(&argv);
                Box::pin(eval_async_inner(a, env)).await?
            };
            argv.push(val);
        }
        return cljrs_runtime::interp::apply::dispatch_method(&s[1..], &target, &argv);
    }

    let callee = eval(head, env)?;
    match &callee {
        Value::NativeFunction(nf)
            if cljrs_runtime::interp::apply::is_form_intercepted(&nf.get().name) =>
        {
            return eval(whole, env);
        }
        // A macro head should have been expanded already, but guard regardless.
        Value::Macro(_) => return eval(whole, env),
        _ => {}
    }

    let _callee_root = cljrs_runtime::env::gc_roots::root_value(&callee);
    let mut argv: Vec<Value> = Vec::with_capacity(args.len());
    for a in args {
        let val = {
            let _args_root = cljrs_runtime::env::gc_roots::root_values(&argv);
            Box::pin(eval_async_inner(a, env)).await?
        };
        argv.push(val);
    }
    cljrs_runtime::env::apply::apply_value(&callee, argv, env)
}

/// Flatten `recur` arguments for a variadic arity so the rest collection is
/// spread back into individual positional arguments (mirrors `call_cljrs_fn`).
fn flatten_recur_args(arity: &CljxFnArity, new_args: Vec<Value>) -> EvalResult<Vec<Value>> {
    if arity.rest_param.is_some() {
        let n = arity.params.len();
        if new_args.len() == n + 1 {
            let mut flat = new_args[..n].to_vec();
            match &new_args[n] {
                Value::Nil => {}
                rest_val => {
                    flat.extend(value_to_seq_vec(rest_val).map_err(value_error_to_eval_error)?)
                }
            }
            return Ok(flat);
        }
    }
    Ok(new_args)
}
