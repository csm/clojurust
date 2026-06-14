//! `AsyncRuntimeImpl` — the Tokio-backed implementation of `AsyncRuntime`.

use cljrs_env::async_hook::AsyncRuntime;
use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_interp::apply::select_arity;
use cljrs_value::{NativeObjectBox, Value};

use crate::channel::CljChannel;
use crate::eval_async::{run_async_fn, spawn_future};
use crate::state_machine::{
    lookup_poll_fn, lookup_poll_fn_named, mark_compile_attempted, spawn_state_machine,
};

/// Whether async-JIT activation is enabled (default on; disable with
/// `CLJRS_NO_ASYNC_JIT`).
fn async_jit_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("CLJRS_NO_ASYNC_JIT").is_err())
}

pub(crate) struct AsyncRuntimeImpl;

impl AsyncRuntimeImpl {
    pub fn new() -> Self {
        Self
    }
}

impl AsyncRuntime for AsyncRuntimeImpl {
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, mut env: Env) -> Value {
        // Native fast path: if this arity has a compiled poll function (or one
        // can be compiled now via the JIT hook), run the native state machine;
        // otherwise fall back to the tree-walker.  The compiled poll function
        // runs detached on the executor, so it carries the eval context
        // (globals + defining ns) it needs to resolve globals / call other fns.
        if let Value::Fn(f) = &callee {
            // Scope the GcPtr borrow so `callee`/`env` can move into the
            // fallback below; only Copy/cloned data escapes.
            let info = {
                let fr = f.get();
                select_arity(fr, args.len())
                    .ok()
                    .map(|a| (a.ir_arity_id, fr.defining_ns.clone(), fr.name.clone()))
            };
            if let Some((id, ns, name)) = info {
                let ctx = (env.globals.clone(), ns.clone());
                // JIT registry (keyed by runtime ir_arity_id).
                if let Some((poll_fn, n_slots)) = lookup_poll_fn(id) {
                    return spawn_state_machine(poll_fn, n_slots, args, Some(ctx));
                }
                // AOT registry (keyed by ns/name/arity, registered by the harness).
                if let Some(nm) = name.as_deref()
                    && let Some((poll_fn, n_slots)) = lookup_poll_fn_named(&ns, nm, args.len())
                {
                    return spawn_state_machine(poll_fn, n_slots, args, Some(ctx));
                }
                // One-shot compile attempt (when the JIT installed a hook).
                if async_jit_enabled()
                    && mark_compile_attempted(id)
                    && let Some(hook) = cljrs_env::async_hook::async_compile_hook()
                {
                    hook(&callee, args.len(), &mut env);
                    if let Some((poll_fn, n_slots)) = lookup_poll_fn(id) {
                        return spawn_state_machine(poll_fn, n_slots, args, Some(ctx));
                    }
                }
            }
        }
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
                    cljrs_env::gc_roots::async_gc_collect();
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
