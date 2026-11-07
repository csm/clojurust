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
    /// Responses read while collecting another request's, kept for their own
    /// `collect` (an interrupt's reply can arrive before the eval's "done").
    stashed: Vec<Msg>,
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
            stashed: Vec::new(),
        }
    }

    /// Send a request and collect every response for it up to and including
    /// the one whose status contains "done".
    fn request(&mut self, pairs: &[(&str, &str)]) -> Vec<Msg> {
        let id = self.send(pairs);
        self.collect(&id)
    }

    /// Send a request without waiting for its responses; returns its id.
    fn send(&mut self, pairs: &[(&str, &str)]) -> String {
        self.next_id += 1;
        let id = format!("t-{}", self.next_id);
        let mut dict = BTreeMap::new();
        dict.insert(b"id".to_vec(), Bencode::str(&id));
        for (k, v) in pairs {
            dict.insert(k.as_bytes().to_vec(), Bencode::str(v));
        }
        let bytes = bencode::encode_to_vec(&Bencode::Dict(dict));
        self.stream.write_all(&bytes).expect("write failed");
        id
    }

    /// Collect the responses to request `id` up to and including "done".
    fn collect(&mut self, id: &str) -> Vec<Msg> {
        let id_of = |dict: &Msg| {
            dict.get(b"id".as_slice())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        let mut responses = Vec::new();
        loop {
            let dict = match self
                .stashed
                .iter()
                .position(|m| id_of(m).as_deref() == Some(id))
            {
                Some(i) => self.stashed.remove(i),
                None => {
                    let msg = self.read_message();
                    msg.as_dict().expect("response is not a dict").clone()
                }
            };
            // Responses to other in-flight requests wait for their own collect.
            if id_of(&dict).as_deref() != Some(id) {
                self.stashed.push(dict);
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
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
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
    for op in [
        "clone",
        "eval",
        "completions",
        "lookup",
        "interrupt",
        "macroexpand",
        "analyze-last-stacktrace",
        "stacktrace",
    ] {
        assert!(ops.contains_key(op.as_bytes()), "missing op {op}");
    }

    // describe: advertises a Clojure language level as well as its own
    // version. Clients gate features on `versions.clojure`; without it CIDER
    // reports "Can't determine Clojure version" and turns part of itself off
    // against a server that otherwise works.
    let versions = resp[0]
        .get(b"versions".as_slice())
        .and_then(|v| v.as_dict())
        .expect("describe has versions");
    for key in ["cljrs", "nrepl", "clojure"] {
        assert!(
            versions.contains_key(key.as_bytes()),
            "describe omits versions.{key}"
        );
    }
    let clojure = versions
        .get(b"clojure".as_slice())
        .and_then(|v| v.as_dict())
        .expect("versions.clojure is a dict");
    for key in ["major", "minor", "incremental", "version-string"] {
        assert!(
            clojure.contains_key(key.as_bytes()),
            "versions.clojure omits {key}"
        );
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

    // interrupt stops an eval that is already running.
    interrupt_running_eval(&mut c, &session_a);

    // close removes the session.
    let resp = c.request(&[("op", "close"), ("session", &session_a)]);
    assert!(statuses(&resp[0]).contains(&"session-closed".to_string()));
    let resp = c.request(&[("op", "ls-sessions")]);
    let sessions: Vec<&str> = match resp[0].get(b"sessions".as_slice()) {
        Some(Bencode::List(items)) => items.iter().filter_map(|v| v.as_str()).collect(),
        _ => Vec::new(),
    };
    assert!(!sessions.contains(&session_a.as_str()));

    // Output is streamed while a form runs, not batched at its end.
    let resp = eval(
        &mut c,
        &session_b,
        "(do (println \"first\") (Thread/sleep 200) (println \"second\") :ok)",
    );
    let outs: Vec<&str> = resp
        .iter()
        .filter_map(|m| m.get(b"out".as_slice()).and_then(|v| v.as_str()))
        .collect();
    assert_eq!(outs, vec!["first\n", "second\n"], "responses: {resp:?}");
    assert_eq!(field(&resp, "value"), Some(":ok"));

    // macroexpand with each expander.
    let expand = |c: &mut Client, expander: &str, code: &str, display: &str| -> Vec<Msg> {
        c.request(&[
            ("op", "macroexpand"),
            ("session", &session_b),
            ("ns", "user"),
            ("code", code),
            ("expander", expander),
            ("display-namespaces", display),
        ])
    };
    for expander in ["macroexpand-1", "macroexpand", "macroexpand-all"] {
        let resp = expand(&mut c, expander, "(when x y)", "qualified");
        let expansion = field(&resp, "expansion").unwrap_or_else(|| panic!("{resp:?}"));
        assert!(expansion.starts_with("(if x"), "{expander}: {expansion}");
        assert!(statuses(resp.last().unwrap()).contains(&"done".to_string()));
    }
    let resp = expand(&mut c, "macroexpand", "(when x y)", "none");
    assert!(field(&resp, "expansion").is_some(), "{resp:?}");
    // display-namespaces: `none` drops every namespace; `tidy` drops only one
    // the bare name still resolves through, and otherwise prefers an alias.
    eval(
        &mut c,
        &session_b,
        "(ns tidy.probe (:require [clojure.string :as string]))",
    );
    eval(
        &mut c,
        &session_b,
        "(defmacro probe [xs] `(vector (clojure.string/join \",\" ~xs) (inc 1) (cljrs.unloaded/f 2)))",
    );
    let shown = |c: &mut Client, display: &str| -> String {
        let resp = c.request(&[
            ("op", "macroexpand"),
            ("session", &session_b),
            ("ns", "tidy.probe"),
            ("code", "(probe [1])"),
            ("expander", "macroexpand-1"),
            ("display-namespaces", display),
        ]);
        field(&resp, "expansion")
            .unwrap_or_else(|| panic!("{resp:?}"))
            .to_string()
    };
    assert_eq!(
        shown(&mut c, "qualified"),
        "(clojure.core/vector (clojure.string/join \",\" [1]) (clojure.core/inc 1) (cljrs.unloaded/f 2))"
    );
    assert_eq!(
        shown(&mut c, "tidy"),
        "(vector (string/join \",\" [1]) (inc 1) (cljrs.unloaded/f 2))"
    );
    assert_eq!(
        shown(&mut c, "none"),
        "(vector (join \",\" [1]) (inc 1) (f 2))"
    );
    eval(&mut c, &session_b, "(in-ns 'user)");

    let resp = expand(&mut c, "macroexpand", "(when", "qualified");
    assert!(
        statuses(resp.last().unwrap()).contains(&"macroexpand-error".to_string()),
        "{resp:?}"
    );

    // analyze-last-stacktrace: no-error on a fresh session, then the causes
    // of *e after a throw.
    let resp = c.request(&[("op", "clone")]);
    let session_c = field(&resp, "new-session")
        .expect("new-session")
        .to_string();
    let resp = c.request(&[("op", "analyze-last-stacktrace"), ("session", &session_c)]);
    assert!(
        statuses(&resp[0]).contains(&"no-error".to_string()),
        "{resp:?}"
    );
    eval(&mut c, &session_c, "(throw (ex-info \"boom\" {:a 1}))");
    for op in ["analyze-last-stacktrace", "stacktrace"] {
        let resp = c.request(&[("op", op), ("session", &session_c)]);
        assert_eq!(field(&resp, "message"), Some("boom"), "{resp:?}");
        assert!(
            field(&resp, "data").unwrap_or("").contains(":a"),
            "{resp:?}"
        );
        assert!(field(&resp, "class").is_some());
        assert!(matches!(
            resp[0].get(b"stacktrace".as_slice()),
            Some(Bencode::List(_))
        ));
        assert_eq!(statuses(resp.last().unwrap()), vec!["done".to_string()]);
    }

    // unknown op gets a clean error.
    let resp = c.request(&[("op", "frobnicate")]);
    assert!(statuses(&resp[0]).contains(&"unknown-op".to_string()));
}

/// A host whose interpreter thread also runs its own work drives the server
/// through a `Poller` instead of the blocking `serve`: nREPL jobs are handled
/// between the host's own steps, and neither starves the other.
#[test]
fn poller_interleaves_nrepl_jobs_with_host_work() {
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals).expect("start");
    let port = server.port();
    let mut poller = server.into_poller();

    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let client = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut c = Client::connect(port);
            let resp = c.request(&[("op", "clone")]);
            let session = field(&resp, "new-session")
                .expect("new-session")
                .to_string();
            // Output streams through the Poller path too.
            let resp = eval(
                &mut c,
                &session,
                "(do (println \"a\") (Thread/sleep 100) (println \"b\") (+ 40 2))",
            );
            let outs = resp
                .iter()
                .filter(|m| m.contains_key(b"out".as_slice()))
                .count();
            assert_eq!(outs, 2, "responses: {resp:?}");
            field(&resp, "value").map(str::to_string)
        }));
        let _ = done_tx.send(());
        result
    });

    let mut eval_form = cljrs_runtime::tiered::eval;
    let mut host_steps = 0u32;
    while done_rx.try_recv().is_err() {
        let status = poller.poll_timeout(&mut eval_form, Duration::from_millis(20));
        assert_eq!(status, cljrs_nrepl::PollStatus::Open);
        // The host's own work between polls.
        host_steps += 1;
    }
    let value = match client.join().expect("client thread") {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    };
    assert_eq!(value.as_deref(), Some("42"));
    assert!(host_steps > 0, "the host never got a turn");
    // Dropping the poller shuts the server down and joins its network thread.
    drop(poller);
}

