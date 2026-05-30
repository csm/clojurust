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

fn load_source(globals: &Arc<cljrs_env::env::GlobalEnv>, ns: &str, source: &str) {
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
