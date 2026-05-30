//! Phase A integration tests: TCP client connect / read / write / close.
//!
//! Done criterion from networking-plan.md Phase A:
//!   "a client can connect, (>! out req-bytes), (close! out), and drain :in to EOF."

use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use cljrs_async::channel::CljChannel;
use cljrs_gc::GcPtr;
use cljrs_value::{Keyword, Value};

// ── Test helpers ──────────────────────────────────────────────────────────────

fn setup_globals() -> Arc<cljrs_env::env::GlobalEnv> {
    let globals = cljrs_stdlib::standard_env();
    cljrs_net::init(&globals);
    globals
}

fn chan_of(v: &Value) -> &CljChannel {
    match v {
        Value::NativeObject(obj) => obj
            .get()
            .downcast_ref::<CljChannel>()
            .expect("expected CljChannel"),
        other => panic!("expected NativeObject (channel), got {}", other.type_name()),
    }
}

fn conn_field(conn: &cljrs_value::MapValue, key: &str) -> Value {
    conn.get(&Value::keyword(Keyword::simple(key)))
        .unwrap_or_else(|| panic!("connection map missing :{key}"))
}

// ── Echo server ───────────────────────────────────────────────────────────────

/// Accept one connection, read until EOF, echo the bytes back, and close.
async fn run_echo_server(listener: TcpListener) {
    let (mut stream, _) = listener.accept().await.unwrap();
    let mut buf = vec![0u8; 4096];
    let mut received = Vec::new();
    loop {
        match stream.read(&mut buf).await.unwrap() {
            0 => break,
            n => received.extend_from_slice(&buf[..n]),
        }
    }
    stream.write_all(&received).await.unwrap();
    // stream drop closes the connection, sending EOF to the client
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Phase A done criterion: connect → put bytes on :out → close! :out → drain :in to EOF.
#[test]
fn test_connect_send_recv_close() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();

        // Spawn the echo server (uses only Tokio/std types — no LocalSet required).
        tokio::task::spawn_local(run_echo_server(listener));

        let _globals = setup_globals();

        // TCP connect — returns a promise channel.
        let promise = cljrs_net::tcp::connect_to("127.0.0.1", port, 8, 8);

        // Await the promise channel (drive the LocalSet while waiting).
        let conn_val = chan_of(&promise).take().await;
        let conn = match &conn_val {
            Value::Map(m) => m.clone(),
            Value::Error(e) => panic!("connect failed: {}", e.get().message()),
            other => panic!("expected conn map, got {}", other.type_name()),
        };

        // Verify map keys exist.
        let in_val = conn_field(&conn, "in");
        let out_val = conn_field(&conn, "out");
        let _remote = conn_field(&conn, "remote-addr");
        let _local_addr = conn_field(&conn, "local-addr");
        let _resource = conn_field(&conn, "resource");

        let in_ch = chan_of(&in_val);
        let out_ch = chan_of(&out_val);

        // Put request bytes on :out.
        let request = b"hello from cljrs-net";
        let signed: Vec<i8> = request.iter().map(|&b| b as i8).collect();
        out_ch
            .put(Value::ByteArray(GcPtr::new(std::sync::Mutex::new(signed))))
            .await;

        // Close :out — triggers TCP half-close (FIN) after drain.
        out_ch.close();

        // Drain :in until EOF (Value::Nil from a closed channel).
        let mut response: Vec<u8> = Vec::new();
        loop {
            match in_ch.take().await {
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
            "echo server must echo back the same bytes"
        );
    });
}

/// Verify that a connection failure delivers `Value::Error` on the promise
/// channel rather than panicking.
#[test]
fn test_connect_failure_delivers_error() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        // Port 1 is privileged and almost certainly not listening.
        let promise = cljrs_net::tcp::connect_to("127.0.0.1", 1, 8, 8);

        let result = chan_of(&promise).take().await;
        assert!(
            matches!(result, Value::Error(_)),
            "expected Value::Error for refused connection, got {}",
            result.type_name()
        );
    });
}
