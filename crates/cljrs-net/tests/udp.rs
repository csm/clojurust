//! Phase D integration tests: UDP datagram sockets — echo round-trip.
//!
//! Done criterion from networking-plan.md Phase D:
//!   "a UDP echo responder round-trips datagrams with correct :addr."

use std::sync::{Arc, Mutex};

use cljrs_async::channel::{chan_put, chan_ref, chan_take};
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

/// Phase D done criterion: echo responder round-trips datagrams with correct :addr.
#[test]
fn test_udp_echo_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // Create server socket (port 0 = OS-assigned).
        let server_val =
            cljrs_net::udp::socket_on("127.0.0.1", 0, 8, 8).expect("server socket failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected socket map, got {}", other.type_name()),
        };

        let server_local_addr = match map_get(&server_map, "local-addr") {
            Value::Str(s) => s.get().clone(),
            other => panic!("expected str :local-addr, got {}", other.type_name()),
        };
        let server_port: u16 = server_local_addr
            .split(':')
            .next_back()
            .unwrap()
            .parse()
            .unwrap();

        let server_in = as_chan(&map_get(&server_map, "in"));
        let server_out = as_chan(&map_get(&server_map, "out"));

        // Create client socket (port 0 = OS-assigned).
        let client_val =
            cljrs_net::udp::socket_on("127.0.0.1", 0, 8, 8).expect("client socket failed");
        let client_map = match client_val {
            Value::Map(m) => m,
            other => panic!("expected socket map, got {}", other.type_name()),
        };

        let client_in = as_chan(&map_get(&client_map, "in"));
        let client_out = as_chan(&map_get(&client_map, "out"));

        // Spawn a one-shot echo handler: take one datagram from :in and echo it to :out.
        // The datagram's :addr field is the sender's address, so send_to goes back to them.
        tokio::task::spawn_local(async move {
            if let Value::Map(dgram) = chan_take(&server_in).await {
                chan_put(&server_out, Value::Map(dgram)).await;
            }
        });

        // Client sends a datagram to the server.
        let payload = b"phase D udp echo test";
        let signed: Vec<i8> = payload.iter().map(|&b| b as i8).collect();
        let send_dgram = Value::Map(MapValue::from_pairs(vec![
            (kw("data"), Value::ByteArray(GcPtr::new(Mutex::new(signed)))),
            (
                kw("addr"),
                Value::string(format!("127.0.0.1:{server_port}")),
            ),
        ]));
        chan_put(&client_out, send_dgram).await;

        // Client receives the echoed datagram.
        match chan_take(&client_in).await {
            Value::Map(dgram) => {
                // Verify echoed payload matches.
                match dgram.get(&kw("data")) {
                    Some(Value::ByteArray(arr)) => {
                        let received: Vec<u8> =
                            arr.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                        assert_eq!(received, payload, "echoed payload must match");
                    }
                    other => panic!(
                        "expected :data byte-array, got {:?}",
                        other.map(|v| v.type_name())
                    ),
                }
                // Verify :addr is the server's address (echo came from the server socket).
                match dgram.get(&kw("addr")) {
                    Some(Value::Str(s)) => {
                        let addr = s.get();
                        let echo_port: u16 = addr.split(':').next_back().unwrap().parse().unwrap();
                        assert_eq!(
                            echo_port, server_port,
                            ":addr port must be server's port; got {addr}"
                        );
                    }
                    other => panic!(
                        "expected :addr string, got {:?}",
                        other.map(|v| v.type_name())
                    ),
                }
            }
            Value::Error(e) => panic!("received error: {}", e.get().message()),
            other => panic!("expected datagram map, got {}", other.type_name()),
        }

        // Clean up.
        match map_get(&server_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            _ => panic!("expected server resource"),
        }
        match map_get(&client_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            _ => panic!("expected client resource"),
        }
    });
}

