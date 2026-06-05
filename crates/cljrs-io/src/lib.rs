//! Asynchronous file I/O for clojurust — `clojure.rust.io.async`.
//!
//! Tokio-backed file reads and writes whose results are delivered over
//! `clojure.core.async` channels. The crate offers two complementary shapes:
//!
//! - **Streaming reads** return a *raw channel* that yields a sequence of values
//!   and closes at EOF (`chunk-chan`, `byte-chan`, `char-chan`, `line-chan`).
//!   The channel's buffer bounds read-ahead, giving natural backpressure for
//!   large files.
//! - **Discrete ops** return a *promise channel* (capacity 1) that delivers a
//!   single result and closes (`slurp`, `slurp-bytes`, `read-bytes`, `spit`).
//!
//! Both shapes are built on `cljrs-async` [`CljChannel`]s, so the whole
//! `clojure.core.async` API operates on them uniformly. See the crate README
//! for the design rationale behind the raw-vs-promise split.
//!
//! # Usage
//!
//! ```rust,ignore
//! let globals = cljrs_stdlib::standard_env();
//! cljrs_async::init(&globals); // channels + executor (required)
//! cljrs_io::init(&globals);    // async file I/O
//! ```
//!
//! Like `cljrs-async`, the spawned producer tasks require a Tokio
//! `current_thread` + `LocalSet` executor running on the calling thread.
//!
//! [`CljChannel`]: cljrs_async::channel::CljChannel

use std::sync::Arc;

pub mod charset;
pub mod fs;

use cljrs_async::load_source;
use cljrs_env::env::GlobalEnv;

/// Clojure-level helpers (`error?`, `ok?`) loaded on top of the native
/// primitives at `init` time.
const IO_ASYNC_SOURCE: &str = include_str!("clojure_rust_io_async.cljrs");

/// The namespace this crate populates.
pub const NS: &str = "clojure.rust.io.async";

/// Register the async I/O native functions and load the
/// `clojure.rust.io.async` namespace.
///
/// Requires the async runtime (`cljrs_async::init`) for channel consumption and
/// a running `LocalSet` for the producer tasks. Idempotent: the namespace is
/// built only once.
pub fn init(globals: &Arc<GlobalEnv>) {
    if globals.is_loaded(NS) {
        return;
    }

    globals.get_or_create_ns(NS);
    globals.refer_all(NS, "clojure.core");
    fs::register(globals, NS);
    load_source(globals, NS, IO_ASYNC_SOURCE);
    globals.mark_loaded(NS);
}
