//! Phase E integration tests: TLS client + server — echo round-trip and error path.
//!
//! Done criterion from networking-plan.md Phase E:
//!   "TLS client and server round-trip bytes over an encrypted channel."

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

/// Write cert and key PEM strings to temp files; return (cert_path, key_path).
fn write_temp_pem(cert_pem: &str, key_pem: &str) -> (String, String) {
    let dir = std::env::temp_dir();
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos();
    let cert_path = dir
        .join(format!("cljrs_tls_test_cert_{unique}.pem"))
        .to_string_lossy()
        .into_owned();
    let key_path = dir
        .join(format!("cljrs_tls_test_key_{unique}.pem"))
        .to_string_lossy()
        .into_owned();
    std::fs::write(&cert_path, cert_pem).expect("write cert pem");
    std::fs::write(&key_path, key_pem).expect("write key pem");
    (cert_path, key_path)
}

/// Phase E done criterion: TLS echo server from `tls_listen_on` + client round-trips bytes.
#[test]
fn test_tls_echo_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // Generate a self-signed certificate for "localhost".
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("rcgen cert generation failed");
        let cert_pem = cert.cert.pem();
        let key_pem = cert.key_pair.serialize_pem();

        // Write PEM files to temp dir.
        let (cert_path, key_path) = write_temp_pem(&cert_pem, &key_pem);

        // Build server config from PEM files.
        let server_opts = cljrs_value::MapValue::from_pairs(vec![
            (kw("cert"), Value::string(cert_path.clone())),
            (kw("key"), Value::string(key_path.clone())),
        ]);
        let server_config =
            cljrs_net::tls::build_server_config(&server_opts).expect("build_server_config failed");

        // Start the TLS listener (port 0 = OS-assigned).
        let server_val = cljrs_net::tls::tls_listen_on("127.0.0.1", 0, server_config, 8, 8, 8)
            .expect("tls_listen_on failed");
        let server_map = match server_val {
            Value::Map(m) => m,
            other => panic!("expected server map, got {}", other.type_name()),
        };

        // Extract port from :local-addr.
        let local_addr = match map_get(&server_map, "local-addr") {
            Value::Str(s) => s.get().clone(),
            other => panic!("expected str :local-addr, got {}", other.type_name()),
        };
        let port: u16 = local_addr.split(':').next_back().unwrap().parse().unwrap();

        let conns_ch = as_chan(&map_get(&server_map, "conns"));

        // Spawn a one-shot echo handler.
        let conns_for_handler = conns_ch.clone();
        tokio::task::spawn_local(async move {
            let conn_val = chan_take(&conns_for_handler).await;
            let conn = match conn_val {
                Value::Map(m) => m,
                Value::Error(e) => panic!("handler got error from :conns: {}", e.get().message()),
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

        // Build insecure client config (accepts self-signed cert).
        let client_opts = cljrs_value::MapValue::from_pairs(vec![(
            kw("insecure-skip-verify"),
            Value::Bool(true),
        )]);
        let client_config =
            cljrs_net::tls::build_client_config(&client_opts).expect("build_client_config failed");

        // Connect the TLS client.
        let promise = cljrs_net::tls::tls_connect_to("localhost", port, client_config, 8, 8);
        let conn_val = chan_take(&as_chan(&promise)).await;
        let conn = match conn_val {
            Value::Map(m) => m,
            Value::Error(e) => panic!("TLS connect failed: {}", e.get().message()),
            other => panic!("expected conn map, got {}", other.type_name()),
        };

        let out_ch = as_chan(&map_get(&conn, "out"));
        let in_ch = as_chan(&map_get(&conn, "in"));

        // Send a request and half-close the write side.
        let request = b"phase E TLS echo test";
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

        assert_eq!(
            response, request,
            "TLS echo server must return the same bytes"
        );

        // Cleanup temp files.
        let _ = std::fs::remove_file(&cert_path);
        let _ = std::fs::remove_file(&key_path);

        // Close the server listener.
        match map_get(&server_map, "resource") {
            Value::Resource(handle) => {
                let _ = handle.close();
            }
            other => panic!("expected resource, got {}", other.type_name()),
        }
    });
}

/// A TLS connection attempt to a port that isn't listening must yield Value::Error.
#[test]
fn test_tls_connect_failure() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // Build a minimal client config (insecure to avoid cert issues — we
        // won't get that far anyway since there's no server).
        let client_opts = cljrs_value::MapValue::from_pairs(vec![(
            kw("insecure-skip-verify"),
            Value::Bool(true),
        )]);
        let client_config =
            cljrs_net::tls::build_client_config(&client_opts).expect("build_client_config failed");

        // Port 1 is privileged and almost certainly not listening.
        let promise = cljrs_net::tls::tls_connect_to("127.0.0.1", 1, client_config, 8, 8);

        let result = chan_take(&as_chan(&promise)).await;
        assert!(
            matches!(result, Value::Error(_)),
            "expected Value::Error for refused TLS connection, got {}",
            result.type_name()
        );
    });
}
