//! Hook trait for the optional async runtime (`cljrs-async`).
//!
//! Core crates never import Tokio. When `cljrs-async` is linked, it calls
//! `GlobalEnv::set_async_runtime` to install itself. The evaluator then
//! delegates `^:async` fn dispatch through this trait.

use cljrs_value::Value;

use crate::env::Env;

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
}
