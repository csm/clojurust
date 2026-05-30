//! Networking for clojurust — TCP over core.async channels.
//!
//! # Usage
//!
//! ```rust,ignore
//! let globals = cljrs_stdlib::standard_env();
//! cljrs_async::init(&globals); // channels + executor (required)
//! cljrs_net::init(&globals);   // networking
//! ```
//!
//! Like `cljrs-async` and `cljrs-io`, requires a Tokio `current_thread` +
//! `LocalSet` executor running on the calling thread. The CLI links this crate
//! by default under the `net` feature (on by default, same as `async`).

use std::sync::Arc;

use cljrs_async::load_source;

pub mod tcp;

/// Clojure source for `clojure.rust.net.tcp`.
const NET_TCP_SOURCE: &str = include_str!("clojure_rust_net_tcp.cljrs");

/// Clojure source for the umbrella `clojure.rust.net` namespace.
const NET_SOURCE: &str = include_str!("clojure_rust_net.cljrs");

pub const NS_TCP: &str = "clojure.rust.net.tcp";
pub const NS: &str = "clojure.rust.net";

/// Register the networking namespaces.
///
/// Calls `cljrs_async::init` internally (idempotent) so callers only need to
/// call this one function. Requires a running `LocalSet` executor. Idempotent.
pub fn init(globals: &Arc<cljrs_env::env::GlobalEnv>) {
    cljrs_async::init(globals);

    if !globals.is_loaded(NS_TCP) {
        globals.get_or_create_ns(NS_TCP);
        globals.refer_all(NS_TCP, "clojure.core");
        tcp::register(globals, NS_TCP);
        load_source(globals, NS_TCP, NET_TCP_SOURCE);
        globals.mark_loaded(NS_TCP);
    }

    if !globals.is_loaded(NS) {
        globals.get_or_create_ns(NS);
        globals.refer_all(NS, "clojure.core");
        load_source(globals, NS, NET_SOURCE);
        globals.mark_loaded(NS);
    }
}