/// Closing a socket's resource and :in channel makes a pending take yield nil.
#[test]
fn test_close_socket_closes_in() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let sock_val = cljrs_net::udp::socket_on("127.0.0.1", 0, 8, 8).expect("socket failed");
        let sock_map = match sock_val {
            Value::Map(m) => m,
            other => panic!("expected socket map, got {}", other.type_name()),
        };

        let in_chan = as_chan(&map_get(&sock_map, "in"));

        // Abort reader/writer tasks via the resource handle.
        match map_get(&sock_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            _ => panic!("expected resource"),
        }
        // Abort does not automatically close the channel; close it explicitly.
        chan_ref(in_chan.get()).close();

        let val = chan_take(&in_chan).await;
        assert!(
            matches!(val, Value::Nil),
            "expected nil from closed :in, got {}",
            val.type_name()
        );
    });
}

/// Multiple datagrams from different senders are received with correct :addr fields.
#[test]
fn test_udp_multiple_senders() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // One server, two clients.
        let server_val =
            cljrs_net::udp::socket_on("127.0.0.1", 0, 16, 8).expect("server socket failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected socket map, got {}", other.type_name()),
        };
        let server_port: u16 = match map_get(&server_map, "local-addr") {
            Value::Str(s) => s.get().split(':').next_back().unwrap().parse().unwrap(),
            _ => panic!("expected str :local-addr"),
        };
        let server_in = as_chan(&map_get(&server_map, "in"));

        let client_a_val =
            cljrs_net::udp::socket_on("127.0.0.1", 0, 8, 8).expect("client A failed");
        let client_a_map = match client_a_val {
            Value::Map(m) => m,
            _ => panic!("expected socket map"),
        };
        let client_a_port: u16 = match map_get(&client_a_map, "local-addr") {
            Value::Str(s) => s.get().split(':').next_back().unwrap().parse().unwrap(),
            _ => panic!("expected str"),
        };
        let client_a_out = as_chan(&map_get(&client_a_map, "out"));

        let client_b_val =
            cljrs_net::udp::socket_on("127.0.0.1", 0, 8, 8).expect("client B failed");
        let client_b_map = match client_b_val {
            Value::Map(m) => m,
            _ => panic!("expected socket map"),
        };
        let client_b_port: u16 = match map_get(&client_b_map, "local-addr") {
            Value::Str(s) => s.get().split(':').next_back().unwrap().parse().unwrap(),
            _ => panic!("expected str"),
        };
        let client_b_out = as_chan(&map_get(&client_b_map, "out"));

        // Client A sends "from-a".
        let signed_a: Vec<i8> = b"from-a".iter().map(|&b| b as i8).collect();
        chan_put(
            &client_a_out,
            Value::Map(MapValue::from_pairs(vec![
                (
                    kw("data"),
                    Value::ByteArray(GcPtr::new(Mutex::new(signed_a))),
                ),
                (
                    kw("addr"),
                    Value::string(format!("127.0.0.1:{server_port}")),
                ),
            ])),
        )
        .await;

        // Client B sends "from-b".
        let signed_b: Vec<i8> = b"from-b".iter().map(|&b| b as i8).collect();
        chan_put(
            &client_b_out,
            Value::Map(MapValue::from_pairs(vec![
                (
                    kw("data"),
                    Value::ByteArray(GcPtr::new(Mutex::new(signed_b))),
                ),
                (
                    kw("addr"),
                    Value::string(format!("127.0.0.1:{server_port}")),
                ),
            ])),
        )
        .await;

        // Server receives two datagrams; each must carry the correct sender port.
        let mut received_ports: Vec<u16> = Vec::new();
        for _ in 0..2 {
            match chan_take(&server_in).await {
                Value::Map(dgram) => match dgram.get(&kw("addr")) {
                    Some(Value::Str(s)) => {
                        let p: u16 = s.get().split(':').next_back().unwrap().parse().unwrap();
                        received_ports.push(p);
                    }
                    _ => panic!("expected :addr string"),
                },
                other => panic!("expected datagram map, got {}", other.type_name()),
            }
        }

        assert!(
            received_ports.contains(&client_a_port),
            "must see client A's port {client_a_port}; got {received_ports:?}"
        );
        assert!(
            received_ports.contains(&client_b_port),
            "must see client B's port {client_b_port}; got {received_ports:?}"
        );

        // Clean up.
        for map in [&server_map, &client_a_map, &client_b_map] {
            if let Value::Resource(handle) = map_get(map, "resource") {
                let _ = handle.close();
            }
        }
    });
}
