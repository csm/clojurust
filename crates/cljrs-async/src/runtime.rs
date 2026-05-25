//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_env::async_hook::AsyncRuntime;
use cljrs_env::env::Env;
use cljrs_value::Value;

pub(crate) struct AsyncRuntimeImpl;

impl AsyncRuntimeImpl {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncRuntime for AsyncRuntimeImpl {
    fn spawn_async_call(&self, _callee: Value, _args: Vec<Value>, _env: Env) -> Value {
        // Phase B: spawn the ^:async fn body as a spawn_local task, return Value::Future.
        todo!("cljrs-async: ^:async dispatch not yet implemented (Phase B)")
    }
}
