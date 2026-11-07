//! Admission budgets and resource limits (CLJ-ASYNC-003): accepted jobs,
//! per-session jobs, per-call job budgets, request frame size, and queued
//! output are all bounded, and `interrupt` keeps working under overload.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use cljrs_nrepl::bencode::{self, Bencode};
use cljrs_nrepl::{Config, Limits, PollBatch, PollStatus, Poller};

type Msg = BTreeMap<Vec<u8>, Bencode>;

struct Client {
    stream: TcpStream,
    buf: Vec<u8>,
    next_id: u64,
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

    fn request(&mut self, pairs: &[(&str, &str)]) -> Vec<Msg> {
        let id = self.send(pairs);
        self.collect(&id)
    }

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
                None => self
                    .read_message()
                    .as_dict()
                    .expect("response is not a dict")
                    .clone(),
            };
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

    /// Read until the server closes the connection, discarding data.
    fn expect_closed(&mut self) {
        let mut chunk = [0u8; 64 * 1024];
        loop {
            match self.stream.read(&mut chunk) {
                Ok(0) => return,
                Ok(_) => continue,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => return,
                Err(e) => panic!("expected the server to close the connection: {e}"),
            }
        }
    }

    fn clone_session(&mut self) -> String {
        let resp = self.request(&[("op", "clone")]);
        field(&resp, "new-session").unwrap().to_string()
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

fn all_statuses(responses: &[Msg]) -> Vec<String> {
    responses.iter().flat_map(statuses).collect()
}

fn field<'a>(responses: &'a [Msg], key: &str) -> Option<&'a str> {
    responses
        .iter()
        .find_map(|m| m.get(key.as_bytes()).and_then(|v| v.as_str()))
}

fn eval(client: &mut Client, session: &str, code: &str) -> Vec<Msg> {
    client.request(&[("op", "eval"), ("session", session), ("code", code)])
}

fn start(limits: Limits) -> (Poller, u16) {
    let globals = {
        let runtime = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime");
        cljrs_stdlib::install(&runtime);
        runtime.into_globals()
    };
    let config = Config {
        limits,
        ..Config::default()
    };
    let server = cljrs_nrepl::start(config, globals).expect("start");
    let port = server.port();
    (server.into_poller(), port)
}

/// Run `body` on a client thread; returns its handle and a done signal.
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

