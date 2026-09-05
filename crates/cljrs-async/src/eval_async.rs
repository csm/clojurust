// `EvalResult`'s error is `cljrs_runtime`'s `EvalError`, returned by value from
// every evaluator entry point; the synchronous side allows this lint crate-wide
// (`cljrs-runtime/src/lib.rs`) and `runtime.rs` allows it per function.  Clippy
// only began flagging `async fn`s for it in 1.98 — the code below is unchanged
// from when it passed under 1.94.
#![allow(clippy::result_large_err)]

//! Asynchronous tree-walking evaluation for `^:async` function bodies.
//!
//! [`eval_async`] mirrors the synchronous [`cljrs_runtime::interp::eval::eval`] for the
//! handful of forms where an `await` can legitimately appear — `await` itself,
//! `do`, `if`, `let`/`let*`, and function-call arguments — and delegates every
//! other form to the synchronous evaluator. When it reaches an `(await x)` it
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
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::interp::apply::{bind_fn_params, select_arity};
use cljrs_runtime::interp::destructure::{bind_pattern, value_to_seq_vec};
use cljrs_runtime::interp::eval::{eval, is_special_form};
use cljrs_runtime::interp::macros::macroexpand;
use cljrs_value::value::SetValue;
use cljrs_value::{
    CljxFnArity, CljxFuture, FutureState, MapValue, PersistentHashSet, PersistentList,
    PersistentVector, Value,
};

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
    // GC builds: heap-promotion fallback — the task's captured environment is
    // opaque to the publish-barrier scan, and the task may run after any
    // bump-region scope active right now has closed.  Poison the active
    // regions so they are retired (kept alive) instead of reset; a no-op (one
    // thread-local read) when no region is open, which is the common case.
    cljrs_gc::region::poison_active_regions();
    let future = GcPtr::new(CljxFuture::new());
    let task_future = future.clone();
    let gas_meters = cljrs_runtime::env::gas::active_meters();
    tokio::task::spawn_local(async move {
        // Root the result future across GC cycles: the spawning scope's alloc
        // frame may have dropped before the task gets to run.
        let anchor = Value::Future(task_future.clone());
        let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
        let mut task = Box::pin(task);
        let result = std::future::poll_fn(|cx| {
            // LocalSet tasks share an OS thread, so TLS state must be scoped
            // to one poll and removed before another task can run.
            let _gas_guards = cljrs_runtime::env::gas::install_meters(&gas_meters);
            task.as_mut().poll(cx)
        })
        .await;
        settle_future(&task_future, result);
    });
    Value::Future(future)
}

/// Write a completed result into a future and wake blocking `deref` waiters.
pub(crate) fn settle_future(future: &GcPtr<CljxFuture>, result: EvalResult) {
    let mut state = future.get().state.lock().unwrap();
    // Cancellation is sticky. `future-cancel` does not interrupt the task, so a
    // task cancelled mid-body still runs to completion and lands here; letting
    // it write its result would flip `future-cancelled?` back to false and hand
    // `await` a value it had already promised to raise on. Cancel wins: the
    // side effects happened, the result is discarded.
    if matches!(&*state, FutureState::Cancelled) {
        return;
    }
    *state = match result {
        Ok(v) => FutureState::Done(v),
        // Preserve the thrown value (and any non-Thrown error as a fresh
        // Value::Error) so `await` can re-throw it with ex-data/ex-cause intact.
        Err(EvalError::GasExhausted) => FutureState::GasExhausted,
        Err(e) => FutureState::Failed(e.to_error_value()),
    };
    drop(state);
    future.get().cond.notify_all();
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
                current_args = flatten_recur_args(&arity, new_args);
            }
            Err(e) => return Err(e),
        }
    }
}

