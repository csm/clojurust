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
        // Spin-poll: yield the CPU between attempts.
        loop {
            if let Some(val) = ch.try_take() {
                return Ok(val);
            }
            std::thread::yield_now();
        }
    }

    fn chan_put_blocking(&self, chan: Value, val: Value) -> EvalResult<()> {
        let ch = downcast_channel(&chan)?;
        if ch.capacity() == 0 {
            // Rendezvous: offer and spin until a taker picks it up.
            use crate::channel::RvOffer;
            loop {
                match ch.rv_offer(&val) {
                    RvOffer::Offered(token) => {
                        // Wait until the token is consumed.
                        loop {
                            use crate::channel::RvStatus;
                            match ch.rv_status(token) {
                                RvStatus::Taken => return Ok(()),
                                RvStatus::ClosedUntaken => {
                                    return Err(EvalError::Runtime(
                                        "chan-put: channel closed before value was taken".into(),
                                    ));
                                }
                                RvStatus::Waiting => std::thread::yield_now(),
                            }
                        }
                    }
                    RvOffer::Full => std::thread::yield_now(),
                    RvOffer::Closed => {
                        return Err(EvalError::Runtime("chan-put: channel is closed".into()));
                    }
                }
            }
        } else {
            // Buffered: spin until there is room.
            loop {
                match ch.try_put_buffered(&val) {
                    Some(true) => return Ok(()),
                    Some(false) => {
                        return Err(EvalError::Runtime("chan-put: channel is closed".into()));
                    }
                    None => std::thread::yield_now(),
                }
            }
        }
    }
}

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
