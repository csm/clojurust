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
        "{}",
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

#[test]
fn nrepl_root_waits_preserve_output_and_session_bindings() {
    use cljrs_nrepl::bencode::{self, Bencode};
    use std::collections::BTreeMap;
    use std::io::Read;
    struct Server(std::process::Child);
    impl Drop for Server {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let mut server = Server(
        command()
            .args(["nrepl", "--port", "0"])
            .current_dir(dir.path())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    let port: u16 = loop {
        if let Ok(port) = std::fs::read_to_string(dir.path().join(".nrepl-port")) {
            break port.trim().parse().unwrap();
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
