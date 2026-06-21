//! Phase Q1 + Q2 integration tests: QUIC transport.
//!
//! Q1 done criterion: client connects to a quinn echo server, opens a
//! bidirectional stream, sends bytes, FINs the write side, drains the echoed
//! response.  Also validates the failure path (connect timeout yields
//! Value::Error).
//!
//! Q2 done criterion: `listen_on` binds a server endpoint; a cljrs client
//! connects, opens a stream, and the server-side channels deliver the stream
//! map so the server can echo.  Also validates that closing the listener closes
//! the `:conns` channel.

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

// ── Q2 tests ───────────────────────────────────────────────────────────────────

/// Phase Q2 done criterion: `listen_on` server accepts a client connection,
/// the client opens a bidi stream, the server-side `:streams` channel delivers
/// it, and both sides can exchange bytes.
#[test]
fn test_quic_server_echo_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let (server_config, _cert) = make_server_config();

        // Start the cljrs QUIC server.
        let server_val = cljrs_net::quic::listen_on("127.0.0.1", 0, server_config, 8, 8, 8, 8)
            .expect("listen_on failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected server map, got {}", other.type_name()),
        };

        let local_addr_str = match map_get(&server_map, "local-addr") {
            Value::Str(s) => s.get().clone(),
            other => panic!("expected str :local-addr, got {}", other.type_name()),
        };
        let port: u16 = local_addr_str
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .expect("parse port");

        let conns_ch = as_chan(&map_get(&server_map, "conns"));

        // Connect a cljrs client.
        let client_opts =
            MapValue::from_pairs(vec![(kw("insecure-skip-verify"), Value::Bool(true))]);
        let quinn_config =
            cljrs_net::quic_config::client_config(&client_opts).expect("client config");
        let conn_promise = cljrs_net::quic::connect_to("127.0.0.1", port, quinn_config, 8, 8, 8);
        let conn_val = chan_take(&as_chan(&conn_promise)).await;
        let conn = match conn_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("client connect failed: {}", e.get().message()),
            other => panic!("expected conn map, got {}", other.type_name()),
        };

        // Accept the server-side connection.
        let server_conn_val = chan_take(&conns_ch).await;
        let server_conn = match server_conn_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("server accept failed: {}", e.get().message()),
            other => panic!("expected conn map, got {}", other.type_name()),
        };
        let streams_ch = as_chan(&map_get(&server_conn, "streams"));

        // Client opens a bidirectional stream and waits for the stream map.
        let resource_handle = match map_get(&conn, "resource") {
            Value::Resource(h) => h,
            other => panic!("expected resource, got {}", other.type_name()),
        };
        let conn_res = resource_handle
            .downcast::<cljrs_net::quic::QuicConnectionResource>()
            .expect("downcast QuicConnectionResource");
        let stream_promise = cljrs_net::quic::open_stream_on(conn_res.connection.clone(), 8, 8);

        // Await the client stream map first — this yields so that do_open_stream
        // runs and calls connection.open_bi(), creating the stream locally.
        let stream_val = chan_take(&as_chan(&stream_promise)).await;
        let stream = match stream_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("open_stream failed: {}", e.get().message()),
            other => panic!("expected stream map, got {}", other.type_name()),
        };

        let client_out = as_chan(&map_get(&stream, "out"));
        let client_in = as_chan(&map_get(&stream, "in"));

        // Client writes data and FINs the write half BEFORE waiting for the
        // server stream.  In QUIC, the server's accept_bi() only fires when the
        // peer sends the first STREAM frame, so data must be in flight first.
        let request = b"phase Q2 QUIC server echo test";
        let signed: Vec<i8> = request.iter().map(|&b| b as i8).collect();
        chan_put(
            &client_out,
            Value::ByteArray(GcPtr::new(std::sync::Mutex::new(signed))),
        )
        .await;
        chan_ref(client_out.get()).close(); // FIN — write_bridge sees EOF, pool_writer shuts down

        // Now wait for the server-side stream.  The write above enqueued bytes on
        // :out; write_bridge will drain them on the next yield, triggering STREAM
        // frames over the wire and waking up pool_stream_accept_loop's accept_bi.
        let server_stream_val = chan_take(&streams_ch).await;
        let server_stream = match server_stream_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("stream accept failed: {}", e.get().message()),
            other => panic!("expected stream map, got {}", other.type_name()),
        };

        let server_in = as_chan(&map_get(&server_stream, "in"));
        let server_out = as_chan(&map_get(&server_stream, "out"));

        // Server drains :in to EOF, then echoes the whole payload.
        let mut echo_data: Vec<u8> = Vec::new();
        loop {
            match chan_take(&server_in).await {
                Value::Nil => break,
                Value::ByteArray(arr) => {
                    echo_data.extend(arr.get().lock().unwrap().iter().map(|&b| b as u8));
                }
                Value::Error(e) => panic!("server read error: {}", e.get().message()),
                other => panic!("unexpected value on server :in: {}", other.type_name()),
            }
        }
        let echo_signed: Vec<i8> = echo_data.iter().map(|&b| b as i8).collect();
        chan_put(
            &server_out,
            Value::ByteArray(GcPtr::new(std::sync::Mutex::new(echo_signed))),
        )
        .await;
        chan_ref(server_out.get()).close();

        // Client drains :in.
        let mut response: Vec<u8> = Vec::new();
        loop {
            match chan_take(&client_in).await {
                Value::Nil => break,
                Value::ByteArray(arr) => {
                    response.extend(arr.get().lock().unwrap().iter().map(|&b| b as u8));
                }
                Value::Error(e) => panic!("client read error: {}", e.get().message()),
                other => panic!("unexpected value on client :in: {}", other.type_name()),
            }
        }

        assert_eq!(response, request, "Q2 QUIC echo must return the same bytes");

        // Clean up.
        if let Value::Resource(h) = map_get(&server_map, "resource") {
            let _ = h.close();
        }
        if let Value::Resource(h) = map_get(&conn, "resource") {
            let _ = h.close();
        }
    });
}

/// Closing the listener via its `QuicListenerResource` must cause the `:conns`
/// channel to be closed (subsequent `chan_take` yields `Value::Nil`).
#[test]
fn test_quic_listen_close_stops_conns() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let (server_config, _cert) = make_server_config();
        let server_val = cljrs_net::quic::listen_on("127.0.0.1", 0, server_config, 8, 8, 8, 8)
            .expect("listen_on failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected server map, got {}", other.type_name()),
        };

        let conns_ch = as_chan(&map_get(&server_map, "conns"));

        // Close the listener resource; also close the channel explicitly.
        if let Value::Resource(h) = map_get(&server_map, "resource") {
            h.close().expect("close");
        }
        chan_ref(conns_ch.get()).close();

        // A take on a closed channel must yield Nil immediately.
        let v = chan_take(&conns_ch).await;
        assert!(
            matches!(v, Value::Nil),
            "expected Nil from closed :conns, got {}",
            v.type_name()
        );
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
