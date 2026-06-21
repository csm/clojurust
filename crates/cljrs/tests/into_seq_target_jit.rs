//! Regression test for issue #198: `into` with a lazy-seq or cons as the
//! target collection must not throw under the JIT tier.
//!
//! `into` is a native builtin — the JIT calls the same `builtin_into` as the
//! interpreter.  Running the function hot ensures the JIT compiles the
//! surrounding loop, triggering promotion, so any ABI-seam issue would surface.
//!
//! NOTE: Several pre-existing JIT region/value bugs fire when Cons values are
//! stored in multiple let-bindings or when `first` is called on a JIT-promoted
//! Cons.  Those bugs affect `conj` equally and are tracked separately.
//! Tests here use patterns confirmed to be free of those pre-existing issues:
//! calling `count` directly on the `into` result without storing it.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

fn run_jit(src: &str) -> (String, String) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_into_seq_jit_{}_{nanos}_{seq}.cljrs",
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
        "cljrs exited with {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn into_lazy_seq_target_count_correct_under_jit() {
    // Count is taken directly on the into result (no Cons let-binding) to
    // avoid pre-existing JIT region bugs that fire on multiple stored Cons bindings.
    let src = r#"
(defn check-into [i]
  (let [c1 (count (into (map inc [1 2]) [9]))
        c2 (count (into (seq [1 2]) [9]))
        c3 (count (into (map inc [1 2]) []))]
    (when (or (not= c1 3) (not= c2 3) (not= c3 2))
      (println "WRONG at" i "c1=" c1 "c2=" c2 "c3=" c3))
    (when (= i 9999)
      (println "ok c1=" c1 "c2=" c2 "c3=" c3))))

(dotimes [i 10000]
  (check-into i))
"#;

    let (stdout, _) = run_jit(src);
    assert!(
        !stdout.contains("WRONG at"),
        "`into` with lazy-seq/cons target returned wrong counts under JIT:\n{stdout}"
    );
    assert!(
        stdout.contains("ok c1= 3 c2= 3 c3= 2"),
        "expected confirmation line not found in:\n{stdout}"
    );
}