/// Start a non-terminating eval in `session`, check that an interrupt for
/// another id is refused, interrupt it, and check the session still works.
fn interrupt_running_eval(c: &mut Client, session: &str) {
    let eval_id = c.send(&[
        ("op", "eval"),
        ("session", session),
        ("code", "(loop [] (recur))"),
    ]);
    // Let the eval get going; the interrupt must stop it mid-form.
    std::thread::sleep(Duration::from_millis(300));

    let resp = c.request(&[
        ("op", "interrupt"),
        ("session", session),
        ("interrupt-id", "no-such-id"),
    ]);
    assert_eq!(statuses(&resp[0]), ["interrupt-id-mismatch", "done"]);

    let started = std::time::Instant::now();
    let interrupt_id = c.send(&[
        ("op", "interrupt"),
        ("session", session),
        ("interrupt-id", &eval_id),
    ]);
    let eval_resp = c.collect(&eval_id);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "interrupt took {:?}",
        started.elapsed()
    );
    let eval_statuses: Vec<Vec<String>> = eval_resp.iter().map(statuses).collect();
    assert!(
        eval_statuses.contains(&vec!["interrupted".to_string()]),
        "{eval_statuses:?}"
    );
    assert_eq!(eval_statuses.last().unwrap(), &vec!["done".to_string()]);
    let resp = c.collect(&interrupt_id);
    assert_eq!(statuses(&resp[0]), ["done"]);

    // The session is still usable.
    let resp = eval(c, session, "(+ 1 2)");
    assert_eq!(field(&resp, "value"), Some("3"));

    // No stale flag: a long-running eval after the interrupt, and after an
    // eval that fails, still runs to completion.
    let counted = "(loop [n 0] (if (< n 200000) (recur (inc n)) n))";
    let resp = eval(c, session, counted);
    assert_eq!(field(&resp, "value"), Some("200000"));
    let resp = eval(c, session, "(throw (ex-info \"boom\" {}))");
    assert!(
        resp.iter()
            .any(|m| statuses(m).contains(&"eval-error".to_string()))
    );
    let resp = eval(c, session, counted);
    assert_eq!(field(&resp, "value"), Some("200000"));

    // Nothing runs any more.
    let resp = c.request(&[("op", "interrupt"), ("session", session)]);
    assert_eq!(statuses(&resp[0]), ["session-idle", "done"]);
}

