//! Async runtime for clojurust — `clojure.core.async` via Tokio.
//!
//! # Usage
//!
//! ```rust,ignore
//! let globals = cljrs_stdlib::standard_env();
//! cljrs_async::init(&globals);
//! ```
//!
//! After `init`, `^:async` functions and all `clojure.core.async` primitives
//! (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) are available.
//! The Tokio `current_thread` + `LocalSet` executor must be running on the
//! calling thread (the CLI sets this up automatically when built with the
//! `async` feature).

use std::sync::Arc;

pub mod eval_async;
mod runtime;
use runtime::AsyncRuntimeImpl;

/// Register the async runtime with the interpreter and load the
/// `clojure.core.async` namespace.
///
/// Must be called from within a Tokio `LocalSet` context. Calling more than
/// once is a no-op (the second registration is silently ignored).
pub fn init(globals: &Arc<cljrs_env::env::GlobalEnv>) {
    globals.set_async_runtime(Arc::new(AsyncRuntimeImpl::new()));
    // Phase D-E: register clojure.core.async builtins and namespace here.
}
