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

/// Run a program that must reach Tier 1, escalating its workload until the
/// background worker actually publishes.
///
/// Publication is inherently asynchronous: the mutator enqueues at the warm
/// threshold and a separate worker lowers and publishes. A short program can
/// therefore finish before the worker is ever scheduled, and asserting on
/// whatever happened to land by exit is a race, not a property. Under
/// full-suite parallelism that race loses often enough to fail CI, and the
/// failure is indistinguishable from a genuine tier-up regression — which is
/// the expensive part.
///
/// `make_src` receives an iteration count so the same program can be made
/// arbitrarily longer-running. The happy path stays as fast as before: the
/// first, smallest attempt is the old workload, and the larger ones only run
/// on a machine too loaded to have scheduled the worker yet.
///
/// This widens the window rather than closing it. The deterministic fix is
/// runtime-side — drain pending background lowering at shutdown, which would
/// also stop a human debugging a short program from seeing an empty ir log.
/// Until that exists, an escalating budget is the honest test-side answer:
/// still a real failure when publication never happens at all.
fn run_until_published(
    make_src: impl Fn(usize) -> String,
    extra_args: &[&str],
    extra_env: &[(&str, &str)],
) -> (String, String) {
    const BUDGETS: [usize; 3] = [200, 5_000, 50_000];
    let mut last = (String::new(), String::new());
    for n in BUDGETS {
        let (stdout, stderr) = run_warm(&make_src(n), extra_args, extra_env);
        if stderr.contains("background lower published") {
            return (stdout, stderr);
        }
        last = (stdout, stderr);
    }
    panic!(
        "worker never published across {BUDGETS:?} iterations\nstdout:\n{}\nstderr:\n{}",
        last.0, last.1
    );
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
    // The program checks its own arithmetic against the closed form for
    // sum(i^2), so the correctness assertion holds at whatever workload the
    // escalating budget settles on.
    let (stdout, stderr) = run_until_published(
        |n| {
            format!(
                r#"
        (defn warm-me [x] (* x x))
        (let [n {n}
              acc (loop [i 0 acc 0]
                    (if (< i n) (recur (+ i 1) (+ acc (warm-me i))) acc))
              want (quot (* (- n 1) n (- (* 2 n) 1)) 6)]
          (println (if (= acc want) "sum-ok" (str "WRONG " acc " != " want))))
    "#
            )
        },
        &[],
        &[],
    );
    assert!(!stdout.contains("WRONG"), "stdout:\n{stdout}");
    assert!(stdout.contains("sum-ok"), "stdout:\n{stdout}");
    assert!(
        stderr.contains("background lower published"),
        "no background publish in stderr:\n{stderr}"
    );
}