/// The Poller path (a host driving the server from its own loop) can stop a
/// running eval too: the eval runs inside `poll`, so the interrupt must reach
/// it from the network thread while `poll` has not returned.
#[test]
fn poller_interrupts_running_eval() {
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals).expect("start");
    let port = server.port();
    let mut poller = server.into_poller();

    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let client = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut c = Client::connect(port);
            let resp = c.request(&[("op", "clone")]);
            let session = field(&resp, "new-session")
                .expect("new-session")
                .to_string();
            interrupt_running_eval(&mut c, &session);
        }));
        let _ = done_tx.send(());
        result
    });

    let mut eval_form = cljrs_runtime::tiered::eval;
    while done_rx.try_recv().is_err() {
        poller.poll_timeout(&mut eval_form, Duration::from_millis(20));
    }
    if let Err(panic) = client.join().expect("client thread") {
        std::panic::resume_unwind(panic);
    }
}

/// A host that owns its loop (a UI frame, say) and gives the isolate a short
/// slice per turn: `spawn_pending` starts each job on a fiber and returns,
/// and `drive_until` runs the fibers for the slice.
struct SpawningHost {
    poller: cljrs_nrepl::Poller,
    rt: tokio::runtime::Runtime,
    local: Option<tokio::task::LocalSet>,
    carrier: cljrs_gc::mutator::MutatorState,
    /// Longest single turn: spawn plus one slice.
    longest_turn: Duration,
}

