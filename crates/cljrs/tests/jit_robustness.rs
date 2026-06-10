//! Phase 10.3 — the JIT declines functions it cannot yet compile correctly
//! (e.g. those containing inner closures) *gracefully*, rather than panicking.
//!
//! The JIT compiles a single arity at a time and does not declare a function's
//! closure subfunctions, so `AllocClosure` codegen cannot resolve them.  This
//! used to index a missing map key and panic — and because the panic happened
//! on the background JIT worker thread, the *first* hot closure-bearing function
//! killed the worker and silently disabled the JIT for the rest of the session.
//!
//! Codegen now returns a clean error for an undeclared subfunction, so such
//! functions fall back to the (correct) interpreter and the worker keeps
//! compiling everything else.  This test exercises a script that mixes a
//! closure-bearing function with a closure-free one and asserts: results are
//! correct, no panic surfaces, and the program completes normally.

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
        "cljrs_jit_robustness_{}_{nanos}_{seq}.cljrs",
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
    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

#[test]
fn closure_bearing_fn_declines_without_panicking() {
    // `make-adder` returns an inner closure → the JIT must decline it and fall
    // back to the interpreter.  `vsum` is closure-free → it may JIT-compile.
    // Both run hot so the JIT engages.
    let src = r#"
        (defn make-adder [n] (fn [x] (+ x n)))
        (defn vsum [a b & xs] [a b (count xs)])
        (dotimes [i 30000]
          ((make-adder 5) 1)
          (vsum 1 2 3 4)
          (when (= i 29999)
            (println "adder=" ((make-adder 5) 1))
            (println "vsum=" (vsum 1 2 3 4))))
    "#;

    let run = run_jit(src);

    // Results must be correct regardless of which tier executed them.
    assert!(
        run.stdout.contains("adder= 6"),
        "closure result wrong; stdout:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("vsum= [1 2 2]"),
        "variadic result wrong; stdout:\n{}",
        run.stdout
    );

    // The closure decline must be graceful: no worker panic backtrace.
    assert!(
        !run.stderr.contains("panicked"),
        "JIT worker panicked instead of declining gracefully; stderr:\n{}",
        run.stderr
    );
}
