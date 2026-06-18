//! Phase Q1 integration tests: QUIC client transport.
//!
//! Done criterion: client connects to a quinn echo server, opens a bidirectional
//! stream, sends bytes, FINs the write side, drains the echoed response.
//! Also validates the failure path (connect timeout to a dead UDP port yields
//! Value::Error).

use std::sync::Arc;

use cljrs_async::channel::{chan_put, chan_ref, chan_take};
use cljrs_async::worker_pool::WorkerPool;
use cljrs_gc::GcPtr;
use cljrs_value::{Keyword, MapValue, NativeObjectBox, Value};

fn setup_globals() -> Arc<cljrs_env::env::GlobalEnv> {
    let globals = cljrs_stdlib::standard_env();
    cljrs_net::init(&globals);
    globals
}

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn map_get(map: &MapValue, key: &str) -> Value {
    map.get(&kw(key))
        .unwrap_or_else(|| panic!("map missing :{key}"))
}

fn as_chan(v: &Value) -> GcPtr<NativeObjectBox> {
    match v {
        Value::NativeObject(obj) => obj.clone(),
        other => panic!("expected NativeObject (channel), got {}", other.type_name()),
    }
}

/// Build a self-signed rustls ServerConfig + QuicServerConfig using rcgen.
fn make_server_config() -> (quinn::ServerConfig, rcgen::CertifiedKey) {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("rcgen cert generation failed");

    let cert_der = rustls::pki_types::CertificateDer::from(certified.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());

    let rustls_cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("tls versions")
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .expect("server cert");

    let quic_cfg = quinn::crypto::rustls::QuicServerConfig::try_from(Arc::new(rustls_cfg))
        .expect("quic server config");

    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_cfg));
    (server_config, certified)
}

/// Spawn a simple QUIC echo server on the WorkerPool.
/// Returns the bound port.
fn spawn_echo_server(server_config: quinn::ServerConfig) -> u16 {
    let endpoint = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap())
        .expect("server endpoint");
    let port = endpoint.local_addr().unwrap().port();

    WorkerPool::global().handle().spawn(async move {
        use tokio::io::AsyncWriteExt;
        while let Some(incoming) = endpoint.accept().await {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                loop {
                    match conn.accept_bi().await {
                        Err(_) => break,
                        Ok((mut send, mut recv)) => {
                            tokio::spawn(async move {
                                // quinn's own read_to_end takes a byte limit
                                let data = recv.read_to_end(1 << 20).await.unwrap_or_default();
                                send.write_all(&data).await.ok();
                                send.shutdown().await.ok();
                            });
                        }
                    }
                }
            });
        }
    });

    port
}

/// Phase Q1 done criterion: QUIC client connects, opens a stream, echoes bytes.
#[test]
fn test_quic_echo_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let (server_config, _cert) = make_server_config();
        let port = spawn_echo_server(server_config);

        // Build insecure client config (accepts self-signed cert).
        let client_opts =
            MapValue::from_pairs(vec![(kw("insecure-skip-verify"), Value::Bool(true))]);
        let quinn_config =
            cljrs_net::quic_config::client_config(&client_opts).expect("client config");

        // Connect.
        let promise = cljrs_net::quic::connect_to("127.0.0.1", port, quinn_config, 8, 8, 8);
        let conn_val = chan_take(&as_chan(&promise)).await;
        let conn = match conn_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("QUIC connect failed: {}", e.get().message()),
            other => panic!("expected map, got {}", other.type_name()),
        };

        // Extract connection resource to open a stream.
        let resource_handle = match map_get(&conn, "resource") {
            Value::Resource(h) => h,
            other => panic!("expected resource, got {}", other.type_name()),
        };
        let conn_res = resource_handle
            .downcast::<cljrs_net::quic::QuicConnectionResource>()
            .expect("QuicConnectionResource downcast");
        let connection = conn_res.connection.clone();

        // Open a bidirectional stream.
        let stream_promise = cljrs_net::quic::open_stream_on(connection, 8, 8);
        let stream_val = chan_take(&as_chan(&stream_promise)).await;
        let stream = match stream_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("open_stream failed: {}", e.get().message()),
            other => panic!("expected stream map, got {}", other.type_name()),
        };

        // Verify stream-id is present and non-negative.
        match map_get(&stream, "stream-id") {
            Value::Long(id) => assert!(id >= 0, "stream-id must be non-negative"),
            other => panic!("expected long :stream-id, got {}", other.type_name()),
        }

        let out_ch = as_chan(&map_get(&stream, "out"));
        let in_ch = as_chan(&map_get(&stream, "in"));

        // Send bytes and FIN the write side.
        let request = b"phase Q1 QUIC echo test";
        let signed: Vec<i8> = request.iter().map(|&b| b as i8).collect();
        chan_put(
            &out_ch,
            Value::ByteArray(GcPtr::new(std::sync::Mutex::new(signed))),
        )
        .await;
        chan_ref(out_ch.get()).close();

        // Drain :in until stream EOF.
        let mut response: Vec<u8> = Vec::new();
        loop {
            match chan_take(&in_ch).await {
                Value::Nil => break,
                Value::ByteArray(arr) => {
                    let bytes: Vec<u8> =
                        arr.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                    response.extend_from_slice(&bytes);
                }
                Value::Error(e) => panic!("read error: {}", e.get().message()),
                other => panic!("unexpected on :in: {}", other.type_name()),
            }
        }

        assert_eq!(response, request, "QUIC echo must return the same bytes");

        // Close the connection.
        if let Value::Resource(handle) = map_get(&conn, "resource") {
            let _ = handle.close();
        }
    });
}

/// A QUIC connect attempt to a port that is not listening must yield Value::Error.
#[test]
fn test_quic_connect_failure() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let client_opts =
            MapValue::from_pairs(vec![(kw("insecure-skip-verify"), Value::Bool(true))]);
        let quinn_config =
            cljrs_net::quic_config::client_config(&client_opts).expect("client config");

        // Port 1 — privileged, not listening; QUIC will time out or get ICMP
        // unreachable.
        let promise = cljrs_net::quic::connect_to("127.0.0.1", 1, quinn_config, 8, 8, 8);

        let result = chan_take(&as_chan(&promise)).await;
        assert!(
            matches!(result, Value::Error(_)),
            "expected Value::Error for unreachable QUIC port, got {}",
            result.type_name()
        );
    });
}
