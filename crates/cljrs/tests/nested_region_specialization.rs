//! Regression test for #506: stage-4 cross-function region promotion can
//! clone a function (`middle`) that already contains a region-specialised
//! callee (`leaf__rg1`).  The clone's inner subfunctions are renamed, and the
//! `CallWithRegion` targets inside it must be renamed along with them —
//! otherwise the IR interpreter fails with
//! `CallWithRegion target leaf__rg1 not found in subfunctions`.

use std::process::Command;

fn eval(jit_threshold: &str, src: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", jit_threshold, "eval", src])
        .env("CLJRS_EAGER_LOWER", "1")
        .output()
        .expect("spawn cljrs")
}

const SRC: &str = r#"
(do
  (defn leaf [x] [x])
  (defn middle [x] [(count (leaf x))])
  (defn outer [x] (count (middle x)))
  (outer 7))"#;

#[test]
fn nested_region_specialization_resolves_call_targets() {
    // Threshold 0 runs the IR interpreter / JIT path immediately; a high
    // threshold keeps it in the IR interpreter.
    for threshold in ["0", "1000000"] {
        let output = eval(threshold, SRC);
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success(),
            "threshold {threshold}: cljrs exited with {:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
        assert_eq!(
            stdout.trim(),
            "1",
            "threshold {threshold}; stderr:\n{stderr}"
        );
    }
}
