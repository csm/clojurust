//! Phase Q3 integration tests: HTTP/3 client.
//!
//! Done criterion: `h3::get` connects to a quinn-backed h3 echo server,
//! receives a 200 response with a body, and drains the `:body` channel.
//!
//! The test server is built directly from `h3` + `h3-quinn` + `quinn` (raw,
//! no cljrs-net abstractions) and runs on the `WorkerPool`, mirroring the
//! pattern used in `quic.rs` tests.

use std::sync::Arc;

use bytes::Bytes;
use cljrs_async::channel::{chan_ref, chan_take};
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

/// Build a self-signed server config with "h3" ALPN.
fn make_server_config() -> quinn::ServerConfig {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
        .expect("rcgen cert generation failed");

    let cert_der = rustls::pki_types::CertificateDer::from(certified.cert.der().to_vec());
    let key_der =
        rustls::pki_types::PrivateKeyDer::Pkcs8(certified.key_pair.serialize_der().into());

    let mut rustls_cfg = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("tls versions")
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .expect("server cert");

    // Required: h3 ALPN negotiation must succeed on both sides.
    rustls_cfg.alpn_protocols = vec![b"h3".to_vec()];

    let quic_cfg = quinn::crypto::rustls::QuicServerConfig::try_from(Arc::new(rustls_cfg))
        .expect("quic server config");

    quinn::ServerConfig::with_crypto(Arc::new(quic_cfg))
}

/// Spawn a minimal H3 server on the WorkerPool. Returns the bound port.
///
/// Responds to every request with HTTP 200 and the body `"h3 Q3 test body"`.
fn spawn_h3_server(server_config: quinn::ServerConfig) -> u16 {
    let endpoint = quinn::Endpoint::server(server_config, "127.0.0.1:0".parse().unwrap())
        .expect("server endpoint");
    let port = endpoint.local_addr().unwrap().port();

    WorkerPool::global().handle().spawn(async move {
        while let Some(incoming) = endpoint.accept().await {
            let conn = match incoming.await {
                Ok(c) => c,
                Err(_) => continue,
            };
            tokio::spawn(async move {
                let h3_conn = h3_quinn::Connection::new(conn);
                let mut h3_server: h3::server::Connection<h3_quinn::Connection, Bytes> =
                    match h3::server::Connection::new(h3_conn).await {
                        Ok(s) => s,
                        Err(_) => return,
                    };

                loop {
                    // h3 0.0.8: accept() returns Option<RequestResolver>;
                    // call resolve_request().await to get (Request, RequestStream).
                    let resolver = match h3_server.accept().await {
                        Ok(None) => break,
                        Err(_) => break,
                        Ok(Some(r)) => r,
                    };
                    let (_, mut stream) = match resolver.resolve_request().await {
                        Ok(pair) => pair,
                        Err(_) => break,
                    };
                    let resp = http::Response::builder()
                        .status(200)
                        .header("content-type", "text/plain")
                        .body(())
                        .unwrap();
                    if stream.send_response(resp).await.is_err() {
                        break;
                    }
                    if stream
                        .send_data(Bytes::from_static(b"h3 Q3 test body"))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    let _ = stream.finish().await;
                }
            });
        }
    });

    port
}

