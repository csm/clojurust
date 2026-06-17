//! Build quinn client/server configs from Clojure opts maps.
//!
//! Delegates TLS layer construction to `tls::build_client_config` /
//! `tls::build_server_config`, then wraps the resulting rustls configs into
//! quinn via `QuicClientConfig::try_from` / `QuicServerConfig::try_from`.
//! Transport parameters (`:max-idle-ms`, `:keep-alive-ms`, `:max-streams`) are
//! applied to a `quinn::TransportConfig` and attached to the returned config.

use std::sync::Arc;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};

use cljrs_value::{Keyword, MapValue, Value, ValueError, ValueResult};

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn opts_u64(opts: &MapValue, key: &str) -> Option<u64> {
    match opts.get(&kw(key))? {
        Value::Long(n) if n > 0 => Some(n as u64),
        _ => None,
    }
}

fn transport_config(opts: &MapValue) -> Arc<quinn::TransportConfig> {
    let mut transport = quinn::TransportConfig::default();

    if let Some(ms) = opts_u64(opts, "max-idle-ms") {
        if let Ok(v) = quinn::VarInt::try_from(ms) {
            transport.max_idle_timeout(Some(quinn::IdleTimeout::from(v)));
        }
    }

    if let Some(ms) = opts_u64(opts, "keep-alive-ms") {
        transport.keep_alive_interval(Some(Duration::from_millis(ms)));
    }

    if let Some(n) = opts_u64(opts, "max-streams") {
        if let Ok(v) = quinn::VarInt::try_from(n) {
            transport.max_concurrent_bidi_streams(v);
        }
    }

    Arc::new(transport)
}

/// Build a `quinn::ClientConfig` from a Clojure opts map.
///
/// All TLS options (`:insecure-skip-verify`, `:alpn`, `:roots`) are forwarded
/// to `tls::build_client_config`. QUIC transport params (`:max-idle-ms`,
/// `:keep-alive-ms`, `:max-streams`) are applied via `TransportConfig`.
pub fn client_config(opts: &MapValue) -> ValueResult<quinn::ClientConfig> {
    let rustls_cfg = crate::tls::build_client_config(opts)?;
    let quic_cfg = QuicClientConfig::try_from(rustls_cfg)
        .map_err(|e| ValueError::Other(format!("quinn client config: {e}")))?;
    let mut config = quinn::ClientConfig::new(Arc::new(quic_cfg));
    config.transport_config(transport_config(opts));
    Ok(config)
}

/// Build a `quinn::ServerConfig` from a Clojure opts map.
///
/// Requires `:cert` and `:key` (PEM file paths). All other TLS options are
/// forwarded to `tls::build_server_config`. QUIC transport params are applied.
pub fn server_config(opts: &MapValue) -> ValueResult<quinn::ServerConfig> {
    let rustls_cfg = crate::tls::build_server_config(opts)?;
    let quic_cfg = QuicServerConfig::try_from(rustls_cfg)
        .map_err(|e| ValueError::Other(format!("quinn server config: {e}")))?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(quic_cfg));
    config.transport_config(transport_config(opts));
    Ok(config)
}