impl SpawningHost {
    fn start() -> (SpawningHost, u16) {
        let globals = {
            let runtime = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
                .build()
                .expect("runtime");
            cljrs_stdlib::install(&runtime);
            runtime.into_globals()
        };
        let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals).expect("start");
        let port = server.port();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let host = SpawningHost {
            poller: server.into_poller(),
            rt,
            local: Some(tokio::task::LocalSet::new()),
            carrier: cljrs_gc::mutator::MutatorState::fresh(),
            longest_turn: Duration::ZERO,
        };
        (host, port)
    }

    /// Run turns until `done` fires.
    fn run_until(&mut self, done: &std::sync::mpsc::Receiver<()>) {
        let local = self.local.as_ref().unwrap();
        while done.try_recv().is_err() {
            let started = std::time::Instant::now();
            let status = self
                .poller
                .spawn_pending(local, cljrs_runtime::tiered::eval);
            assert_eq!(status, cljrs_nrepl::PollStatus::Open);
            cljrs_async::fiber::drive_until(
                &self.rt,
                local,
                &mut self.carrier,
                started + Duration::from_millis(5),
            );
            self.longest_turn = self.longest_turn.max(started.elapsed());
        }
    }
}

impl Drop for SpawningHost {
    fn drop(&mut self) {
        let local = self.local.take().unwrap();
        cljrs_async::fiber::drop_local_with_carrier(local, &mut self.carrier);
    }
}

fn spawn_client<T: Send + 'static>(
    port: u16,
    body: impl FnOnce(&mut Client) -> T + Send + 'static,
) -> (
    std::thread::JoinHandle<std::thread::Result<T>>,
    std::sync::mpsc::Receiver<()>,
) {
    let (done_tx, done_rx) = std::sync::mpsc::channel::<()>();
    let client = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            body(&mut Client::connect(port))
        }));
        let _ = done_tx.send(());
        result
    });
    (client, done_rx)
}

