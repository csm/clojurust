//! Regression tests for #501: a region scope opened for one non-escaping
//! allocation (here a set literal) must not swallow other values produced
//! while it is open.
//!
//! Compiled code allocates collections opportunistically into the innermost
//! open region (`box_coll_val`), so before the fix the default map returned
//! by `(get options :missing {:a 1})`, the caller's map found by that `get`,
//! and a fresh vector returned by a compiled callee all landed in the
//! `#{:x}` region and were freed at its `RegionEnd` while still live — the
//! function returned nil, or a dangling pointer that crashed the process.
//!
//! The JIT shares the region-scope lowering with AOT, so a low JIT threshold
//! exercises it without the cost of building a native binary.

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
    let script = std::env::temp_dir().join(format!(
        "cljrs_region_scope_escape_{}_{nanos}_{seq}.cljrs",
        std::process::id()
    ));
    std::fs::write(&script, src).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", "50", "run"])
        .arg(&script)
        .env("CLJRS_EAGER_LOWER", "1")
        .output()
        .expect("spawn cljrs");
    let _ = std::fs::remove_file(&script);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[test]
fn values_produced_inside_a_set_literal_region_survive_it() {
    let src = r#"
        (defn callee [] (contains? #{"a"} "a") nil)
        (defn kept-map [options]
          (let [value (get options :missing {:a 1})]
            (contains? #{:x} :x)
            (callee)
            value))
        (defn kept-vector [options]
          (let [value (:missing options [1 2])]
            (contains? #{:x} :x)
            (callee)
            value))
        (defn between [options]
          (let [s #{:x}
                value (get options :missing {:a 1})]
            (contains? s :x)
            (callee)
            value))
        (defn fresh [n] (if (pos? n) [n n] {:n n}))
        (defn via-callee [n]
          (let [s #{:x}
                value (fresh n)]
            (contains? s :x)
            (callee)
            value))
        (def bad (atom 0))
        (defn check [expected actual]
          (when-not (= expected actual) (swap! bad inc)))
        (dotimes [_ 2000]
          (check {:a 1} (kept-map {}))
          (check {:b 2} (kept-map {:missing {:b 2}}))
          (check [1 2] (kept-vector {}))
          (check {:a 1} (between {}))
          (check [3 3] (via-callee 3))
          (check {:n 0} (via-callee 0)))
        (println "bad" @bad)
        (println (pr-str (kept-map {})) (pr-str (kept-vector {})) (pr-str (via-callee 3)))
    "#;
    assert_eq!(run_jit(src), "bad 0\n{:a 1} [1 2] [3 3]\n");
}
