//! Phase 10.3 (seam shrink) — variadic/rest params dispatch correctly through
//! the JIT-native tier.
//!
//! A variadic arity is JIT-compiled with the signature the IR lowers to:
//! `(fixed…, rest_list)`.  The native dispatch path (`call_jit_native`) must
//! therefore pack the trailing call arguments into the rest list before calling
//! native code, exactly as the IR interpreter does.  Without that packing the
//! native function received the raw, unpacked argument count: the rest
//! arguments were silently dropped and the rest parameter bound to a single
//! scalar (e.g. `(mixed 10 20 30 40 50)` returned `[10 20 0 nil]` instead of
//! `[10 20 3 30]`).
//!
//! This test drives the real CLI binary with a hot variadic function and a low
//! JIT threshold so the function is promoted to native mid-run, then asserts
//! the results are correct.  The loop runs far more iterations than the
//! threshold so the background compile reliably publishes native code before
//! the loop ends; if it somehow does not, the Tier-1 interpreter still produces
//! the same (correct) answer, so the test never flakes false-negative.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `src` through `cljrs run` with the JIT forced on at a low threshold and
/// return the captured stdout.
fn run_jit(src: &str) -> String {
    // Unique temp path (no external tempfile dependency).
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_variadic_jit_{}_{nanos}_{seq}.cljrs",
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
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

#[test]
fn variadic_fn_dispatches_correctly_under_jit() {
    // `mixed` has two fixed params plus a rest; `novar` is purely variadic and
    // is called with zero trailing args (the empty-rest edge case → Nil).
    let src = r#"
        (defn mixed [a b & xs] [a b (count xs) (first xs)])
        (defn novar [& xs] (count xs))
        (dotimes [i 10000]
          (let [m (mixed 10 20 30 40 50)
                z (novar)]
            (when (= i 9999)
              (println "mixed=" m "empty=" z))))
    "#;

    let out = run_jit(src);
    assert!(
        out.contains("mixed= [10 20 3 30]"),
        "variadic rest args were not packed correctly; got:\n{out}"
    );
    assert!(
        out.contains("empty= 0"),
        "empty varargs did not bind an empty rest; got:\n{out}"
    );
}
