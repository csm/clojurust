//! Regression test for issue #198: `into` with a lazy-seq or cons as the
//! target collection must not throw under the JIT tier.
//!
//! `into` is a native builtin — the JIT calls the same `builtin_into` as the
//! interpreter.  Running the function hot ensures the JIT compiles the
//! surrounding loop, triggering promotion, so any ABI-seam issue would surface.

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
fn into_lazy_seq_target_correct_under_jit() {
    // Run the `into` forms inside a hot loop so the surrounding defn gets
    // JIT-compiled.  Verify correct results at every iteration.
    let src = r#"
(defn check-into [i]
  (let [a (vec (into (map inc [1 2]) [9]))
        b (vec (into (seq [1 2]) [9]))
        c (count (into (map inc [1 2]) []))]
    (when (or (not= a [9 2 3]) (not= b [9 1 2]) (not= c 2))
      (println "WRONG at" i "a=" a "b=" b "c=" c))
    (when (= i 9999)
      (println "ok a=" a "b=" b "c=" c))))

(dotimes [i 10000]
  (check-into i))
"#;

    let (stdout, _stderr) = run_jit(src);
    assert!(
        !stdout.contains("WRONG at"),
        "`into` with lazy-seq/cons target returned wrong results under JIT:\n{stdout}"
    );
    assert!(
        stdout.contains("ok a= [9 2 3] b= [9 1 2] c= 2"),
        "expected confirmation line not found in:\n{stdout}"
    );
}
