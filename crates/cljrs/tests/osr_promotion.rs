//! Phase 10.4 milestone — a *single-call* hot `loop`/`recur` promotes to
//! native code mid-run via OSR (on-stack replacement).
//!
//! Invocation-count tiering can never promote this shape: the function is
//! called exactly once and never returns to re-dispatch.  The loop back-edge
//! counter must trip instead, the JIT worker compiles an OSR-entry variant in
//! the background, and the Tier-1 interpreter transfers its register file into
//! the native frame at a loop-header entry — all observable through
//! `-X debug:jit` and, above all, through a correct final result.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Run {
    stdout: String,
    stderr: String,
}

fn run_jit(src: &str) -> Run {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_osr_promotion_{}_{nanos}_{seq}.cljrs",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", "100", "-X", "debug:jit", "run"])
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
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn single_call_hot_loop_promotes_mid_run() {
    // One call, two million back-edges.  The OSR threshold follows
    // --jit-threshold (100), so the loop gets hot almost immediately and has
    // ample iterations left for the background compile to land mid-run.
    let src = r#"
        (defn sum-below [n]
          (loop [i 0 acc 0]
            (if (< i n)
              (recur (+ i 1) (+ acc i))
              acc)))
        (println "sum=" (sum-below 2000000))
    "#;

    let run = run_jit(src);

    // Correctness first: 0+1+…+1999999 = 1999999000000, whichever tier
    // finished the loop.
    assert!(
        run.stdout.contains("sum= 1999999000000"),
        "wrong loop result; stdout:\n{}\nstderr:\n{}",
        run.stdout,
        run.stderr
    );

    // The back-edge counter must have requested OSR compilation…
    assert!(
        run.stderr.contains("osr enqueue"),
        "loop never tripped the back-edge counter; stderr:\n{}",
        run.stderr
    );
    // …the worker must have compiled and published the OSR entry…
    assert!(
        run.stderr.contains("osr compiled"),
        "OSR entry was not compiled; stderr:\n{}",
        run.stderr
    );
    // …and the interpreter must have transferred into native code mid-run.
    assert!(
        run.stderr.contains("osr entering native code"),
        "interpreter never transferred into the OSR frame; stderr:\n{}",
        run.stderr
    );
}
