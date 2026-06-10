//! Phase 10.3 — correctness of rt_abi bridges at the JIT-native dispatch seam.
//!
//! Regression tests for two silent-nil bugs in JIT-native dispatch
//! (`call_jit_native`, cljrs-eval/src/apply.rs):
//!
//! 1. **Missing eval context.**  Native code resolves globals
//!    (`rt_load_global`) and calls function values (`rt_call`, the HOF
//!    bridges) through rt_abi, which dispatches via the thread-local eval
//!    context.  Tier-1 (`execute_ir`) and the AOT preamble push one; the
//!    JIT-native path did not — so once a function was promoted to native
//!    code, every higher-order call (`reduce`/`map`/`filter` with `+`,
//!    `inc`, a lambda…), every `(f x)` call of a function-valued argument,
//!    and every closure built by `rt_make_fn` then invoked via `rt_call`
//!    failed inside `callback::invoke` and was swallowed into nil.
//!
//! 2. **Swallowed throws.**  `(throw …)` inside native code stashes the
//!    thrown value in a thread-local and returns the nil sentinel; only an
//!    `rt_try` *inside* compiled code checked it.  An uncaught native throw
//!    therefore surfaced as a nil return value (and the stale slot could
//!    misfire a later `rt_try` on the same thread).  The seam now takes the
//!    pending exception and re-raises it as `EvalError::Thrown`.
//!
//! Each script runs the hot function far past the JIT threshold so the
//! background compile reliably publishes native code mid-run, and asserts
//! per-iteration correctness; if promotion somehow doesn't land, Tier-1
//! produces the same (correct) answers, so the tests never flake
//! false-negative.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `src` through `cljrs run` with the JIT forced on at a low threshold and
/// return the captured stdout.
fn run_jit(src: &str) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_jit_seam_{}_{nanos}_{seq}.cljrs",
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
fn hofs_with_function_values_survive_jit_promotion() {
    let src = r#"
        (defn sum [v] (reduce + v))
        (defn doubles [v] (mapv (fn [x] (* 2 x)) v))
        (defn evens [v] (filterv even? v))
        (defn call-it [f x] (f x))
        (dotimes [i 10000]
          (let [a (sum [1 2 3])
                b (doubles [1 2 3])
                c (evens [1 2 3 4])
                d (call-it inc 41)
                e (call-it (fn [y] (* y 10)) 7)]
            (when (or (not= a 6) (not= b [2 4 6]) (not= c [2 4])
                      (not= d 42) (not= e 70))
              (println "WRONG at" i ":" a b c d e))
            (when (= i 9999)
              (println "results:" a b c d e))))
    "#;

    let out = run_jit(src);
    assert!(
        !out.contains("WRONG at"),
        "HOF/fn-value calls returned wrong results under JIT; got:\n{out}"
    );
    assert!(
        out.contains("results: 6 [2 4 6] [2 4] 42 70"),
        "final results wrong; got:\n{out}"
    );
}

#[test]
fn uncaught_native_throw_propagates_to_interpreter_caller() {
    // `boom` is JIT-eligible and throws on the hot path; the try/catch lives
    // in the *interpreter* caller, so the thrown value must cross the
    // JIT-native dispatch seam as an error, not a nil return.
    let src = r#"
        (defn boom [x] (if (> x 5) (throw (ex-info "boom" {:x x})) x))
        (dotimes [i 10000]
          (let [e (try (boom 10) (catch Exception ex (ex-message ex)))]
            (when (not= e "boom")
              (println "WRONG at" i "got" e))
            (when (= i 9999)
              (println "caught:" e))))
    "#;

    let out = run_jit(src);
    assert!(
        !out.contains("WRONG at"),
        "throw from JIT-native frame was swallowed; got:\n{out}"
    );
    assert!(out.contains("caught: boom"), "got:\n{out}");
}

#[test]
fn closures_compile_and_run_correctly_under_jit() {
    // Closure-bearing functions now JIT-compile (subfunctions are declared
    // and compiled into the same module, as AOT does).  Cover the three
    // shapes that matter:
    //  - a closure created *and invoked* inside the same native frame
    //    (the formerly-broken rt_make_fn → rt_call round trip),
    //  - a closure escaping the native frame, invoked later by the
    //    interpreter (its module epoch is pinned against unloading),
    //  - a closure whose body needs the eval context (calls a global fn).
    let src = r#"
        (defn call-twice [n]
          (let [f (fn [x] (+ x n))]
            (+ (f 1) (f 2))))
        (defn make-adder [n] (fn [x] (+ x n)))
        (defn tagger [tag] (fn [x] (str tag ":" x)))
        (dotimes [i 10000]
          (let [a (call-twice 10)
                b ((make-adder 5) 1)
                c ((tagger "t") 9)]
            (when (or (not= a 23) (not= b 6) (not= c "t:9"))
              (println "WRONG at" i ":" a b c))
            (when (= i 9999)
              (println "closures:" a b c))))
    "#;

    let out = run_jit(src);
    assert!(
        !out.contains("WRONG at"),
        "closure results wrong under JIT; got:\n{out}"
    );
    assert!(out.contains("closures: 23 6 t:9"), "got:\n{out}");
}
