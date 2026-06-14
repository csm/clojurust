//! Hook trait for the optional async runtime (`cljrs-async`).
//!
//! Core crates never import Tokio. When `cljrs-async` is linked, it calls
//! `GlobalEnv::set_async_runtime` to install itself. The evaluator then
//! delegates `^:async` fn dispatch through this trait.

use cljrs_value::Value;

use crate::env::Env;

use crate::error::EvalResult;

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

// ── Async JIT compile hook ──────────────────────────────────────────────────
//
// `cljrs-async` drives `^:async` dispatch but cannot compile (it sits below
// `cljrs-jit`).  `cljrs-jit::init` installs this hook; the async dispatcher
// invokes it (once per arity) to lower + compile + register a native poll
// function for the called `^:async` arity.  A no-op when the JIT is absent, so
// dispatch keeps tree-walking via `eval_async`.

/// Signature of the async-JIT compile hook: `(callee_fn, nargs, env)`.
pub type AsyncCompileHook = fn(&Value, usize, &mut Env);

static ASYNC_COMPILE_HOOK: std::sync::OnceLock<AsyncCompileHook> = std::sync::OnceLock::new();

/// Install the async-JIT compile hook (called once by `cljrs-jit::init`).
pub fn set_async_compile_hook(hook: AsyncCompileHook) {
    let _ = ASYNC_COMPILE_HOOK.set(hook);
}

/// The installed async-JIT compile hook, if any.
pub fn async_compile_hook() -> Option<AsyncCompileHook> {
    ASYNC_COMPILE_HOOK.get().copied()
}