/// `(str tag id)` with a `nil` arg must keep treating `nil` as the empty
/// string once the caller tiers up from tree-walk to the IR interpreter —
/// not print the literal `"nil"`.
#[test]
fn str_with_nil_arg_stays_correct_after_tiering_up() {
    let src = r#"
        (defn get-tag [tag id] (str tag id))
        (loop [i 0]
          (when (< i 200)
            (let [got (get-tag "h1" nil)]
              (when (not= got "h1") (println "WRONG at" i got)))
            (recur (+ i 1))))
        (println "done")
    "#;
    let (stdout, stderr) = run_warm(src, &["--jit-threshold", "0"], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "str(nil) corrupted after IR lowering:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(stdout.contains("done"), "stdout:\n{stdout}");
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

/// Background lowering is owned by `cljrs_runtime::tiered` and must work with the JIT
/// disabled: functions still tier up from tree-walk to the IR interpreter.
#[test]
fn background_lowering_works_without_jit() {
    let (stdout, stderr) = run_until_published(
        |n| {
            format!(
                r#"
        (defn no-jit-fn [x] (- (* 2 x) 3))
        (let [n {n}
              acc (loop [i 0 acc 0]
                    (if (< i n)
                      (do
                        (when (not= (no-jit-fn i) (- (* 2 i) 3)) (println "WRONG at" i))
                        (recur (+ i 1) (+ acc (no-jit-fn i))))
                      acc))
              want (- (* n (- n 1)) (* 3 n))]
          (println (if (= acc want) "sum-ok" (str "WRONG " acc " != " want))))
    "#
            )
        },
        &[],
        &[("CLJRS_NO_JIT", "1")],
    );
    assert!(!stdout.contains("WRONG"), "stdout:\n{stdout}");
    assert!(stdout.contains("sum-ok"), "stdout:\n{stdout}");
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

/// Regression for the Tier-1 "not callable" family: forms the tree-walker
/// implements specially must not lower to generic calls of their clojure.core
/// stub vars (which return nil) or to unresolvable globals.  Each shape below
/// worked tree-walked and broke permanently once its function crossed the
/// warm threshold:
///
/// - `(.method target …)` interop lowered to `LoadGlobal(ns, ".method")` →
///   "var not found ns/.method" at Tier 1 (and a nil call — "not callable:
///   <nil> is not callable" — once JIT-compiled).  Now lowers to a dot-marked
///   `CallDirect` routed to the interpreter's method dispatch.
/// - `Math/abs` is registered in clojure.core under its full slash name, but
///   lowering split it into (ns="Math", name="abs") → "var not found".  Both
///   `load_global_value` and `rt_load_global` now retry the whole name.
/// - `((var f) …)` lowered to a call of the `var` stub → nil → "not callable:
///   <nil> is not callable".  Now lowers to `LoadVar` like `#'f`.
/// - `(with-out-str …)` dispatched KnownFn::WithOutStr to the clojure.core
///   stub → returned nil.  Now captures output natively.
#[test]
fn interpreter_special_shapes_survive_promotion() {
    let src = r#"
        (defn shout [s] (.toUpperCase s))
        (defn measure [s] (+ (.length s) (.indexOf s "b")))
        (defn absolutize [x] (Math/abs x))
        (defn leaf [x] (str "v" x))
        (defn via-var [x] ((var leaf) x))
        (defn capture [x] (with-out-str (print "out" x)))
        (loop [i 0]
          (when (< i 200)
            (when (not= (shout "abc") "ABC") (println "WRONG-interop at" i))
            (when (not= (measure "abc") 4) (println "WRONG-measure at" i))
            (when (not= (absolutize -7) 7) (println "WRONG-math at" i))
            (when (not= (via-var 3) "v3") (println "WRONG-var at" i (via-var 3)))
            (when (not= (capture 1) "out 1") (println "WRONG-wos at" i (capture 1)))
            (recur (+ i 1))))
        (println "special-shapes ok")
    "#;
    let (stdout, stderr) = run_warm(src, &[], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "interpreter-special shape diverged after warm-up:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("special-shapes ok"),
        "program did not complete:\n{stdout}\nstderr:\n{stderr}"
    );
}

/// Interpreter-only special forms with no IR equivalent (defmulti, defmethod,
/// defrecord, reify, …) must be *rejected* by lowering — the function stays
/// at Tier 0 — rather than lowered to calls of their nil stub vars.  A
/// defmethod registered from inside a warm function must keep taking effect
/// after the warm threshold.
#[test]
fn interpreter_only_forms_stay_at_tier0() {
    let src = r#"
        (defmulti pick :kind)
        (defmethod pick :default [m] :none)
        (defn install [k v]
          (defmethod pick k [m] v)
          k)
        (loop [i 0]
          (when (< i 60)
            (install :a :got-a)
            (when (not= (pick {:kind :a}) :got-a) (println "WRONG-defmethod at" i))
            (recur (+ i 1))))
        (println "tier0-forms ok")
    "#;
    let (stdout, stderr) = run_warm(src, &[], &[]);
    assert!(
        !stdout.contains("WRONG"),
        "defmethod inside a warm fn stopped registering:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("tier0-forms ok"),
        "program did not complete:\n{stdout}\nstderr:\n{stderr}"
    );
}
