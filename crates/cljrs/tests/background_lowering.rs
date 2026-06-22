//! Phase 10.7 — background IR lowering (warm tier).
//!
//! Unlike the other JIT suites these tests deliberately do **not** set
//! `CLJRS_EAGER_LOWER`: functions must start at Tier 0 (tree-walk), get
//! counted, cross the warm threshold (`CLJRS_IR_THRESHOLD`), be lowered to
//! optimized IR on the background `cljrs-ir-lower` worker, and only then
//! dispatch through the Tier-1 IR interpreter — proceeding to JIT compilation
//! if invocations continue.
//!
//! Each script asserts per-iteration correctness (printing `WRONG …` on any
//! mismatch), so the tests stay green whether or not a particular promotion
//! lands mid-run — what they catch is any tier producing different results.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `src` through `cljrs run` with background lowering forced hot
/// (`CLJRS_IR_THRESHOLD=10`) and IR debug logging on.  Returns
/// `(stdout, stderr)`.
fn run_warm(src: &str, extra_args: &[&str], extra_env: &[(&str, &str)]) -> (String, String) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_bg_lower_{}_{nanos}_{seq}.cljrs",
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write script");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cljrs"));
    cmd.args(["-X", "debug:ir"])
        .args(extra_args)
        .arg("run")
        .arg(&path)
        .env("CLJRS_IR_THRESHOLD", "10")
        .env_remove("CLJRS_EAGER_LOWER");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("spawn cljrs");

    let _ = std::fs::remove_file(&path);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        String::from_utf8(output.stderr).expect("utf8 stderr"),
    )
}

/// Tier-up correctness: a hot function runs far past both the warm threshold
/// (10) and a low JIT threshold (50), so a single run exercises
/// tree-walk → background-lowered IR → JIT-native, asserting every iteration.
#[test]
fn tiering_up_mid_run_keeps_results_correct() {
    let src = r#"
        (defn poly [x] (+ (* 3 x x) (* 2 x) 7))
        (defn run-all [n]
          (loop [i 0 bad 0]
            (if (< i n)
              (let [got (poly i)
                    want (+ (* 3 i i) (* 2 i) 7)]
                (when (not= got want) (println "WRONG at" i got want))
                (recur (+ i 1) bad))
              bad)))
        (run-all 20000)
        (println "final:" (poly 100))
    "#;
    let (stdout, stderr) = run_warm(src, &["--jit-threshold", "50"], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "tier mismatch:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("final: 30207"), "stdout:\n{stdout}");
}

/// The debug log must show the worker actually publishing IR — i.e. the
/// function reached Tier 1 via the background path, not eager lowering.
#[test]
fn background_lowering_publishes_ir() {
    let src = r#"
        (defn warm-me [x] (* x x))
        (loop [i 0 acc 0]
          (if (< i 200)
            (recur (+ i 1) (+ acc (warm-me i)))
            (println "sum:" acc)))
    "#;
    let (stdout, stderr) = run_warm(src, &[], &[]);
    assert!(stdout.contains("sum: 2646700"), "stdout:\n{stdout}");
    assert!(
        stderr.contains("background lower published"),
        "no background publish in stderr:\n{stderr}"
    );
}

/// Rebinding a defn while its caller is warm/hot must be reflected
/// immediately: the dependent's IR is invalidated and re-lowered in the
/// background, and every interim call (tree-walk fallback) already resolves
/// the new binding.
#[test]
fn rebind_during_warm_window_takes_effect_immediately() {
    let src = r#"
        (defn helper [x] (+ x 1))
        (defn caller [x] (helper x))
        (loop [i 0]
          (when (< i 500)
            (when (not= (caller i) (+ i 1)) (println "WRONG-BEFORE at" i))
            (recur (+ i 1))))
        (defn helper [x] (+ x 100))
        (loop [i 0]
          (when (< i 500)
            (when (not= (caller i) (+ i 100)) (println "WRONG-AFTER at" i (caller i)))
            (recur (+ i 1))))
        (println "rebound:" (caller 1))
    "#;
    let (stdout, stderr) = run_warm(src, &["--jit-threshold", "50"], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "stale results after rebind:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("rebound: 101"), "stdout:\n{stdout}");
}

/// Background lowering is owned by cljrs-eval and must work with the JIT
/// disabled: functions still tier up from tree-walk to the IR interpreter.
#[test]
fn background_lowering_works_without_jit() {
    let src = r#"
        (defn no-jit-fn [x] (- (* 2 x) 3))
        (loop [i 0 acc 0]
          (if (< i 200)
            (do
              (when (not= (no-jit-fn i) (- (* 2 i) 3)) (println "WRONG at" i))
              (recur (+ i 1) (+ acc (no-jit-fn i))))
            (println "sum:" acc)))
    "#;
    let (stdout, stderr) = run_warm(src, &[], &[("CLJRS_NO_JIT", "1")]);
    assert!(!stdout.contains("WRONG"), "stdout:\n{stdout}");
    assert!(stdout.contains("sum: 39200"), "stdout:\n{stdout}");
    assert!(
        stderr.contains("background lower published"),
        "worker did not run without JIT:\n{stderr}"
    );
}

/// Regression for #211: sequential destructuring of a collection shorter than
/// its pattern must bind the missing positions to `nil`, not throw — at every
/// tier.  Once `parse`'s reduce closure crossed the warm threshold it was
/// IR-lowered, and its inner `[opt-type opt optarg]` destructure of a
/// two-element token threw "index out of bounds: 2 >= 2" (the lowerer emitted a
/// strict `nth`; Clojure destructuring is `(nth coll idx nil)`).  Mirrors
/// `clojure.tools.cli/parse-option-tokens`: a `reduce` returning a three-element
/// vector whose step destructures short tokens.
#[test]
fn short_destructure_in_warm_reduce_yields_nil_not_oob() {
    let src = r#"
        (defn parse [tokens]
          (reduce
            (fn [[m errors args] [opt-type opt optarg]]
              (if (= opt-type :short)
                [(assoc m opt 1) errors args]
                [m errors (conj args opt)]))
            [{} [] []]
            tokens))
        (loop [i 0]
          (when (< i 200)
            (let [[opts errors args] (parse [[:short "-v"]])]
              (when (or (not= opts {"-v" 1}) (not= errors []) (not= args []))
                (println "WRONG at" i opts errors args)))
            (recur (+ i 1))))
        (println "ok")
    "#;
    let (stdout, stderr) = run_warm(src, &["--jit-threshold", "50"], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "short destructure miscompiled after warm-up:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("ok"), "program did not complete:\n{stdout}");
}

/// `--ir-threshold 0` disables background lowering: no publish may appear.
#[test]
fn ir_threshold_zero_disables_background_lowering() {
    let src = r#"
        (defn stay-cold [x] (+ x 5))
        (loop [i 0 acc 0]
          (if (< i 200)
            (recur (+ i 1) (+ acc (stay-cold i)))
            (println "sum:" acc)))
    "#;
    let (stdout, stderr) = run_warm(src, &["--ir-threshold", "0"], &[]);
    assert!(stdout.contains("sum: 20900"), "stdout:\n{stdout}");
    assert!(
        !stderr.contains("background lower published"),
        "lowering ran despite --ir-threshold 0:\n{stderr}"
    );
}
