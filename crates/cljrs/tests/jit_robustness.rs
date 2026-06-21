//! Phase 10.3 — the JIT worker survives functions it cannot compile: codegen
//! declines with a clean error (never a panic), and a worker panic from any
//! other cause is caught so one bad function cannot disable the JIT for the
//! rest of the session.
//!
//! Historical note: closure-bearing functions used to be the trigger — the JIT
//! compiled a single arity without declaring its closure subfunctions, so
//! `AllocClosure` codegen indexed a missing map key and panicked on the
//! background worker thread.  Closures now compile (subfunctions are declared
//! and compiled into the same module, as AOT does), but the decline-gracefully
//! behavior still guards everything codegen cannot express
//! (`lookup_user_func` returns `Err`, the worker `catch_unwind`s).  This test
//! mixes a closure-bearing function with a closure-free one and asserts:
//! results are correct, no panic surfaces, and the program completes normally.

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
fn closure_bearing_fn_runs_correctly_without_panicking() {
    // `make-adder` returns an inner closure → it now JIT-compiles (closure
    // subfunctions are compiled into the same module); whichever tier runs it
    // must produce correct results without killing the worker.  `vsum` is
    // closure-free → it may JIT-compile.  Both run hot so the JIT engages.
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

#[test]
fn string_join_char_elements_from_jit() {
    // `clojure.string/join` is never JIT-compiled (builtin-source namespace),
    // but a hot user function that calls it must still see correct results.
    // Characters must render as their string value ("80"), not reader syntax
    // ("\8\0"). Regression for issue #200.
    let src = r#"
        (require '[clojure.string :as s])
        (defn join-chars [chars sep]
          (s/join sep chars))
        (dotimes [_ 100]
          (join-chars [\8 \0] "")
          (join-chars [\8 \0] "-"))
        (println (join-chars [\8 \0] ""))
        (println (join-chars [\8 \0] "-"))
    "#;

    let run = run_jit(src);

    assert!(
        run.stdout.contains("80"),
        "expected \"80\" in stdout, got:\n{}",
        run.stdout
    );
    assert!(
        run.stdout.contains("8-0"),
        "expected \"8-0\" in stdout, got:\n{}",
        run.stdout
    );
    assert!(
        !run.stdout.contains("\\8"),
        "join produced reader syntax (\\8) instead of char value; stdout:\n{}",
        run.stdout
    );
}
