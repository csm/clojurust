//! Regression tests for issue #517: sending an object array through an
//! isolate channel used to hold the array's lock while serializing its
//! elements, so a lazy element that reads its own array deadlocked, and a
//! self-containing array deadlocked on the re-lock.
//!
//! Each case runs `cljrs eval` in a subprocess with a timeout, so a
//! regression fails the test instead of hanging the suite.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const TIMEOUT: Duration = Duration::from_secs(60);

/// Evaluate `src` in a fresh `cljrs` process (tree-walker, no config) and
/// return its stdout, failing if it does not exit within [`TIMEOUT`].
fn eval(src: &str) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .env("CLJRS_NO_IR", "1")
        .arg("--no-config")
        .arg("eval")
        .arg(src)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn cljrs binary");
    // Drain both pipes on their own threads so a large stderr (a backtrace)
    // cannot fill the pipe and stall the child into a false timeout.
    let drain = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = pipe.read_to_string(&mut text);
            text
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait for cljrs") {
            break status;
        }
        if start.elapsed() > TIMEOUT {
            let _ = child.kill();
            let _ = child.wait();
            panic!("cljrs did not finish within {TIMEOUT:?} (deadlock?) for:\n{src}");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let stdout = stdout.join().unwrap();
    let stderr = stderr.join().unwrap();
    assert!(
        status.success(),
        "cljrs failed for:\n{src}\nstderr: {stderr}"
    );
    stdout
}

const PRELUDE: &str = "(require '[clojure.core.async :as a])";

fn program(body: &str) -> String {
    format!("(do {PRELUDE} (let [[tx rx] (a/isolate-chan)] {body}))")
}

#[test]
fn lazy_element_reading_its_own_array_does_not_deadlock() {
    let out = eval(&program(
        "(let [ar (object-array 1)]
           (aset ar 0 (lazy-seq (do (aget ar 0) nil)))
           (println :sent (a/isolate-put! tx ar))
           (println :received (vec (a/isolate-poll! rx))))",
    ));
    assert_eq!(out, ":sent true\n:received [nil]\n");
}

#[test]
fn lazy_element_mutating_its_own_array_sends_the_snapshot() {
    let out = eval(&program(
        "(let [ar (object-array [nil 1 2])]
           (aset ar 0 (lazy-seq (aset ar 1 :changed) (list (aget ar 2))))
           (a/isolate-put! tx ar)
           (println (pr-str (vec (a/isolate-poll! rx))) (aget ar 1)))",
    ));
    assert_eq!(out, "[(2) 1 2] :changed\n");
}

#[test]
fn direct_array_cycle_is_rejected() {
    let out = eval(&program(
        "(let [ar (object-array 1)]
           (aset ar 0 ar)
           (println (try (a/isolate-put! tx ar)
                         (catch Exception e (ex-message e)))))",
    ));
    assert!(out.contains("cyclic"), "unexpected output: {out}");
}

#[test]
fn indirect_array_cycle_is_rejected() {
    let out = eval(&program(
        "(let [a1 (object-array 1) a2 (object-array 1)]
           (aset a1 0 [a2])
           (aset a2 0 a1)
           (println (try (a/isolate-put! tx a1)
                         (catch Exception e (ex-message e)))))",
    ));
    assert!(out.contains("cyclic"), "unexpected output: {out}");
}

#[test]
fn acyclic_and_repeated_children_cross() {
    let out = eval(&program(
        "(let [child (object-array [1 2])
               ar (object-array [child child 3])]
           (a/isolate-put! tx ar)
           (let [copy (a/isolate-poll! rx)]
             (println (mapv #(if (number? %) % (vec %)) copy)
                      (identical? (aget copy 0) (aget copy 1)))))",
    ));
    assert_eq!(out, "[[1 2] [1 2] 3] false\n");
}
