//! Hook trait for the optional async runtime (`cljrs-async`).
//!
//! Core crates never import Tokio. When `cljrs-async` is linked, it calls
//! `GlobalEnv::set_async_runtime` to install itself. The evaluator then
//! delegates `^:async` fn dispatch through this trait.

use cljrs_value::Value;

use crate::env::env::Env;

use crate::env::error::EvalResult;

/// Interface implemented by `cljrs-async` and registered with `GlobalEnv`.
///
/// All methods are called from the LocalSet thread, so `Value` / `Env` need
/// not be `Send`. The trait itself must be `Send + Sync` so the
/// `Arc<dyn AsyncRuntime>` inside `GlobalEnv` can be shared.
pub trait AsyncRuntime: Send + Sync {
    /// Spawn a call to an `^:async` function as a LocalSet task.
    ///
    /// `callee` is the `Value::Fn` being invoked, `args` are the already-
    /// evaluated arguments, `env` is the calling environment. Returns a
    /// `Value::Future` immediately; the body runs concurrently.
    fn spawn_async_call(&self, callee: Value, args: Vec<Value>, env: Env) -> Value;

    /// Spawn an ordinary synchronous call on a parking-capable fiber.
    /// Runtimes without fibers retain their async task implementation.
    fn spawn_fiber_call(&self, callee: Value, args: Vec<Value>, env: Env) -> EvalResult {
        Ok(self.spawn_async_call(callee, args, env))
    }

    /// `await` support: if `val` is one of this runtime's channels, take from it,
    /// parking the calling fiber (or blocking the OS thread outside a fiber),
    /// and keep it rooted for the whole wait. On wasm32, which cannot park, a
    /// ready value (or `nil` for a closed channel) is returned at once and a take
    /// that would block is an error. Returns `None` for any other value.
    fn take_if_channel(&self, val: &Value) -> Option<EvalResult> {
        let _ = val;
        None
    }

    /// Block the current OS thread until a value can be taken from the channel.
    ///
    /// Used by the IR interpreter's sync-context fallback for `ChanTake`.
    /// Returns `Value::Nil` on a closed channel.
    fn chan_take_blocking(&self, chan: Value) -> EvalResult;

    /// Block the current OS thread until the value is accepted by the channel.
    ///
    /// Used by the IR interpreter's sync-context fallback for `ChanPut`.
    fn chan_put_blocking(&self, chan: Value, val: Value) -> EvalResult<()>;
}