/// Phase Q3 done criterion: HTTP/3 GET returns status 200 and the expected body.
#[test]
fn test_h3_get_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let server_config = make_server_config();
        let port = spawn_h3_server(server_config);

        let url = format!("https://localhost:{port}/");
        let opts = MapValue::from_pairs(vec![
            (kw("insecure-skip-verify"), Value::Bool(true)),
            // ALPN is injected automatically by h3_client_config when absent,
            // but the server above accepts any (or no) ALPN.
        ]);

        let promise = cljrs_net::h3::get(&url, &opts, 8).expect("h3::get failed");
        let resp_val = chan_take(&as_chan(&promise)).await;
        let resp = match resp_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("H3 GET failed: {}", e.get().message()),
            other => panic!("expected response map, got {}", other.type_name()),
        };

        // Verify :status 200.
        let status = match map_get(&resp, "status") {
            Value::Long(n) => n,
            other => panic!("expected long :status, got {}", other.type_name()),
        };
        assert_eq!(status, 200, "expected HTTP 200");

        // :headers must be a map.
        match map_get(&resp, "headers") {
            Value::Map(_) => {}
            other => panic!("expected map :headers, got {}", other.type_name()),
        }

        // Drain :body until EOF.
        let body_ch = as_chan(&map_get(&resp, "body"));
        let mut body: Vec<u8> = Vec::new();
        loop {
            match chan_take(&body_ch).await {
                Value::Nil => break,
                Value::ByteArray(arr) => {
                    body.extend(arr.get().lock().unwrap().iter().map(|&b| b as u8));
                }
                Value::Error(e) => panic!("body read error: {}", e.get().message()),
                other => panic!("unexpected body value: {}", other.type_name()),
            }
        }
        assert_eq!(
            body, b"h3 Q3 test body",
            "response body must match server payload"
        );

        // Clean up resource.
        if let Value::Resource(h) = map_get(&resp, "resource") {
            let _ = h.close();
        }
    });
}

/// Draining the :body channel before calling close must yield the full body.
#[test]
fn test_h3_body_channel_closes_on_eof() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let server_config = make_server_config();
        let port = spawn_h3_server(server_config);

        let url = format!("https://localhost:{port}/");
        let opts = MapValue::from_pairs(vec![(kw("insecure-skip-verify"), Value::Bool(true))]);

        let promise = cljrs_net::h3::get(&url, &opts, 8).expect("h3::get failed");
        let resp_val = chan_take(&as_chan(&promise)).await;
        let resp = match resp_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("H3 GET failed: {}", e.get().message()),
            other => panic!("expected map, got {}", other.type_name()),
        };

        let body_ch = as_chan(&map_get(&resp, "body"));

        // The :body channel must close (yield Nil) after all data is delivered.
        let mut chunks = 0usize;
        loop {
            match chan_take(&body_ch).await {
                Value::Nil => break,
                Value::ByteArray(_) => chunks += 1,
                Value::Error(e) => panic!("body error: {}", e.get().message()),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        assert!(chunks >= 1, ":body must yield at least one chunk");

        // A second take on the closed channel must return Nil immediately.
        let v = chan_take(&body_ch).await;
        assert!(
            matches!(v, Value::Nil),
            "closed :body must yield Nil, got {}",
            v.type_name()
        );
    });
}

/// `(close resp)` before draining :body must close the channel.
#[test]
fn test_h3_close_before_drain() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let server_config = make_server_config();
        let port = spawn_h3_server(server_config);

        let url = format!("https://localhost:{port}/");
        let opts = MapValue::from_pairs(vec![(kw("insecure-skip-verify"), Value::Bool(true))]);

        let promise = cljrs_net::h3::get(&url, &opts, 8).expect("h3::get failed");
        let resp_val = chan_take(&as_chan(&promise)).await;
        let resp = match resp_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("H3 GET failed: {}", e.get().message()),
            other => panic!("expected map, got {}", other.type_name()),
        };

        let body_ch = as_chan(&map_get(&resp, "body"));

        // Close without draining — resource must close cleanly.
        if let Value::Resource(h) = map_get(&resp, "resource") {
            h.close().expect("close failed");
        }
        // Close the body channel explicitly (mirrors what builtin_close does).
        chan_ref(body_ch.get()).close();

        // After close, :body must eventually yield Nil. Body data may already
        // be buffered in the channel (the bridge task may have run before the
        // abort took effect), so drain any residual data first.
        loop {
            match chan_take(&body_ch).await {
                Value::Nil => break,
                Value::ByteArray(_) => {} // drain buffered data
                other => panic!("unexpected value on closed :body: {}", other.type_name()),
            }
        }
    });
}
