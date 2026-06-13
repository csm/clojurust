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
pub mod channel;
pub mod eval_async;
pub mod isolate;
pub mod isolate_channel;
mod runtime;
pub mod worker_pool;
use runtime::AsyncRuntimeImpl;

// Re-exported so sibling native crates (e.g. `cljrs-io`) can spawn work onto the
// shared `LocalSet` executor and await Clojure futures/promises without reaching
// into private modules.
pub use eval_async::{await_value, spawn_future};

/// Clojure-level `clojure.core.async` definitions (the `alt` macro), evaluated
/// on top of the native primitives at `init` time.
const CORE_ASYNC_SOURCE: &str = include_str!("core_async.cljrs");

/// Error-value helpers (`error?`, `ok?`, `throw-err`) shared by the
/// channel-based APIs. Loaded as `clojure.rust.error` before `clojure.core.async`
/// so the `<?` family can refer to `throw-err`.
const ERROR_SOURCE: &str = include_str!("clojure_rust_error.cljrs");

/// Register the async runtime with the interpreter and load the
/// `clojure.core.async` namespace.
///
/// Must be called from within a Tokio `LocalSet` context for spawned tasks to
/// run. Idempotent: the namespace is built only once.
pub fn init(globals: &Arc<cljrs_env::env::GlobalEnv>) {
    globals.set_async_runtime(Arc::new(AsyncRuntimeImpl::new()));
    runtime::spawn_gc_service();

    // clojure.rust.error first — the <? family refers its `throw-err`.
    if !globals.is_loaded("clojure.rust.error") {
        globals.get_or_create_ns("clojure.rust.error");
        globals.refer_all("clojure.rust.error", "clojure.core");
        load_source(globals, "clojure.rust.error", ERROR_SOURCE);
        globals.mark_loaded("clojure.rust.error");
    }

    let ns = "clojure.core.async";
    if globals.is_loaded(ns) {
        return;
    }

    // Build the namespace: refer clojure.core so the macro source can use core
    // fns/macros, register the native primitives, then evaluate the source.
    globals.get_or_create_ns(ns);
    globals.refer_all(ns, "clojure.core");
    builtins::register(globals, ns);
    load_source(globals, ns, CORE_ASYNC_SOURCE);
    globals.mark_loaded(ns);
}

/// Evaluate a Clojure source string form-by-form into an already-created
/// namespace. Parse/eval failures are reported but do not abort `init`.
/// Exported for use by crates that register their own namespaces.
pub fn load_source(globals: &Arc<cljrs_env::env::GlobalEnv>, ns: &str, source: &str) {
    let mut env = cljrs_env::env::Env::new(globals.clone(), ns);
    let mut parser = cljrs_reader::Parser::new(source.to_string(), format!("<{ns}>"));
    match parser.parse_all() {
        Ok(forms) => {
            for form in forms {
                let _alloc_frame = cljrs_gc::push_alloc_frame();
                if let Err(e) = cljrs_interp::eval::eval(&form, &mut env) {
                    eprintln!("[{ns} warning] {e:?}");
                }
            }
        }
        Err(e) => eprintln!("[{ns} parse error] {e:?}"),
    }
}
