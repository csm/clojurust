//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_env::async_hook::AsyncRuntime;
use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_value::{NativeObjectBox, Value};

use crate::channel::CljChannel;
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

    fn chan_take_blocking(&self, chan: Value) -> EvalResult {
        let ch = downcast_channel(&chan)?;
        Ok(ch.take_blocking())
    }

    fn chan_put_blocking(&self, chan: Value, val: Value) -> EvalResult<()> {
        let ch = downcast_channel(&chan)?;
        if ch.put_blocking(val) {
            Ok(())
        } else {
            Err(EvalError::Runtime("chan-put: channel is closed".into()))
        }
    }
}

#[allow(clippy::result_large_err)]
fn downcast_channel(val: &Value) -> EvalResult<&CljChannel> {
    match val {
        Value::NativeObject(ptr) => {
            let obj: &NativeObjectBox = ptr.get();
            obj.downcast_ref::<CljChannel>().ok_or_else(|| {
                EvalError::Runtime(format!("expected Channel, got {}", obj.type_tag()))
            })
        }
        other => Err(EvalError::Runtime(format!(
            "expected Channel, got {}",
            other.type_name(),
        ))),
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
/// On `wasm32` this is a no-op — see below.
pub(crate) fn spawn_gc_service() {
    // On native, `spawn_local` panics when called outside a `LocalSet` context
    // (e.g. in unit tests that call `init()` before `block_on_local`).
    // `catch_unwind` silences that panic; the service simply won't run, which is
    // fine — safepoints inside `await_value` still fire whenever code runs in a
    // LocalSet.
    #[cfg(not(target_arch = "wasm32"))]
    let _ = std::panic::catch_unwind(|| {
        tokio::task::spawn_local(async {
            loop {
                tokio::task::yield_now().await;
                cljrs_env::gc_roots::async_gc_collect();
            }
        });
    });

    // On wasm32, tokio's yield_now() only cooperates with the LocalSet scheduler
    // — it does NOT yield back to the browser event loop.  A `loop { yield_now();
    // gc(); }` task therefore generates an endless chain of microtasks that
    // starves rendering and bogs down the browser.  Skip the service entirely;
    // GC safepoints in `await_value` fire at every real async suspension point,
    // which is sufficient for correctness.
    #[cfg(target_arch = "wasm32")]
    let _ = ();
}
