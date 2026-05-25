//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_env::async_hook::AsyncRuntime;
use cljrs_env::env::Env;
use cljrs_value::Value;

use crate::eval_async::{run_async_fn, spawn_future};

pub(crate) struct AsyncRuntimeImpl;

impl AsyncRuntimeImpl {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncRuntime for AsyncRuntimeImpl {
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, env: Env) -> Value {
        // `spawn_future` keeps the task on the current LocalSet thread, so the
        // `!Send` Clojure values (env, args, GcPtrs) never cross threads, and
        // delivers the body's result into the returned Future.
        spawn_future(async move { run_async_fn(callee, args, &env).await })
    }
}
