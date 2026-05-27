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

/// Spawn a long-lived background task on the current `LocalSet` that services
/// GC requests between poll cycles.
///
/// At each cooperative yield the task calls [`cljrs_env::gc_roots::async_gc_collect`].
/// Because `LocalSet` is single-threaded, no other tasks run while that function
/// executes, making it safe to scan thread-local root stacks for all suspended tasks.
///
/// Must be called from within a Tokio `LocalSet` context (e.g., from `init`).
pub(crate) fn spawn_gc_service() {
    // `spawn_local` panics when called outside a `LocalSet` context (e.g., in
    // unit tests that call `init()` before entering `block_on_local`).  In that
    // case the GC service simply won't run, which is fine: the service is a
    // best-effort background collector; the safepoints inside `await_value`
    // still fire whenever the code is actually running inside a LocalSet.
    let _ = std::panic::catch_unwind(|| {
        tokio::task::spawn_local(async {
            loop {
                tokio::task::yield_now().await;
                cljrs_env::gc_roots::async_gc_collect();
            }
        });
    });
}
