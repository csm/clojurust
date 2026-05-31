//! Phase C integration tests: framing — line protocol and length-prefixed protocol.
//!
//! Done criterion from networking-plan.md Phase C:
//!   "a line-protocol and a length-prefixed-frame protocol both work end-to-end
//!   over a TCP connection, built purely from `frame` + transducers."

use std::sync::{Arc, Mutex};

use cljrs_async::channel::{chan_put, chan_ref, chan_take};
use cljrs_env::env::Env;
use cljrs_gc::GcPtr;
use cljrs_reader::Parser;
use cljrs_value::{Keyword, Value};

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

fn as_chan(v: &Value) -> GcPtr<cljrs_value::NativeObjectBox> {
    match v {
        Value::NativeObject(obj) => obj.clone(),
        other => panic!("expected channel, got {}", other.type_name()),
    }
}

fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

fn string_of(v: Value) -> String {
    match v {
        Value::Str(s) => s.get().clone(),
        other => panic!("expected string, got {}", other.type_name()),
    }
}

fn bytes_of(v: Value) -> Vec<u8> {
    match v {
        Value::ByteArray(arr) => arr.get().lock().unwrap().iter().map(|&b| b as u8).collect(),
        other => panic!("expected byte-array, got {}", other.type_name()),
    }
}

// ── Helper: connect client + server ──────────────────────────────────────────

/// Start a server on a free port, connect a client, and return
/// `(server_conn_map, client_conn_map)`.
async fn setup_connection() -> (cljrs_value::MapValue, cljrs_value::MapValue) {
    let server_val = cljrs_net::tcp::listen_on("127.0.0.1", 0, 8, 8, 8).expect("listen_on failed");
    let server_map = match server_val {
        Value::Map(m) => m,
        other => panic!("expected server map, got {}", other.type_name()),
    };

    let local_addr = match map_get(&server_map, "local-addr") {
        Value::Str(s) => s.get().clone(),
        other => panic!("expected str :local-addr, got {}", other.type_name()),
    };
    let port: u16 = local_addr.split(':').last().unwrap().parse().unwrap();

    // Start connecting — the promise resolves once the server accepts.
    let promise = cljrs_net::tcp::connect_to("127.0.0.1", port, 8, 8);
    let promise_chan = as_chan(&promise);

    // Accept the connection on the server side.
    let conns_ch = as_chan(&map_get(&server_map, "conns"));
    let server_conn = match chan_take(&conns_ch).await {
        Value::Map(m) => m,
        other => panic!("expected conn map from :conns, got {}", other.type_name()),
    };

    // Await the client side.
    let client_conn = match chan_take(&promise_chan).await {
        Value::Map(m) => m,
        Value::Error(e) => panic!("connect failed: {}", e.get().message()),
        other => panic!("expected conn map, got {}", other.type_name()),
    };

    (server_conn, client_conn)
}

// ── Phase C / line protocol ────────────────────────────────────────────────────

/// Phase C done criterion (lines): send line-encoded messages from client,
/// decode them on the server with `frame` + `lines`, verify the decoded strings.
#[test]
fn test_line_protocol_end_to_end() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let (server_conn, client_conn) = setup_connection().await;

        let server_in = as_chan(&map_get(&server_conn, "in"));
        let server_out = as_chan(&map_get(&server_conn, "out"));

        // Attach a lines framer to the server's :in channel.
        let lines_spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::Lines,
            out_buf: 8,
        };
        let msg_chan = cljrs_net::frame::frame_channel(server_in, lines_spec, 8);

        let client_out = as_chan(&map_get(&client_conn, "out"));
        let client_in = as_chan(&map_get(&client_conn, "in"));

        // Send three lines from the client (each UTF-8 + '\n').
        let messages = vec!["hello", "world", "from cljrs-net"];
        for msg in &messages {
            chan_put(&client_out, cljrs_net::frame::encode_line(msg)).await;
        }
        // Half-close the client write side → server :in sees EOF.
        chan_ref(client_out.get()).close();

        // Server: read three decoded strings from the framed channel.
        let mut received: Vec<String> = Vec::new();
        loop {
            match chan_take(&msg_chan).await {
                Value::Nil => break,
                v @ Value::Str(_) => received.push(string_of(v)),
                Value::Error(e) => panic!("frame error: {}", e.get().message()),
                other => panic!("unexpected value from framer: {}", other.type_name()),
            }
        }
        assert_eq!(received, messages, "decoded lines must match sent messages");

        // Echo them back line-encoded so we can verify the wire round-trip.
        for msg in &received {
            chan_put(&server_out, cljrs_net::frame::encode_line(msg)).await;
        }
        chan_ref(server_out.get()).close();

        // Client: drain raw :in and verify the response bytes equal re-encoded lines.
        let mut raw_response: Vec<u8> = Vec::new();
        loop {
            match chan_take(&client_in).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => raw_response.extend_from_slice(&bytes_of(v)),
                Value::Error(e) => panic!("client read error: {}", e.get().message()),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        let expected: String = messages.iter().map(|m| format!("{m}\n")).collect();
        assert_eq!(
            raw_response,
            expected.as_bytes(),
            "echoed response must match line-encoded originals"
        );
    });
}

