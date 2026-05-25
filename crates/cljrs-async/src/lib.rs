#![allow(clippy::type_complexity)]
//! Async runtime for clojurust — `clojure.core.async` via Tokio.
//!
//! # Usage
//!
//! ```rust,ignore
//! let globals = cljrs_stdlib::standard_env();
//! cljrs_async::init(&globals);
//! ```
//!
//! After `init`, `^:async` functions, `await`, and the `clojure.core.async`
//! primitives implemented so far (`timeout`, `alts`, `alt`) are available.
//! The Tokio `current_thread` + `LocalSet` executor must be running on the
//! calling thread (the CLI sets this up automatically when built with the
//! `async` feature).

use std::sync::Arc;

mod builtins;
pub mod eval_async;
mod runtime;
use runtime::AsyncRuntimeImpl;

/// Clojure-level `clojure.core.async` definitions (the `alt` macro), evaluated
/// on top of the native primitives at `init` time.
const CORE_ASYNC_SOURCE: &str = include_str!("core_async.cljrs");

/// Register the async runtime with the interpreter and load the
/// `clojure.core.async` namespace.
///
/// Must be called from within a Tokio `LocalSet` context for spawned tasks to
/// run. Idempotent: the namespace is built only once.
pub fn init(globals: &Arc<cljrs_env::env::GlobalEnv>) {
    globals.set_async_runtime(Arc::new(AsyncRuntimeImpl::new()));

    let ns = "clojure.core.async";
    if globals.is_loaded(ns) {
        return;
    }

    // Build the namespace: refer clojure.core so the macro source can use core
    // fns/macros, register the native primitives, then evaluate the source.
    globals.get_or_create_ns(ns);
    globals.refer_all(ns, "clojure.core");
    builtins::register(globals, ns);

    let mut env = cljrs_env::env::Env::new(globals.clone(), ns);
    let mut parser =
        cljrs_reader::Parser::new(CORE_ASYNC_SOURCE.to_string(), "<clojure.core.async>".into());
    match parser.parse_all() {
        Ok(forms) => {
            for form in forms {
                let _alloc_frame = cljrs_gc::push_alloc_frame();
                if let Err(e) = cljrs_interp::eval::eval(&form, &mut env) {
                    eprintln!("[clojure.core.async warning] {e:?}");
                }
            }
        }
        Err(e) => eprintln!("[clojure.core.async parse error] {e:?}"),
    }
    globals.mark_loaded(ns);
}
