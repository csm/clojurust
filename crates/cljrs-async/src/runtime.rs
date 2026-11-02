//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_runtime::env::async_hook::AsyncRuntime;
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_value::{NativeObjectBox, Value};

use crate::channel::CljChannel;
#[cfg(target_arch = "wasm32")]
use crate::eval_async::run_async_fn;
#[cfg(not(target_arch = "wasm32"))]
use crate::task::spawn_future;
#[cfg(target_arch = "wasm32")]
use crate::task::spawn_future_with_roots;

pub(crate) struct AsyncRuntimeImpl;

impl AsyncRuntimeImpl {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncRuntime for AsyncRuntimeImpl {
    /// Native targets run an `^:async` call on its own fiber. The body uses the
    /// ordinary synchronous evaluator, so `await`, `deref`, and channel ops
    /// park the fiber. The call still returns a `Future` immediately.
    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, env: Env) -> Value {
        match call_fiber_with(callee, args, env, Vec::new(), FiberEntry::Body) {
            Ok(fiber) => spawn_future(fiber.into_task()),
            Err(error) => spawn_future(async move { Err(error) }),
        }
    }

    /// wasm32 has no stack switching, so `^:async` bodies keep the colored
    /// `eval_async` tree-walker there.
    #[cfg(target_arch = "wasm32")]
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, env: Env) -> Value {
        let mut roots = args.clone();
        roots.push(callee.clone());
        spawn_future_with_roots(roots, async move { run_async_fn(callee, args, &env).await })
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::result_large_err)]
    fn spawn_fiber_call(&self, callee: Value, args: Vec<Value>, env: Env) -> EvalResult {
        Ok(spawn_future(
            call_fiber(callee, args, env, Vec::new())?.into_task(),
        ))
    }

    fn take_if_channel(&self, val: &Value) -> Option<EvalResult> {
        let Value::NativeObject(obj) = val else {
            return None;
        };
        if obj.get().type_tag() != crate::channel::CHANNEL_TAG {
            return None;
        }
        Some(self.chan_take_blocking(val.clone()))
    }

    fn chan_take_blocking(&self, chan: Value) -> EvalResult {
        // Root the channel for the whole wait: once its producer closes it and
        // drops its own reference, this local may be the only one left.
        let _root = cljrs_runtime::env::gc_roots::root_value(&chan);
        let ch = downcast_channel(&chan)?;
        cljrs_value::parking::block_on(ch.take_result())
            .map_err(cljrs_runtime::env::error::value_error_to_eval_error)
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

/// Build a call fiber without an intermediate future cell or delivery fiber.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::result_large_err)]
pub(crate) fn call_fiber(
    callee: Value,
    args: Vec<Value>,
    env: Env,
    roots: Vec<Value>,
) -> Result<crate::fiber::Fiber, EvalError> {
    call_fiber_with(callee, args, env, roots, FiberEntry::Apply)
}

/// How a call fiber enters its callee.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy)]
enum FiberEntry {
    /// Ordinary application, including `^:async` dispatch of the callee.
    Apply,
    /// Run the callee's body directly. Used by `^:async` dispatch itself, so the
    /// spawned fiber does not dispatch (and spawn) again.
    Body,
}

#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::result_large_err)]
fn call_fiber_with(
    callee: Value,
    args: Vec<Value>,
    mut env: Env,
    mut roots: Vec<Value>,
    entry: FiberEntry,
) -> Result<crate::fiber::Fiber, EvalError> {
    roots.push(callee.clone());
    roots.extend(args.iter().cloned());
    roots.extend(
        env.frames
            .iter()
            .flat_map(|frame| frame.bindings.iter().map(|(_, value)| value.clone())),
    );
    conveyed_fiber(roots, move || match entry {
        FiberEntry::Apply => cljrs_runtime::env::apply::apply_value(&callee, args, &mut env),
        FiberEntry::Body => cljrs_runtime::env::apply::apply_fn_body(&callee, args, &mut env),
    })
}

/// Build a fiber that runs `body` with the caller's dynamic bindings (a
/// snapshot), active gas meters, and interrupt flags installed, as `future`
/// does. `roots` are the managed values `body` captures; the conveyed binding
/// values are added to them.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::result_large_err)]
pub(crate) fn conveyed_fiber(
    mut roots: Vec<Value>,
    body: impl FnOnce() -> EvalResult + 'static,
) -> Result<crate::fiber::Fiber, EvalError> {
    use cljrs_runtime::env::{dynamics, gas};
    #[cfg(feature = "no-gc")]
    if cljrs_gc::alloc_ctx::invocation_is_active() {
        return Err(EvalError::Runtime(
            "cannot spawn a fiber from an isolated allocation invocation".into(),
        ));
    }
    let bindings = dynamics::capture_current();
    let meters = gas::active_meters();
    let interrupts = gas::active_interrupts();
    roots.extend(bindings.iter().flat_map(|frame| frame.values().cloned()));
    crate::fiber::Fiber::with_roots(roots, move || {
        dynamics::install_frames(bindings);
        let _interrupts = gas::InterruptScope::enter(&interrupts);
        let _meters = gas::install_meters(&meters);
        body()
    })
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
/// At each cooperative yield the task calls [`cljrs_runtime::env::gc_roots::async_gc_collect`].
/// Because `LocalSet` is single-threaded, no other tasks run while that function
/// executes, making it safe to scan thread-local root stacks for all suspended tasks.
///
/// Must be called from within a Tokio `LocalSet` context (e.g., from `init`).
/// On `wasm32` this is a no-op — see below.
pub(crate) fn spawn_gc_service() {
    // On native, `spawn_local` panics unless it is called inside a Tokio
    // runtime *and* a `LocalSet`.  Several callers invoke `init()` with no
    // runtime at all — notably the AOT compiler, which registers
    // `clojure.core.async` only so `require`/`go`/`await` resolve during
    // macro-expansion, and unit tests that call `init()` before
    // `block_on_local`.  Probe for a runtime first and skip the GC service when
    // there is none: a missing service outside a LocalSet is expected and
    // harmless — safepoints inside `await_value` still fire whenever code runs
    // in a LocalSet.  Probing (rather than provoke-and-`catch_unwind`) avoids
    // emitting a scary—but-caught—panic message to stderr during compilation.
    // The `catch_unwind` remains as a guard for the pathological case of a
    // runtime present without a LocalSet, which none of our call paths hit.
    #[cfg(not(target_arch = "wasm32"))]
    if tokio::runtime::Handle::try_current().is_ok() {
        let _ = std::panic::catch_unwind(|| {
            tokio::task::spawn_local(async {
                loop {
                    tokio::task::yield_now().await;
                    cljrs_runtime::env::gc_roots::async_gc_collect();
                }
            });
        });
    }

    // On wasm32, tokio's yield_now() only cooperates with the LocalSet scheduler
    // — it does NOT yield back to the browser event loop.  A `loop { yield_now();
    // gc(); }` task therefore generates an endless chain of microtasks that
    // starves rendering and bogs down the browser.  Skip the service entirely;
    // GC safepoints in `await_value` fire at every real async suspension point,
    // which is sufficient for correctness.
    #[cfg(target_arch = "wasm32")]
    let _ = ();
}
