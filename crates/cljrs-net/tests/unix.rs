//! Phase F integration tests: Unix-domain stream sockets — echo round-trip.
//!
//! Done criterion from networking-plan.md Phase F:
//!   "a Unix-socket echo server round-trips with a Unix-socket client."
//!
//! All tests are `#[cfg(unix)]`; on non-Unix targets this file compiles to an
//! empty module.

#[cfg(unix)]
mod unix_tests {
    use std::sync::Arc;

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

    /// Unique socket path per test — PID + suffix avoids collisions in parallel runs.
    fn sock(suffix: &str) -> String {
        format!("/tmp/cljrs_net_unix_{}_{suffix}.sock", std::process::id())
    }

    // ── Phase F done criterion ────────────────────────────────────────────────

    /// Echo server built from `listen_on` + accept handler round-trips bytes.
    #[test]
    fn test_unix_echo_round_trip() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            let _globals = setup_globals();

            let path = sock("echo");

            let server_val =
                cljrs_net::unix::listen_on(&path, 8, 8, 8).expect("listen_on failed");
            let server_map = match server_val {
                Value::Map(m) => m,
                other => panic!("expected server map, got {}", other.type_name()),
            };

            let conns_ch = as_chan(&map_get(&server_map, "conns"));

            // Spawn a one-shot echo handler.
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
            let promise = cljrs_net::unix::connect_to(&path, 8, 8);
            let conn_val = chan_take(&as_chan(&promise)).await;
            let conn = match conn_val {
                Value::Map(m) => m,
                Value::Error(e) => panic!("connect failed: {}", e.get().message()),
                other => panic!("expected conn map, got {}", other.type_name()),
            };

            let out_ch = as_chan(&map_get(&conn, "out"));
            let in_ch = as_chan(&map_get(&conn, "in"));

            // Send request and half-close the write side.
            let request = b"phase F unix socket echo test";
            let signed: Vec<i8> = request.iter().map(|&b| b as i8).collect();
            chan_put(
                &out_ch,
                Value::ByteArray(GcPtr::new(std::sync::Mutex::new(signed))),
            )
            .await;
            chan_ref(out_ch.get()).close();

            // Drain :in until EOF, collecting echoed bytes.
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

            // Close the listener; this also unlinks the socket path.
            match map_get(&server_map, "resource") {
                Value::Resource(handle) => {
                    let _ = handle.close();
                }
                other => panic!("expected resource, got {}", other.type_name()),
            }
        });
    }

    // ── Server lifecycle ──────────────────────────────────────────────────────

    /// Closing the server resource closes :conns so consumers see nil.
    #[test]
    fn test_close_unix_server_closes_conns() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            let _globals = setup_globals();

            let path = sock("close_conns");

            let server_val =
                cljrs_net::unix::listen_on(&path, 8, 8, 8).expect("listen_on failed");
            let server_map = match server_val {
                Value::Map(m) => m,
                other => panic!("expected server map, got {}", other.type_name()),
            };

            let conns_ch = as_chan(&map_get(&server_map, "conns"));

            match map_get(&server_map, "resource") {
                Value::Resource(handle) => {
                    let _ = handle.close();
                }
                _ => panic!("expected resource"),
            }
            chan_ref(conns_ch.get()).close();

            let val = chan_take(&conns_ch).await;
            assert!(
                matches!(val, Value::Nil),
                "expected nil from closed :conns, got {}",
                val.type_name()
            );
        });
    }

    /// `UnixListenerResource::close` unlinks the socket path from the filesystem.
    #[test]
    fn test_unix_listener_unlinks_path() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            let _globals = setup_globals();

            let path = sock("unlink");

            let server_val =
                cljrs_net::unix::listen_on(&path, 8, 8, 8).expect("listen_on failed");
            let server_map = match server_val {
                Value::Map(m) => m,
                _ => panic!("expected server map"),
            };

            assert!(
                std::path::Path::new(&path).exists(),
                "socket file must exist while listener is open"
            );

            match map_get(&server_map, "resource") {
                Value::Resource(handle) => {
                    let _ = handle.close();
                }
                _ => panic!("expected resource"),
            }

            // Allow the abort to propagate.
            tokio::task::yield_now().await;

            assert!(
                !std::path::Path::new(&path).exists(),
                "socket file must be unlinked after listener close"
            );
        });
    }

    /// `listen_on` on a path where a stale socket file exists succeeds (pre-unlinks).
    #[test]
    fn test_listen_on_removes_stale_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();

        local.block_on(&rt, async {
            let _globals = setup_globals();

            let path = sock("stale");

            // Create a stale socket file without a listening server.
            std::fs::write(&path, b"").expect("create stale file");
            assert!(std::path::Path::new(&path).exists());

            // listen_on should remove it and bind successfully.
            let server_val = cljrs_net::unix::listen_on(&path, 8, 8, 8)
                .expect("listen_on should succeed despite stale socket");
            let server_map = match server_val {
                Value::Map(m) => m,
                other => panic!("expected server map, got {}", other.type_name()),
            };

            match map_get(&server_map, "resource") {
                Value::Resource(handle) => {
                    let _ = handle.close();
                }
                _ => panic!("expected resource"),
            }
        });
    }
}
