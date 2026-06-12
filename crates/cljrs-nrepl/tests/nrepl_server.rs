//! End-to-end test: start the server against a full stdlib environment and
//! exercise the protocol with a minimal bencode client over TCP.
//!
//! A single #[test] runs the whole scenario: the interpreter (and its
//! GlobalEnv) lives on this test's thread, which runs `serve()` while a
//! client thread drives the socket.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use cljrs_nrepl::bencode::{self, Bencode};

type Msg = BTreeMap<Vec<u8>, Bencode>;

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    next_id: u64,
}

impl Client {
    fn connect(port: u16) -> Client {
        let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect failed");
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        Client {
            stream,
            buf: Vec::new(),
            next_id: 0,
        }
    }

    /// Send a request and collect every response for it up to and including
    /// the one whose status contains "done".
    fn request(&mut self, pairs: &[(&str, &str)]) -> Vec<Msg> {
        self.next_id += 1;
        let id = format!("t-{}", self.next_id);
        let mut dict = BTreeMap::new();
        dict.insert(b"id".to_vec(), Bencode::str(&id));
        for (k, v) in pairs {
            dict.insert(k.as_bytes().to_vec(), Bencode::str(v));
        }
        let bytes = bencode::encode_to_vec(&Bencode::Dict(dict));
        self.stream.write_all(&bytes).expect("write failed");

        let mut responses = Vec::new();
        loop {
            let msg = self.read_message();
            let dict = msg.as_dict().expect("response is not a dict").clone();
            // Only collect responses to this request (defensive; the server
            // sends nothing unsolicited).
            if dict.get(b"id".as_slice()).and_then(|v| v.as_str()) != Some(&id) {
                continue;
            }
            let done = statuses(&dict).iter().any(|s| s == "done");
            responses.push(dict);
            if done {
                return responses;
            }
        }
    }

    fn read_message(&mut self) -> Bencode {
        let mut chunk = [0u8; 4096];
        loop {
            if let Some((msg, consumed)) = bencode::decode(&self.buf).expect("bad bencode") {
                self.buf.drain(..consumed);
                return msg;
            }
            let n = self.stream.read(&mut chunk).expect("read failed");
            assert!(n > 0, "server closed connection mid-response");
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

fn statuses(msg: &Msg) -> Vec<String> {
    match msg.get(b"status".as_slice()) {
        Some(Bencode::List(items)) => items
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

/// First occurrence of a string field across a request's responses.
fn field<'a>(responses: &'a [Msg], key: &str) -> Option<&'a str> {
    responses
        .iter()
        .find_map(|m| m.get(key.as_bytes()).and_then(|v| v.as_str()))
}

fn eval(client: &mut Client, session: &str, code: &str) -> Vec<Msg> {
    client.request(&[("op", "eval"), ("session", session), ("code", code)])
}

#[test]
fn nrepl_end_to_end() {
    let globals = cljrs_stdlib::standard_env();
    let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals).expect("start");
    let port = server.port();
    let shutdown = server.shutdown_handle();

    let client_thread = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| client_scenario(port));
        // Always release the serve() loop, even when assertions failed.
        shutdown.shutdown();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    });

    server.serve().expect("serve");
    client_thread.join().expect("client scenario failed");
}

