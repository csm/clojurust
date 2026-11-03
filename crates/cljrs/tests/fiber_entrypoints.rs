#![cfg(feature = "async")]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cljrs"))
}

fn finish(mut child: std::process::Child) -> String {
    let deadline = Instant::now() + Duration::from_secs(30);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let output = child.wait_with_output().unwrap();
            panic!(
                "entry point hung: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "entry point exited with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn eval_run_and_repl_can_deref_nested_futures() {
    let expr = "@(future @(future (sleep 1) 42))";
    let output = finish(
        command()
            .args(["eval", expr])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert!(output.contains("42"), "{output}");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.cljrs");
    std::fs::write(
        &path,
        format!("(println {expr}) (defn -main [] (println {expr}))"),
    )
    .unwrap();
    let output = finish(
        command()
            .arg("run")
            .arg(&path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert_eq!(output.lines().filter(|l| *l == "42").count(), 2, "{output}");
    let mut repl = command()
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    writeln!(
        repl.stdin.take().unwrap(),
        "(ns fiber.repl)\n(def f (future (sleep 1) 42))\n@f\n{expr}"
    )
    .unwrap();
    let output = finish(repl);
    assert!(output.matches("42").count() >= 2, "{output}");
    assert!(output.contains("fiber.repl"), "{output}");
}

#[test]
fn clojure_test_bodies_run_in_a_root_fiber() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("fiber_test.cljrs"),
        r#"
      (ns fiber-test (:require [clojure.test :refer [deftest is]]
                               [clojure.core.async :as a]))
      (deftest blocking-test
        (is (= 42 @(future @(future 42))))
        (is (= :done (a/<!! (a/io-thread (sleep 1) :done)))))
    "#,
    )
    .unwrap();
    let output = finish(
        command()
            .args(["test", "--src-path"])
            .arg(dir.path())
            .arg("fiber-test")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert!(output.contains("2 passed"), "{output}");
}

struct NreplServer(std::process::Child);
impl Drop for NreplServer {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Start `cljrs nrepl` on an ephemeral port in `dir`, returning the port.
fn start_nrepl(dir: &std::path::Path) -> (NreplServer, u16) {
    use std::io::Read;
    let mut server = NreplServer(
        command()
            .args(["nrepl", "--port", "0"])
            .current_dir(dir)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let port: u16 = loop {
        // The file can be seen before its contents are written.
        if let Some(port) = std::fs::read_to_string(dir.join(".nrepl-port"))
            .ok()
            .and_then(|port| port.trim().parse().ok())
        {
            break port;
        }
        if let Some(status) = server.0.try_wait().unwrap() {
            let mut error = String::new();
            server
                .0
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut error)
                .unwrap();
            panic!("nREPL exited: {status}: {error}");
        }
        assert!(Instant::now() < deadline, "nREPL did not start");
        std::thread::sleep(Duration::from_millis(10));
    };
    (server, port)
}

#[test]
fn nrepl_root_waits_preserve_output_and_session_bindings() {
    use cljrs_nrepl::bencode::{self, Bencode};
    use std::collections::BTreeMap;
    use std::io::Read;
    let dir = tempfile::tempdir().unwrap();
    let (_server, port) = start_nrepl(dir.path());
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut request = BTreeMap::new();
    request.insert(b"op".to_vec(), Bencode::str("eval"));
    request.insert(b"id".to_vec(), Bencode::str("fiber"));
    request.insert(
        b"code".to_vec(),
        Bencode::str("41 (do @(future @(future (sleep 1) 42)) (println :root-output) (inc *1))"),
    );
    stream
        .write_all(&bencode::encode_to_vec(&Bencode::Dict(request)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut values = Vec::new();
    let mut output = String::new();
    loop {
        if let Some((msg, consumed)) = bencode::decode(&bytes).unwrap() {
            bytes.drain(..consumed);
            let dict = msg.as_dict().unwrap();
            assert!(!dict.contains_key(b"ex".as_slice()), "{dict:?}");
            if let Some(value) = dict.get(b"value".as_slice()).and_then(Bencode::as_str) {
                values.push(value.to_owned());
            }
            if let Some(text) = dict.get(b"out".as_slice()).and_then(Bencode::as_str) {
                output.push_str(text);
            }
            if let Some(Bencode::List(status)) = dict.get(b"status".as_slice())
                && status.iter().any(|s| s.as_str() == Some("done"))
            {
                break;
            }
        } else {
            let mut chunk = [0; 4096];
            let n = stream.read(&mut chunk).unwrap();
            assert!(n > 0, "nREPL disconnected");
            bytes.extend_from_slice(&chunk[..n]);
        }
    }
    assert_eq!(values, ["41", "42"]);
    assert_eq!(output, ":root-output\n");
}

/// An `interrupt` stops an eval parked on a wait that would never end, or
/// end only much later, and the session keeps working (issue #446).
#[test]
fn nrepl_interrupt_stops_parked_evals() {
    use cljrs_nrepl::bencode::{self, Bencode};
    use std::collections::BTreeMap;
    use std::io::Read;

    struct Client {
        stream: std::net::TcpStream,
        bytes: Vec<u8>,
        stash: Vec<Bencode>,
        session: Option<String>,
    }
    impl Client {
        fn send(&mut self, fields: &[(&str, &str)]) {
            let mut request = BTreeMap::new();
            if let Some(session) = &self.session {
                request.insert(b"session".to_vec(), Bencode::str(session.as_str()));
            }
            for (key, value) in fields {
                request.insert(key.as_bytes().to_vec(), Bencode::str(*value));
            }
            self.stream
                .write_all(&bencode::encode_to_vec(&Bencode::Dict(request)))
                .unwrap();
        }
        /// Read replies to `id` up to its "done", returning their statuses
        /// and values (or new session id).
        fn collect(&mut self, id: &str) -> (Vec<String>, Vec<String>) {
            let (mut statuses, mut values) = (Vec::new(), Vec::new());
            loop {
                let id_of = |msg: &Bencode| {
                    msg.as_dict()
                        .and_then(|d| d.get(b"id".as_slice()))
                        .and_then(Bencode::as_str)
                        .map(str::to_owned)
                };
                let stashed = self
                    .stash
                    .iter()
                    .position(|m| id_of(m).as_deref() == Some(id));
                let msg = if let Some(index) = stashed {
                    Some(self.stash.remove(index))
                } else if let Some((msg, consumed)) = bencode::decode(&self.bytes).unwrap() {
                    self.bytes.drain(..consumed);
                    Some(msg)
                } else {
                    None
                };
                if let Some(msg) = msg {
                    if id_of(&msg).as_deref() != Some(id) {
                        // A reply to another request, e.g. an interrupt
                        // answered before the eval it stopped.
                        self.stash.push(msg);
                        continue;
                    }
                    let dict = msg.as_dict().unwrap();
                    for key in [b"value".as_slice(), b"new-session".as_slice()] {
                        if let Some(value) = dict.get(key).and_then(Bencode::as_str) {
                            values.push(value.to_owned());
                        }
                    }
                    if let Some(Bencode::List(status)) = dict.get(b"status".as_slice()) {
                        statuses
                            .extend(status.iter().filter_map(|s| s.as_str().map(str::to_owned)));
                        if statuses.iter().any(|s| s == "done") {
                            return (statuses, values);
                        }
                    }
                } else {
                    let mut chunk = [0; 4096];
                    let n = self.stream.read(&mut chunk).unwrap();
                    assert!(n > 0, "nREPL disconnected");
                    self.bytes.extend_from_slice(&chunk[..n]);
                }
            }
        }
    }

    let dir = tempfile::tempdir().unwrap();
    let (_server, port) = start_nrepl(dir.path());
    let stream = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .unwrap();
    let mut client = Client {
        stream,
        bytes: Vec::new(),
        stash: Vec::new(),
        session: None,
    };
    client.send(&[("op", "clone"), ("id", "clone")]);
    client.session = client.collect("clone").1.pop();
    assert!(client.session.is_some());
    client.send(&[
        ("op", "eval"),
        ("id", "require"),
        ("code", "(require '[clojure.core.async :as a])"),
    ]);
    let (statuses, _) = client.collect("require");
    assert!(!statuses.iter().any(|s| s == "eval-error"), "{statuses:?}");

    let parked = [
        "(deref (promise))",
        "(Thread/sleep 20000)",
        "(a/<!! (a/chan))",
        "@(future (Thread/sleep 20000))",
        "(try (deref (promise)) (catch Exception _ :caught))",
    ];
    for (n, code) in parked.iter().enumerate() {
        let id = format!("parked-{n}");
        client.send(&[("op", "eval"), ("id", &id), ("code", code)]);
        std::thread::sleep(Duration::from_millis(300));
        let started = Instant::now();
        client.send(&[
            ("op", "interrupt"),
            ("id", &format!("interrupt-{n}")),
            ("interrupt-id", &id),
        ]);
        let (statuses, values) = client.collect(&id);
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{code}: interrupt took {:?}",
            started.elapsed()
        );
        assert!(
            statuses.iter().any(|s| s == "interrupted"),
            "{code}: {statuses:?} {values:?}"
        );
        assert!(values.is_empty(), "{code}: {values:?}");
        client.collect(&format!("interrupt-{n}"));

        // The interpreter is free again and the flag is not stale.
        let check = format!("check-{n}");
        client.send(&[
            ("op", "eval"),
            ("id", &check),
            ("code", "(do (Thread/sleep 10) @(future (+ 1 2)))"),
        ]);
        assert_eq!(client.collect(&check).1, ["3"], "after {code}");
    }
}

#[test]
fn thread_body_errors_are_reported_and_close_the_result_channel() {
    let output = command().args(["eval", "(require '[clojure.core.async :as a]) (a/<!! (a/io-thread (throw (ex-info \"review-boom\" {}))))"])
        .output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("review-boom"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn executor_panic_preserves_the_callers_environment() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            cljrs::session::with_async_driver(|| {
                let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
                let mut env = cljrs_runtime::env::env::Env::new(runtime.globals().clone(), "user");
                env.push_frame();
                env.bind("saved".into(), cljrs_value::Value::Long(42));
                let forms = cljrs_reader::Parser::new("saved".into(), "<unwind>".into())
                    .parse_all()
                    .unwrap();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .build()
                    .unwrap();
                let panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = rt.block_on(async { cljrs::session::eval_form(&forms[0], &mut env) });
                }));
                assert!(panic.is_err(), "nested executor drive must panic");
                assert_eq!(
                    cljrs::session::eval_form(&forms[0], &mut env).unwrap(),
                    cljrs_value::Value::Long(42)
                );
            });
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn ir_build_reads_defined_functions_outside_evaluation() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("core.ir.bin");
    finish(
        command()
            .args(["ir", "build", "--ns", "clojure.core", "-o"])
            .arg(&output)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    assert!(std::fs::metadata(output).unwrap().len() > 0);
}