/// Asynchronously evaluate a single form.
pub async fn eval_async(form: &Form, env: &mut Env) -> EvalResult {
    // Collection literals may contain `await` sub-expressions (e.g. the
    // return value of a ^:async fn is `[(await x) (await y)]`). Evaluate
    // each element with eval_async so awaits yield cooperatively instead of
    // blocking the LocalSet thread via the sync condvar path.
    match &form.kind {
        FormKind::Vector(elems) => {
            let elems = expand_reader_conds_cow(elems).into_owned();
            let mut vals: Vec<Value> = Vec::with_capacity(elems.len());
            for f in &elems {
                vals.push(Box::pin(eval_async(f, env)).await?);
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
                pairs.push(Box::pin(eval_async(f, env)).await?);
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
                vals.push(Box::pin(eval_async(f, env)).await?);
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
            // Other special forms (binding/…) don't yield yet: run them
            // synchronously. A `recur` that targets the enclosing async fn
            // surfaces as `EvalError::Recur` and is caught by `run_async_fn`.
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
    let val = Box::pin(eval_async(arg, env)).await?;
    await_value(val).await
}

/// Cooperatively await a Clojure value. Futures and promises yield to the
/// executor until resolved; any other value is returned as-is.
pub async fn await_value(val: Value) -> EvalResult {
    match val {
        Value::Future(f) => {
            // Root the future across GC cycles: the alloc frame of the scope that
            // produced it may have dropped before this task reached a yield point.
            let anchor = Value::Future(f.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                {
                    let guard = f.get().state.lock().unwrap();
                    match &*guard {
                        FutureState::Done(v) => {
                            f.get().mark_observed();
                            return Ok(v.clone());
                        }
                        FutureState::Failed(v) => {
                            f.get().mark_observed();
                            return Err(EvalError::Thrown(v.clone()));
                        }
                        FutureState::GasExhausted => {
                            f.get().mark_observed();
                            return Err(EvalError::GasExhausted);
                        }
                        FutureState::Cancelled => {
                            return Err(EvalError::Thrown(CljxFuture::cancelled_error()));
                        }
                        FutureState::Running => {}
                    }
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                tokio::task::yield_now().await;
            }
        }
        Value::Promise(p) => {
            let anchor = Value::Promise(p.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            loop {
                {
                    if let Some(v) = p.get().value.lock().unwrap().as_ref() {
                        return Ok(v.clone());
                    }
                }
                cljrs_runtime::env::gc_roots::async_gc_collect();
                tokio::task::yield_now().await;
            }
        }
        other => Ok(other),
    }
}

/// Evaluate a sequence of body forms, returning the value of the last.
async fn eval_body_async(forms: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in forms {
        result = Box::pin(eval_async(form, env)).await?;
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
    let test = Box::pin(eval_async(test_form, env)).await?;
    let truthy = !matches!(test, Value::Nil | Value::Bool(false));
    if truthy {
        match args.get(1) {
            Some(then) => Box::pin(eval_async(then, env)).await,
            None => Ok(Value::Nil),
        }
    } else {
        match args.get(2) {
            Some(els) => Box::pin(eval_async(els, env)).await,
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
        let val = match Box::pin(eval_async(&pair[1], env)).await {
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
        current_vals.push(Box::pin(eval_async(form, env)).await?);
    }

    let body = &args[1..];
    loop {
        env.push_frame();
        for (pat, val) in patterns.iter().zip(current_vals.iter()) {
            if let Err(e) = bind_pattern(pat, val.clone(), env) {
                env.pop_frame();
                return Err(e);
            }
        }

        let result = eval_body_async(body, env).await;
        env.pop_frame();

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_vals)) => {
                if new_vals.len() != patterns.len() {
                    return Err(EvalError::Arity {
                        name: "recur".into(),
                        expected: patterns.len().to_string(),
                        got: new_vals.len(),
                    });
                }
                current_vals = new_vals;
            }
            Err(e) => return Err(e),
        }
    }
}

/// A function call whose arguments may contain `await`s. Arguments are
/// evaluated with [`eval_async`] (so awaits yield), then the callee is applied.
///
/// Calls whose head resolves to a form-intercepted native fn (`apply`, `swap!`,
/// …) are delegated wholesale to the synchronous evaluator, which performs the
/// special spreading/atom handling those builtins require. Such calls do not
/// yield on awaits inside their arguments in Phase B.
async fn eval_call_async(head: &Form, args: &[Form], whole: &Form, env: &mut Env) -> EvalResult {
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
        let _args_root = cljrs_runtime::env::gc_roots::root_values(&argv);
        argv.push(Box::pin(eval_async(a, env)).await?);
    }
    cljrs_runtime::env::apply::apply_value(&callee, argv, env)
}

/// Flatten `recur` arguments for a variadic arity so the rest collection is
/// spread back into individual positional arguments (mirrors `call_cljrs_fn`).
fn flatten_recur_args(arity: &CljxFnArity, new_args: Vec<Value>) -> Vec<Value> {
    if arity.rest_param.is_some() {
        let n = arity.params.len();
        if new_args.len() == n + 1 {
            let mut flat = new_args[..n].to_vec();
            match &new_args[n] {
                Value::Nil => {}
                rest_val => flat.extend(value_to_seq_vec(rest_val)),
            }
            return flat;
        }
    }
    new_args
}
