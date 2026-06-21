//! JIT-path regression tests for issue #209.
//!
//! `::keyword` must resolve to the namespace where the code was written
//! (the call site), not the macro's definition namespace, even after the
//! JIT promotes a hot function to native code.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_jit(src: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_auto_kw_jit_{}_{nanos}_{seq}.cljrs",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", "50", "run"])
        .arg(&path)
        .env("CLJRS_EAGER_LOWER", "1")
        .output()
        .expect("spawn cljrs");

    let _ = std::fs::remove_file(&path);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstderr:\n{}\nstdout:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout),
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

// ── Regression: issue #209 ────────────────────────────────────────────────────

#[test]
fn auto_kw_if_not_correct_through_jit() {
    // Verify that ::kw resolves to the call-site namespace even after the
    // function is promoted to native code.  Runs 10 000 iterations so the
    // JIT threshold (50) is crossed and native code executes per-iteration.
    let src = r#"
(ns my.app)

(defn check [v]
  (if-not (= v ::error) :WRONG :right))

(def results
  (loop [i 0 acc []]
    (if (= i 10000)
      acc
      (recur (inc i) (conj acc (check ::error))))))

(println (every? #(= % :right) results))
"#;
    let out = run_jit(src);
    assert_eq!(
        out.trim(),
        "true",
        "every iteration must return :right through if-not after JIT"
    );
}

#[test]
fn auto_kw_when_not_correct_through_jit() {
    let src = r#"
(ns my.app)

(defn sentinel? [v]
  (when-not (= v ::sentinel) :not-sentinel))

(def results
  (loop [i 0 acc []]
    (if (= i 10000)
      acc
      (recur (inc i) (conj acc (sentinel? ::sentinel))))))

(println (every? nil? results))
"#;
    let out = run_jit(src);
    assert_eq!(
        out.trim(),
        "true",
        "::kw through when-not must return nil (every? nil?) after JIT"
    );
}

#[test]
fn auto_kw_resolves_to_correct_ns_through_jit() {
    let src = r#"
(ns acme.core)

(defn validate [x]
  (let [sentinel ::missing]
    (if-not (= x sentinel) :found :missing)))

(def r1s
  (loop [i 0 acc []]
    (if (= i 10000)
      acc
      (recur (inc i) (conj acc (validate ::missing))))))

(def r2s
  (loop [i 0 acc []]
    (if (= i 10000)
      acc
      (recur (inc i) (conj acc (validate ::other))))))

(println (and (every? #(= % :missing) r1s)
              (every? #(= % :found) r2s)))
"#;
    let out = run_jit(src);
    assert_eq!(out.trim(), "true");
}