/// Wait (on the host thread, without polling) until the server has admitted
/// `n` unfinished jobs.
fn wait_for_pending(poller: &Poller, n: usize) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while poller.pending_jobs() < n {
        assert!(Instant::now() < deadline, "jobs were never admitted");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// While the host is not polling, accepted jobs pile up only to the server
/// limit; the rest are refused at once with `overloaded`, and `interrupt`
/// is still answered. Once polled, the server accepts work again.
#[test]
fn server_refuses_jobs_beyond_the_pending_limit() {
    let limits = Limits {
        max_pending_jobs: 2,
        ..Limits::default()
    };
    let (mut poller, port) = start(limits);
    assert_eq!(poller.limits(), limits);
    let (go_tx, go_rx) = std::sync::mpsc::channel::<()>();
    let (client, done) = spawn_client(port, move |c| {
        let a = c.send(&[("op", "eval"), ("code", "(+ 1 1)")]);
        let b = c.send(&[("op", "eval"), ("code", "(+ 2 2)")]);
        // Refused immediately even though the host has not polled.
        let resp = c.request(&[("op", "eval"), ("code", "(+ 3 3)")]);
        let st = all_statuses(&resp);
        assert!(st.contains(&"overloaded".to_string()), "{st:?}");
        assert!(st.contains(&"done".to_string()));
        assert!(field(&resp, "err").unwrap().contains("overloaded"));
        // Tooling ops count too.
        let resp = c.request(&[("op", "ls-sessions")]);
        assert!(all_statuses(&resp).contains(&"overloaded".to_string()));
        // Interrupt and describe bypass admission.
        let resp = c.request(&[("op", "interrupt"), ("session", "nope")]);
        assert_eq!(statuses(&resp[0]), ["session-idle", "done"]);
        let resp = c.request(&[("op", "describe")]);
        assert_eq!(statuses(&resp[0]), ["done"]);

        go_tx.send(()).unwrap();
        assert_eq!(field(&c.collect(&a), "value"), Some("2"));
        assert_eq!(field(&c.collect(&b), "value"), Some("4"));
        // Capacity is back.
        let resp = c.request(&[("op", "eval"), ("code", "(+ 4 4)")]);
        assert_eq!(field(&resp, "value"), Some("8"));
    });
    wait_for_pending(&poller, 2);
    go_rx.recv_timeout(Duration::from_secs(30)).expect("client");
    assert_eq!(poller.pending_jobs(), 2);
    let mut eval_form = cljrs_runtime::tiered::eval;
    while done.try_recv().is_err() {
        poller.poll_timeout(&mut eval_form, Duration::from_millis(10));
    }
    join_client(client);
    assert_eq!(poller.pending_jobs(), 0);
}

/// A budgeted poll takes at most its budget and says when it stopped early.
#[test]
fn poll_budget_reports_exhaustion() {
    let (mut poller, port) = start(Limits::default());
    let (client, done) = spawn_client(port, |c| {
        let ids: Vec<String> = (1..=3)
            .map(|n| c.send(&[("op", "eval"), ("code", &format!("(* {n} 10)"))]))
            .collect();
        ids.iter()
            .map(|id| field(&c.collect(id), "value").unwrap().to_string())
            .collect::<Vec<_>>()
    });
    wait_for_pending(&poller, 3);
    // The last ticket is taken just before the job is queued.
    std::thread::sleep(Duration::from_millis(50));
    let mut eval_form = cljrs_runtime::tiered::eval;
    assert_eq!(
        poller.poll_budget(&mut eval_form, 2),
        PollBatch {
            status: PollStatus::Open,
            jobs: 2,
            budget_exhausted: true
        }
    );
    assert_eq!(poller.pending_jobs(), 1);
    assert_eq!(
        poller.poll_timeout_budget(&mut eval_form, Duration::from_millis(10), 0),
        PollBatch {
            status: PollStatus::Open,
            jobs: 0,
            budget_exhausted: true
        }
    );
    assert_eq!(
        poller.poll_budget(&mut eval_form, 2),
        PollBatch {
            status: PollStatus::Open,
            jobs: 1,
            budget_exhausted: false
        }
    );
    done.recv_timeout(Duration::from_secs(30)).expect("client");
    assert_eq!(join_client(client), ["10", "20", "30"]);
}

/// Spawned evaluations parked on a wait still hold their session's slots;
/// another session is unaffected; interrupting the parked eval frees the slot.
#[test]
fn parked_spawned_jobs_count_against_the_session_limit() {
    let limits = Limits {
        max_session_jobs: 1,
        ..Limits::default()
    };
    let (mut poller, port) = start(limits);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    let mut carrier = cljrs_gc::mutator::MutatorState::fresh();
    let (client, done) = spawn_client(port, |c| {
        let a = c.clone_session();
        let b = c.clone_session();
        let parked = c.send(&[
            ("op", "eval"),
            ("session", &a),
            ("code", "(Thread/sleep 60000)"),
        ]);
        std::thread::sleep(Duration::from_millis(200));
        let resp = eval(c, &a, "(+ 1 2)");
        let st = all_statuses(&resp);
        assert!(st.contains(&"overloaded".to_string()), "{st:?}");
        assert!(field(&resp, "err").unwrap().contains("session"));
        // Another session still runs.
        let resp = eval(c, &b, "(+ 1 2)");
        assert_eq!(field(&resp, "value"), Some("3"));
        // The interrupt reaches the parked eval and frees its slot.
        let resp = c.request(&[
            ("op", "interrupt"),
            ("session", &a),
            ("interrupt-id", &parked),
        ]);
        assert_eq!(statuses(&resp[0]), ["done"]);
        assert!(all_statuses(&c.collect(&parked)).contains(&"interrupted".to_string()));
        let resp = eval(c, &a, "(+ 2 3)");
        assert_eq!(field(&resp, "value"), Some("5"));
    });
    while done.try_recv().is_err() {
        let started = Instant::now();
        let batch = poller.spawn_pending_budget(&local, cljrs_runtime::tiered::eval, 4);
        assert_eq!(batch.status, PollStatus::Open);
        cljrs_async::fiber::drive_until(
            &rt,
            &local,
            &mut carrier,
            started + Duration::from_millis(5),
        );
    }
    join_client(client);
    cljrs_async::fiber::drop_local_with_carrier(local, &mut carrier);
    assert_eq!(poller.pending_jobs(), 0);
}

/// A request frame over the limit, complete or not, closes the connection;
/// the server keeps serving other connections.
#[test]
fn oversized_requests_close_the_connection() {
    let limits = Limits {
        max_request_bytes: 4096,
        ..Limits::default()
    };
    let (mut poller, port) = start(limits);
    let (client, done) = spawn_client(port, move |c| {
        // An incomplete frame that keeps growing.
        let mut frame = b"d4:code1000000:".to_vec();
        frame.extend(std::iter::repeat_n(b' ', 16 * 1024));
        let _ = c.stream.write_all(&frame);
        c.expect_closed();

        // A complete frame over the limit.
        let mut c = Client::connect(port);
        let code = format!("(count \"{}\")", "x".repeat(8 * 1024));
        let _ = c.send(&[("op", "eval"), ("code", &code)]);
        c.expect_closed();

        // Within the limit, all is well.
        let mut c = Client::connect(port);
        let code = format!("(count \"{}\")", "x".repeat(1024));
        let resp = c.request(&[("op", "eval"), ("code", &code)]);
        assert_eq!(field(&resp, "value"), Some("1024"));
    });
    let mut eval_form = cljrs_runtime::tiered::eval;
    while done.try_recv().is_err() {
        poller.poll_timeout(&mut eval_form, Duration::from_millis(10));
    }
    join_client(client);
}

/// A client that stops reading is disconnected once its queued replies pass
/// the output limit, instead of buffering the eval's output without bound.
#[test]
fn unread_output_beyond_the_limit_closes_the_connection() {
    let limits = Limits {
        max_output_bytes: 64 * 1024,
        ..Limits::default()
    };
    let (mut poller, port) = start(limits);
    let (client, done) = spawn_client(port, move |c| {
        let line = "y".repeat(1000);
        let id = c.send(&[
            ("op", "eval"),
            (
                "code",
                &format!("(dotimes [_ 20000] (println \"{line}\")) :printed"),
            ),
        ]);
        // Do not read while the eval writes ~20 MB of output.
        std::thread::sleep(Duration::from_secs(3));
        let _ = id;
        c.expect_closed();

        // The server is healthy for other clients.
        let mut c = Client::connect(port);
        let resp = c.request(&[("op", "eval"), ("code", "(+ 20 22)")]);
        assert_eq!(field(&resp, "value"), Some("42"));
    });
    let mut eval_form = cljrs_runtime::tiered::eval;
    while done.try_recv().is_err() {
        poller.poll_timeout(&mut eval_form, Duration::from_millis(10));
    }
    join_client(client);
    assert_eq!(poller.pending_jobs(), 0);
}