fn client_scenario(port: u16) {
    let mut c = Client::connect(port);

    // describe: advertises the ops we implement.
    let resp = c.request(&[("op", "describe")]);
    let ops = resp[0]
        .get(b"ops".as_slice())
        .and_then(|v| v.as_dict())
        .expect("describe has ops");
    for op in ["clone", "eval", "completions", "lookup", "interrupt"] {
        assert!(ops.contains_key(op.as_bytes()), "missing op {op}");
    }

    // clone: two independent sessions.
    let resp = c.request(&[("op", "clone")]);
    let session_a = field(&resp, "new-session")
        .expect("new-session")
        .to_string();
    let resp = c.request(&[("op", "clone")]);
    let session_b = field(&resp, "new-session")
        .expect("new-session")
        .to_string();
    assert_ne!(session_a, session_b);

    // eval: value + ns + done.
    let resp = eval(&mut c, &session_a, "(+ 1 2)");
    assert_eq!(field(&resp, "value"), Some("3"));
    assert_eq!(field(&resp, "ns"), Some("user"));
    assert!(statuses(resp.last().unwrap()).contains(&"done".to_string()));

    // *1 holds the previous result.
    let resp = eval(&mut c, &session_a, "*1");
    assert_eq!(field(&resp, "value"), Some("3"));

    // println output is captured and streamed as "out".
    let resp = eval(&mut c, &session_a, "(println \"hello nrepl\")");
    assert_eq!(field(&resp, "out"), Some("hello nrepl\n"));
    assert_eq!(field(&resp, "value"), Some("nil"));

    // Errors produce err + eval-error, then done (no hang), and set *e.
    let resp = eval(&mut c, &session_a, "(throw (ex-info \"boom\" {}))");
    assert!(field(&resp, "err").unwrap_or_default().contains("boom"));
    assert!(
        resp.iter()
            .any(|m| statuses(m).contains(&"eval-error".to_string())),
        "expected an eval-error status"
    );
    let resp = eval(&mut c, &session_a, "(nil? *e)");
    assert_eq!(field(&resp, "value"), Some("false"));

    // Sessions keep independent namespaces.
    let resp = eval(&mut c, &session_a, "(ns foo.bar)");
    assert_eq!(field(&resp, "ns"), Some("foo.bar"));
    let resp = eval(&mut c, &session_b, "(+ 1 1)");
    assert_eq!(field(&resp, "ns"), Some("user"));

    // Multiple forms in one message stream multiple values; *1 updates
    // between forms of the same message.
    let resp = eval(&mut c, &session_b, "(* 2 3) (inc *1)");
    let values: Vec<&str> = resp
        .iter()
        .filter_map(|m| m.get(b"value".as_slice()).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(values, ["6", "7"]);

    // load-file evaluates the payload in the session.
    let resp = c.request(&[
        ("op", "load-file"),
        ("session", &session_b),
        ("file", "(def loaded-var 99) loaded-var"),
        ("file-name", "test.cljrs"),
    ]);
    let values: Vec<&str> = resp
        .iter()
        .filter_map(|m| m.get(b"value".as_slice()).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(values.last(), Some(&"99"));

    // completions: core vars show up for a short prefix.
    let resp = c.request(&[
        ("op", "completions"),
        ("session", &session_b),
        ("prefix", "ma"),
    ]);
    let completions = resp[0]
        .get(b"completions".as_slice())
        .map(|v| match v {
            Bencode::List(items) => items
                .iter()
                .filter_map(|item| {
                    item.as_dict()
                        .and_then(|d| d.get(b"candidate".as_slice()))
                        .and_then(|c| c.as_str())
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    assert!(
        completions.contains(&"map"),
        "completions for \"ma\" missing map: {completions:?}"
    );

    // lookup: var info for clojure.core/map.
    let resp = c.request(&[("op", "lookup"), ("session", &session_b), ("sym", "map")]);
    let info = resp[0]
        .get(b"info".as_slice())
        .and_then(|v| v.as_dict())
        .expect("lookup has info");
    assert_eq!(
        info.get(b"name".as_slice()).and_then(|v| v.as_str()),
        Some("map")
    );
    assert_eq!(
        info.get(b"ns".as_slice()).and_then(|v| v.as_str()),
        Some("clojure.core")
    );

    // ls-sessions lists both cloned sessions.
    let resp = c.request(&[("op", "ls-sessions")]);
    let sessions: Vec<&str> = match resp[0].get(b"sessions".as_slice()) {
        Some(Bencode::List(items)) => items.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };
    assert!(sessions.contains(&session_a.as_str()));
    assert!(sessions.contains(&session_b.as_str()));

    // interrupt on an idle session reports session-idle.
    let resp = c.request(&[("op", "interrupt"), ("session", &session_a)]);
    assert!(statuses(&resp[0]).contains(&"session-idle".to_string()));

    // close removes the session.
    let resp = c.request(&[("op", "close"), ("session", &session_a)]);
    assert!(statuses(&resp[0]).contains(&"session-closed".to_string()));
    let resp = c.request(&[("op", "ls-sessions")]);
    let sessions: Vec<&str> = match resp[0].get(b"sessions".as_slice()) {
        Some(Bencode::List(items)) => items.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };
    assert!(!sessions.contains(&session_a.as_str()));

    // unknown op gets a clean error.
    let resp = c.request(&[("op", "frobnicate")]);
    assert!(statuses(&resp[0]).contains(&"unknown-op".to_string()));
}
