//! Phase B integration tests: TCP server — listen + accept loop round-trips bytes.
//!
//! Done criterion from networking-plan.md Phase B:
//!   "an echo server built from listen + go round-trips bytes from a connect client."

use std::sync::Arc;

use cljrs_async::channel::{chan_put, chan_ref, chan_take};
use cljrs_gc::GcPtr;
use cljrs_value::{Keyword, NativeObjectBox, Value};

fn setup_globals() -> Arc<cljrs_env::env::GlobalEnv> {
    let globals = cljrs_stdlib::standard_env();
    cljrs_net::init(&globals);
    globals
}

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

fn map_get(map: &cljrs_value::MapValue, key: &str) -> Value {
    map.get(&kw(key))
        .unwrap_or_else(|| panic!("map missing :{key}"))
}

fn as_chan(v: &Value) -> GcPtr<NativeObjectBox> {
    match v {
        Value::NativeObject(obj) => obj.clone(),
        other => panic!("expected NativeObject (channel), got {}", other.type_name()),
    }
}

/// Phase B done criterion: echo server from `listen_on` + accept handler round-trips bytes.
#[test]
fn test_listen_echo_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // Start the server (port 0 = OS-assigned).
        let server_val =
            cljrs_net::tcp::listen_on("127.0.0.1", 0, 8, 8, 8).expect("listen_on failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected server map, got {}", other.type_name()),
        };

        // Extract the port from :local-addr ("127.0.0.1:PORT").
        let local_addr = match map_get(&server_map, "local-addr") {
            Value::Str(s) => s.get().clone(),
            other => panic!("expected str :local-addr, got {}", other.type_name()),
        };
        let port: u16 = local_addr.split(':').last().unwrap().parse().unwrap();

        let conns_ch = as_chan(&map_get(&server_map, "conns"));

        // Spawn a one-shot echo handler: accept one connection, echo all bytes.
        let conns_for_handler = conns_ch.clone();
        tokio::task::spawn_local(async move {
            let conn_val = chan_take(&conns_for_handler).await;
            let conn = match conn_val {
                Value::Map(m) => m,
                other => panic!("handler got non-map from :conns: {}", other.type_name()),
            };

            let in_ch = as_chan(&map_get(&conn, "in"));
            let out_ch = as_chan(&map_get(&conn, "out"));

            loop {
                match chan_take(&in_ch).await {
                    Value::Nil => break,
                    val => {
                        chan_put(&out_ch, val).await;
                    }
                }
            }
            chan_ref(out_ch.get()).close();
        });

        // Connect a client.
        let promise = cljrs_net::tcp::connect_to("127.0.0.1", port, 8, 8);
        let conn_val = chan_take(&as_chan(&promise)).await;
        let conn = match conn_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("connect failed: {}", e.get().message()),
            other => panic!("expected conn map, got {}", other.type_name()),
        };

        let out_ch = as_chan(&map_get(&conn, "out"));
        let in_ch = as_chan(&map_get(&conn, "in"));

        // Send a request and half-close the write side.
        let request = b"phase B echo test";
        let signed: Vec<i8> = request.iter().map(|&b| b as i8).collect();
        chan_put(
            &out_ch,
            Value::ByteArray(GcPtr::new(std::sync::Mutex::new(signed))),
        )
        .await;
        chan_ref(out_ch.get()).close();

        // Drain :in until EOF, collecting the echoed bytes.
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
                other => panic!("unexpected value on :in: {}", other.type_name()),
            }
        }

        assert_eq!(response, request, "echo server must return the same bytes");

        // Close the server listener.
        match map_get(&server_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            other => panic!("expected resource, got {}", other.type_name()),
        }
    });
}

/// Closing the server closes :conns so consumers see nil.
#[test]
fn test_close_server_closes_conns() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let server_val =
            cljrs_net::tcp::listen_on("127.0.0.1", 0, 8, 8, 8).expect("listen_on failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected server map, got {}", other.type_name()),
        };

        let conns_ch = as_chan(&map_get(&server_map, "conns"));

        // Close the server (abort accept loop + close :conns).
        match map_get(&server_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            _ => panic!("expected resource"),
        }
        chan_ref(conns_ch.get()).close();

        // A take on the closed :conns must yield nil immediately.
        let val = chan_take(&conns_ch).await;
        assert!(
            matches!(val, Value::Nil),
            "expected nil from closed :conns, got {}",
            val.type_name()
        );
    });
}