// ── Phase C / length-prefixed protocol ───────────────────────────────────────

/// Phase C done criterion (length-prefixed): send length-prefixed messages from
/// client, decode them on the server with `frame` + `length-prefixed`, verify
/// round-trip.
#[test]
fn test_length_prefixed_protocol_end_to_end() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let (server_conn, client_conn) = setup_connection().await;

        let server_in = as_chan(&map_get(&server_conn, "in"));
        let server_out = as_chan(&map_get(&server_conn, "out"));

        // Attach a 4-byte big-endian length-prefixed framer to the server's :in.
        let lp_spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::LengthPrefixed {
                prefix_len: 4,
                big_endian: true,
            },
            out_buf: 8,
        };
        let msg_chan = cljrs_net::frame::frame_channel(server_in, lp_spec, 8);

        let client_out = as_chan(&map_get(&client_conn, "out"));
        let client_in = as_chan(&map_get(&client_conn, "in"));

        // Send three messages (each prefixed with a 4-byte big-endian length).
        let payloads: Vec<&[u8]> = vec![b"ping", b"hello world", b"length-prefixed test"];
        for payload in &payloads {
            let framed = cljrs_net::frame::encode_length_prefixed(payload, 4, true);
            chan_put(&client_out, framed).await;
        }
        chan_ref(client_out.get()).close();

        // Server: read three decoded byte-arrays.
        let mut received: Vec<Vec<u8>> = Vec::new();
        loop {
            match chan_take(&msg_chan).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => received.push(bytes_of(v)),
                Value::Error(e) => panic!("frame error: {}", e.get().message()),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        let expected_payloads: Vec<Vec<u8>> = payloads.iter().map(|p| p.to_vec()).collect();
        assert_eq!(
            received, expected_payloads,
            "decoded payloads must match sent payloads"
        );

        // Echo back length-prefixed and verify the wire format on the client.
        for payload in &received {
            chan_put(
                &server_out,
                cljrs_net::frame::encode_length_prefixed(payload, 4, true),
            )
            .await;
        }
        chan_ref(server_out.get()).close();

        let mut raw_response: Vec<u8> = Vec::new();
        loop {
            match chan_take(&client_in).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => raw_response.extend_from_slice(&bytes_of(v)),
                Value::Error(e) => panic!("client read error: {}", e.get().message()),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }

        let mut expected_wire: Vec<u8> = Vec::new();
        for payload in &payloads {
            let len = payload.len() as u32;
            expected_wire.extend_from_slice(&len.to_be_bytes());
            expected_wire.extend_from_slice(payload);
        }
        assert_eq!(
            raw_response, expected_wire,
            "echoed response must match length-prefixed wire format"
        );
    });
}

// ── Framer unit tests (no TCP) ────────────────────────────────────────────────

/// Lines framer: partial chunks must be reassembled correctly.
#[test]
fn test_lines_framer_partial_chunks() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let in_chan = cljrs_async::channel::make_chan(8);
        let spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::Lines,
            out_buf: 8,
        };
        let out_chan = cljrs_net::frame::frame_channel(in_chan.clone(), spec, 8);

        // Split "hello\nworld\n" across three chunks.
        chan_put(&in_chan, bytes_value(b"hel")).await;
        chan_put(&in_chan, bytes_value(b"lo\nwo")).await;
        chan_put(&in_chan, bytes_value(b"rld\n")).await;
        chan_ref(in_chan.get()).close();

        let mut lines: Vec<String> = Vec::new();
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => break,
                v @ Value::Str(_) => lines.push(string_of(v)),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        assert_eq!(lines, vec!["hello", "world"]);
    });
}

/// Length-prefixed framer: multiple frames coalesced in a single TCP chunk.
#[test]
fn test_length_prefixed_framer_coalesced_chunks() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let in_chan = cljrs_async::channel::make_chan(8);
        let spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::LengthPrefixed {
                prefix_len: 4,
                big_endian: true,
            },
            out_buf: 8,
        };
        let out_chan = cljrs_net::frame::frame_channel(in_chan.clone(), spec, 8);

        // Two frames in one chunk.
        let mut wire: Vec<u8> = Vec::new();
        for payload in [b"abc" as &[u8], b"de"] {
            let len = payload.len() as u32;
            wire.extend_from_slice(&len.to_be_bytes());
            wire.extend_from_slice(payload);
        }
        chan_put(&in_chan, bytes_value(&wire)).await;
        chan_ref(in_chan.get()).close();

        let mut frames: Vec<Vec<u8>> = Vec::new();
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => frames.push(bytes_of(v)),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        assert_eq!(frames, vec![b"abc".to_vec(), b"de".to_vec()]);
    });
}

