//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_env::async_hook::AsyncRuntime;
use cljrs_env::env::Env;
use cljrs_env::error::EvalError;
use cljrs_gc::GcPtr;
use cljrs_value::{CljxFuture, FutureState, Value};

use crate::eval_async::run_async_fn;

pub(crate) struct AsyncRuntimeImpl;

impl AsyncRuntimeImpl {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncRuntime for AsyncRuntimeImpl {
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, env: Env) -> Value {
        // The result is delivered into a fresh condvar-backed `CljxFuture`,
        // shared between this handle and the spawned task.
        let future = GcPtr::new(CljxFuture::new());
        let task_future = future.clone();

        // `spawn_local` keeps the task on the current LocalSet thread, so the
        // `!Send` Clojure values (env, args, GcPtrs) never cross threads.
        tokio::task::spawn_local(async move {
            let result = run_async_fn(callee, args, &env).await;
            deliver(&task_future, result);
        });

        Value::Future(future)
    }
}

/// Write a completed `^:async` call's result into its future and wake any
/// blocking `deref`/`await` waiters.
fn deliver(future: &GcPtr<CljxFuture>, result: Result<Value, EvalError>) {
    let mut state = future.get().state.lock().unwrap();
    *state = match result {
        Ok(v) => FutureState::Done(v),
        Err(EvalError::Runtime(msg)) => FutureState::Failed(msg),
        Err(e) => FutureState::Failed(format!("{e}")),
    };
    drop(state);
    future.get().cond.notify_all();
}
