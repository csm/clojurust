//! Networking for clojurust — TCP, Unix, UDP, TLS over core.async channels.
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

pub mod frame;
pub mod h3;
mod pool_io;
pub mod quic;
pub mod quic_config;
pub mod tcp;
pub mod tls;
pub mod udp;
pub mod unix;

/// Clojure source for `clojure.rust.net.tcp`.
const NET_TCP_SOURCE: &str = include_str!("clojure_rust_net_tcp.cljrs");

/// Clojure source for the umbrella `clojure.rust.net` namespace.
const NET_SOURCE: &str = include_str!("clojure_rust_net.cljrs");

/// Clojure source for `clojure.rust.net.frame`.
const NET_FRAME_SOURCE: &str = include_str!("clojure_rust_net_frame.cljrs");

/// Clojure source for `clojure.rust.net.udp`.
const NET_UDP_SOURCE: &str = include_str!("clojure_rust_net_udp.cljrs");

/// Clojure source for `clojure.rust.net.tls`.
const NET_TLS_SOURCE: &str = include_str!("clojure_rust_net_tls.cljrs");

/// Clojure source for `clojure.rust.net.unix`.
const NET_UNIX_SOURCE: &str = include_str!("clojure_rust_net_unix.cljrs");

/// Clojure source for `clojure.rust.net.quic`.
const NET_QUIC_SOURCE: &str = include_str!("clojure_rust_net_quic.cljrs");

/// Clojure source for `clojure.rust.net.h3`.
const NET_H3_SOURCE: &str = include_str!("clojure_rust_net_h3.cljrs");

pub const NS_TCP: &str = "clojure.rust.net.tcp";
pub const NS: &str = "clojure.rust.net";
pub const NS_FRAME: &str = "clojure.rust.net.frame";
pub const NS_UDP: &str = "clojure.rust.net.udp";
pub const NS_TLS: &str = "clojure.rust.net.tls";
pub const NS_UNIX: &str = "clojure.rust.net.unix";
pub const NS_QUIC: &str = "clojure.rust.net.quic";
pub const NS_H3: &str = "clojure.rust.net.h3";

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

    if !globals.is_loaded(NS_FRAME) {
        globals.get_or_create_ns(NS_FRAME);
        globals.refer_all(NS_FRAME, "clojure.core");
        frame::register(globals, NS_FRAME);
        load_source(globals, NS_FRAME, NET_FRAME_SOURCE);
        globals.mark_loaded(NS_FRAME);
    }

    if !globals.is_loaded(NS_UDP) {
        globals.get_or_create_ns(NS_UDP);
        globals.refer_all(NS_UDP, "clojure.core");
        udp::register(globals, NS_UDP);
        load_source(globals, NS_UDP, NET_UDP_SOURCE);
        globals.mark_loaded(NS_UDP);
    }

    if !globals.is_loaded(NS_TLS) {
        globals.get_or_create_ns(NS_TLS);
        globals.refer_all(NS_TLS, "clojure.core");
        tls::register(globals, NS_TLS);
        load_source(globals, NS_TLS, NET_TLS_SOURCE);
        globals.mark_loaded(NS_TLS);
    }

    if !globals.is_loaded(NS_UNIX) {
        globals.get_or_create_ns(NS_UNIX);
        globals.refer_all(NS_UNIX, "clojure.core");
        unix::register(globals, NS_UNIX);
        load_source(globals, NS_UNIX, NET_UNIX_SOURCE);
        globals.mark_loaded(NS_UNIX);
    }

    if !globals.is_loaded(NS_QUIC) {
        globals.get_or_create_ns(NS_QUIC);
        globals.refer_all(NS_QUIC, "clojure.core");
        quic::register(globals, NS_QUIC);
        load_source(globals, NS_QUIC, NET_QUIC_SOURCE);
        globals.mark_loaded(NS_QUIC);
    }

    if !globals.is_loaded(NS_H3) {
        globals.get_or_create_ns(NS_H3);
        globals.refer_all(NS_H3, "clojure.core");
        h3::register(globals, NS_H3);
        load_source(globals, NS_H3, NET_H3_SOURCE);
        globals.mark_loaded(NS_H3);
    }

    if !globals.is_loaded(NS) {
        globals.get_or_create_ns(NS);
        globals.refer_all(NS, "clojure.core");
        load_source(globals, NS, NET_SOURCE);
        globals.mark_loaded(NS);
    }
}