fn join_client<T>(client: std::thread::JoinHandle<std::thread::Result<T>>) -> T {
    match client.join().expect("client thread") {
        Ok(value) => value,
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Issue #447: an eval parked on a wait no longer holds the host. While one
/// session sleeps, the host keeps taking short turns and another session's
/// eval completes; the sleeping session's next request waits its turn.
#[test]
fn spawned_jobs_park_without_holding_the_host() {
    let (mut host, port) = SpawningHost::start();
    let (client, done) = spawn_client(port, |c| {
        let clone = |c: &mut Client| {
            let resp = c.request(&[("op", "clone")]);
            field(&resp, "new-session").unwrap().to_string()
        };
        let (a, b) = (clone(c), clone(c));
        let slow = c.send(&[
            ("op", "eval"),
            ("session", &a),
            (
                "code",
                "(do (println \"before\") (Thread/sleep 500) :slept)",
            ),
        ]);
        // Queued behind the sleep in the same session.
        let after = c.send(&[("op", "eval"), ("session", &a), ("code", "*1")]);
        // Another session runs while `a` is parked.
        let started = std::time::Instant::now();
        let resp = eval(c, &b, "(+ 1 2)");
        assert_eq!(field(&resp, "value"), Some("3"));
        assert!(
            started.elapsed() < Duration::from_millis(400),
            "session b waited {:?} behind a parked eval",
            started.elapsed()
        );
        let resp = c.collect(&slow);
        assert_eq!(field(&resp, "out"), Some("before\n"));
        assert_eq!(field(&resp, "value"), Some(":slept"));
        // `*1` is read after the sleep finished, so it sees its value.
        let resp = c.collect(&after);
        assert_eq!(field(&resp, "value"), Some(":slept"));
        // Tooling ops still answer on the spawned path.
        let resp = c.request(&[("op", "ls-sessions")]);
        assert_eq!(statuses(&resp[0]), ["done"]);
    });
    host.run_until(&done);
    join_client(client);
    assert!(
        host.longest_turn < Duration::from_millis(250),
        "a host turn took {:?}",
        host.longest_turn
    );
}

/// Interrupts reach spawned evals: one parked on a long wait, and one
/// running a non-terminating loop.
#[test]
fn spawned_jobs_can_be_interrupted() {
    let (mut host, port) = SpawningHost::start();
    let (client, done) = spawn_client(port, |c| {
        let resp = c.request(&[("op", "clone")]);
        let session = field(&resp, "new-session").unwrap().to_string();
        let parked = c.send(&[
            ("op", "eval"),
            ("session", &session),
            ("code", "(Thread/sleep 60000)"),
        ]);
        std::thread::sleep(Duration::from_millis(200));
        let started = std::time::Instant::now();
        let interrupt = c.send(&[
            ("op", "interrupt"),
            ("session", &session),
            ("interrupt-id", &parked),
        ]);
        let resp = c.collect(&parked);
        assert!(started.elapsed() < Duration::from_secs(5));
        let all: Vec<Vec<String>> = resp.iter().map(statuses).collect();
        assert!(all.contains(&vec!["interrupted".to_string()]), "{all:?}");
        assert_eq!(statuses(&c.collect(&interrupt)[0]), ["done"]);
        interrupt_running_eval(c, &session);
    });
    host.run_until(&done);
    join_client(client);
}

/// Once a client has seen an eval's `done`, the session is idle: an
/// `interrupt` sent immediately after must answer `session-idle`. The server
/// used to send `done` before removing the request from its pending table,
/// so a fast client could still see the finished eval as running.
#[test]
fn interrupt_right_after_done_finds_the_session_idle() {
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals).expect("start");
    let port = server.port();
    let shutdown = server.shutdown_handle();

    let client_thread = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(|| {
            let mut c = Client::connect(port);
            let resp = c.request(&[("op", "clone")]);
            let session = field(&resp, "new-session")
                .expect("new-session")
                .to_string();
            for i in 0..200 {
                let resp = eval(&mut c, &session, "(+ 1 2)");
                assert_eq!(field(&resp, "value"), Some("3"));
                let resp = c.request(&[("op", "interrupt"), ("session", &session)]);
                assert_eq!(
                    statuses(&resp[0]),
                    ["session-idle", "done"],
                    "iteration {i}"
                );
            }
        });
        shutdown.shutdown();
        if let Err(panic) = result {
            std::panic::resume_unwind(panic);
        }
    });

    server.serve().expect("serve");
    client_thread.join().expect("client scenario failed");
}