/// By-delimiter framer: split on null byte, partial chunks.
#[test]
fn test_delimiter_framer_null_byte() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let in_chan = cljrs_async::channel::make_chan(8);
        let spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::Delimiter(0u8),
            out_buf: 8,
        };
        let out_chan = cljrs_net::frame::frame_channel(in_chan.clone(), spec, 8);

        // Two frames separated by \0, delivered in one chunk.
        chan_put(&in_chan, bytes_value(b"frame1\x00frame2\x00")).await;
        chan_ref(in_chan.get()).close();

        let mut frames: Vec<Vec<u8>> = Vec::new();
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => frames.push(bytes_of(v)),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        assert_eq!(frames, vec![b"frame1".to_vec(), b"frame2".to_vec()]);
    });
}

/// Lines framer: last line without trailing newline is emitted at EOF.
#[test]
fn test_lines_framer_no_trailing_newline() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let _globals = setup_globals();

        let in_chan = cljrs_async::channel::make_chan(8);
        let spec = cljrs_net::frame::FramerSpec {
            kind: cljrs_net::frame::FramerKind::Lines,
            out_buf: 8,
        };
        let out_chan = cljrs_net::frame::frame_channel(in_chan.clone(), spec, 8);

        // "first\nsecond" — second line has no trailing newline.
        chan_put(&in_chan, bytes_value(b"first\nsecond")).await;
        chan_ref(in_chan.get()).close();

        let mut lines: Vec<String> = Vec::new();
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => break,
                v @ Value::Str(_) => lines.push(string_of(v)),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }
        assert_eq!(lines, vec!["first", "second"]);
    });
}

// ── Clojure pipe-fn as framer ─────────────────────────────────────────────────

/// `frame` accepts a Clojure `(fn [in-chan] -> out-chan)` as the second argument.
/// The function is called with `in-chan` and its return value (the output channel)
/// is returned by `frame`. This lets framers be written entirely in Clojure
/// without any Rust involvement.
#[test]
fn test_clojure_pipe_fn_as_framer() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();

    local.block_on(&rt, async {
        let globals = setup_globals();

        // Env in the frame namespace so `frame`, `chan`, `go`, etc. resolve.
        let mut env = Env::new(globals.clone(), "clojure.rust.net.frame");

        // Build an input channel and bind it by name for the Clojure expression.
        let in_chan = cljrs_async::channel::make_chan(8);
        env.push_frame();
        env.bind(Arc::from("test-in"), Value::NativeObject(in_chan.clone()));

        // A Clojure identity pipe-fn: reads from ch, puts each value on out, closes at EOF.
        // Written entirely in Clojure — no Rust framer involved.
        let src = r#"
            (let [pipe (fn [ch]
                         (let [out (clojure.core.async/chan 8)]
                           (clojure.core.async/go
                             (loop []
                               (let [v (await (clojure.core.async/take! ch))]
                                 (if (nil? v)
                                   (clojure.core.async/close! out)
                                   (do
                                     (await (clojure.core.async/put! out v))
                                     (recur))))))
                           out))]
              (frame test-in pipe))
        "#;

        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let forms = parser.parse_all().expect("parse error");
        let out_chan_val = cljrs_interp::eval::eval(forms.last().unwrap(), &mut env)
            .expect("eval error: frame with Clojure pipe-fn failed");

        let out_chan = as_chan(&out_chan_val);

        // Feed two byte-arrays into the input channel.
        let payload1 = bytes_value(b"hello");
        let payload2 = bytes_value(b"world");
        chan_put(&in_chan, payload1).await;
        chan_put(&in_chan, payload2).await;
        chan_ref(in_chan.get()).close();

        // Drain the output channel and verify the same byte-arrays come out.
        let mut received: Vec<Vec<u8>> = Vec::new();
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => break,
                v @ Value::ByteArray(_) => received.push(bytes_of(v)),
                Value::Error(e) => panic!("pipe-fn error: {}", e.get().message()),
                other => panic!("unexpected: {}", other.type_name()),
            }
        }

        assert_eq!(
            received,
            vec![b"hello".to_vec(), b"world".to_vec()],
            "Clojure pipe-fn must relay all values and close the output channel at EOF"
        );
    });
}
