//! #538: consuming a lazy sequence must not pin its consumed prefix through
//! an obsolete reference in the native `reduce` call path or the tree-walking
//! function-call path, while every reference the program can still observe
//! (a global, a local used later, a closure capture) keeps the head alive.
//!
//! A native `probe` mapper forces two collections (sweeping grants a grace
//! cycle) at elements 2,000 and 6,000 and records the live object count. A
//! bounded consumer shows no growth between the samples; a retaining one
//! grows by roughly two objects per element.
//!
//! Everything runs in one test so live-object counts are not disturbed by
//! other tests sharing the process heap.
#![cfg(not(feature = "no-gc"))]
#![allow(clippy::result_large_err)] // `with_eval_context` returns `EvalError`.

use cljrs_gc::{GcPtr, HEAP};
use cljrs_reader::Parser;
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::{callback, env::Env, gc_roots};
use cljrs_value::{Arity, NativeFn, Value};
use std::sync::{Arc, Mutex};

/// Sum of `(range 8000)`.
const SUM: i64 = 31_996_000;
/// Allowed growth for a bounded consumer between the two samples.
const BOUNDED: isize = 500;
/// Minimum growth for a consumer that must retain the 4,000 elements between
/// the samples.
const RETAINED: isize = 3_000;

fn force_gc() {
    callback::with_eval_context(|env| {
        gc_roots::force_collect(env);
        gc_roots::force_collect(env);
        Ok(())
    })
    .unwrap();
}

fn eval(src: &str, env: &mut Env) -> Value {
    let forms = Parser::new(src.into(), "<call-frame-liveness>".into())
        .parse_all()
        .unwrap();
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::tiered::eval(form, env).unwrap_or_else(|e| panic!("{src}: {e:?}"));
    }
    result
}

fn setup(mode: ExecutionMode, samples: &Arc<Mutex<Vec<usize>>>) -> Env {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .build()
        .unwrap()
        .into_globals();
    let record = samples.clone();
    globals.intern(
        "user",
        "probe".into(),
        Value::NativeFunction(GcPtr::new(NativeFn::with_closure(
            "probe",
            Arity::Fixed(1),
            move |args| {
                if let Value::Long(2000 | 6000) = args[0] {
                    force_gc();
                    record.lock().unwrap().push(HEAP.count());
                }
                Ok(args[0].clone())
            },
        ))),
    );
    globals.intern(
        "user",
        "gc-id".into(),
        Value::NativeFunction(GcPtr::new(NativeFn::new(
            "gc-id",
            Arity::Fixed(1),
            |args| {
                force_gc();
                Ok(args[0].clone())
            },
        ))),
    );
    Env::new(globals, "user")
}

/// Evaluate `src`, check its result, and return the live-object growth
/// between the two probe samples.
fn growth(src: &str, expected: i64, env: &mut Env, samples: &Mutex<Vec<usize>>) -> isize {
    samples.lock().unwrap().clear();
    assert_eq!(eval(src, env), Value::Long(expected), "{src}");
    let samples = samples.lock().unwrap();
    assert_eq!(samples.len(), 2, "{src}: probe samples");
    samples[1] as isize - samples[0] as isize
}

const BOUNDED_CASES: &[(&str, &str, i64)] = &[
    ("reduce/2", "(reduce + (map probe (range 8000)))", SUM),
    ("reduce/3", "(reduce + 0 (map probe (range 8000)))", SUM),
    (
        "reduce alias",
        "(def my-reduce reduce) (my-reduce + (map probe (range 8000)))",
        SUM,
    ),
    (
        "heap accumulator",
        "(:sum (reduce (fn [m x] (assoc m :sum (+ (:sum m) x)))
                       {:sum 0} (map probe (range 8000))))",
        SUM,
    ),
    (
        "fn recur",
        "(defn consume-recur [s acc]
           (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))
         (consume-recur (map probe (range 8000)) 0)",
        SUM,
    ),
    (
        "final loop",
        "(defn consume-loop [s]
           (loop [s s acc 0]
             (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc)))
         (consume-loop (map probe (range 8000)))",
        SUM,
    ),
    (
        "top-level loop",
        "(loop [s (map probe (range 8000)) acc 0]
           (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))",
        SUM,
    ),
];

const RETAINED_CASES: &[(&str, &str, i64)] = &[
    (
        "global head",
        "(def source (map probe (range 8000))) (reduce + source)",
        SUM,
    ),
    (
        "local used later",
        "(let [s (map probe (range 8000))] (+ (reduce + s) (count s)))",
        SUM + 8000,
    ),
    (
        "captured head",
        "(defn hold [s] (fn [] s))
         (let [g (hold (map probe (range 8000)))] (+ (reduce + (g)) (count (g))))",
        SUM + 8000,
    ),
    (
        "param used after a non-final loop",
        "(defn later [s]
           (let [total (loop [s s acc 0]
                         (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))]
             (+ total (count s))))
         (later (map probe (range 8000)))",
        SUM + 8000,
    ),
    (
        "param read in finally",
        "(def seen (atom 0))
         (defn fin [s]
           (try (loop [s s acc 0]
                  (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))
                (finally (reset! seen (count s)))))
         (+ (fin (map probe (range 8000))) @seen)",
        SUM + 8000,
    ),
    (
        "captured by a loop initializer",
        "(defn cap [s]
           (loop [f (fn [] s) s s acc 0]
             (if-let [s (seq s)] (recur f (next s) (+ acc (first s))) (+ acc (count (f))))))
         (cap (map probe (range 8000)))",
        SUM + 8000,
    ),
];

/// Behaviour that the ownership changes must not alter.
const SEMANTIC_CASES: &[(&str, &str)] = &[
    ("(reduce + [])", "0"),
    ("(reduce + 5 [])", "5"),
    ("(reduce + nil)", "0"),
    ("(reduce + (map gc-id (range 0)))", "0"),
    ("(reduce + (map gc-id (range 10)))", "45"),
    ("(reduce + 100 (map gc-id (range 10)))", "145"),
    ("(reduce str \"abc\")", "\"abc\""),
    ("(reduce + (vals {:a 1 :b 2}))", "3"),
    (
        "(reduce (fn [a x] (if (= x 3) (reduced :stop) x)) (map gc-id (range 10)))",
        ":stop",
    ),
    // A throw while realizing the input propagates. (Its message is
    // currently re-wrapped by lazy-seq realization, so only check it
    // mentions the original.)
    (
        "(try (reduce + (map (fn [x] (if (= x 3) (throw (ex-info \"boom\" {})) x)) (range 10)))
              (catch :default e (boolean (re-find #\"boom\" (ex-message e)))))",
        "true",
    ),
    (
        "(try (reduce (fn [a x] (throw (ex-info \"step\" {}))) 0 [1])
              (catch :default e (ex-message e)))",
        "\"step\"",
    ),
    ("(try (reduce + 1) (catch :default e :not-seqable))", ":not-seqable"),
    ("(apply reduce + [[1 2 3]])", "6"),
    ("(map reduce [+ *] [[1 2] [3 4]])", "(3 12)"),
    ("(mapv (partial reduce +) [[1 2] [3 4]])", "[3 7]"),
    ("(let [r reduce] (identical? r reduce))", "true"),
    ("(let [v [1 2 3]] [(reduce + v) v])", "[6 [1 2 3]]"),
    // Variadic recur: the replacement values are only rooted by the recur
    // itself while the lazy tail is spread, which collects.
    (
        "(defn vs [acc & xs]
           (if-let [s (seq xs)] (recur (conj acc (first s)) (map gc-id (rest s))) acc))
         (vs [] 1 2 3 4)",
        "[1 2 3 4]",
    ),
    (
        "(defn keep-first [acc & xs]
           (if (seq xs) (recur acc (map gc-id (rest xs))) acc))
         (keep-first (into [] (map gc-id) (range 5)) 1 2 3)",
        "[0 1 2 3 4]",
    ),
    // A final loop only hides the parameters it rebinds.
    (
        "(defn partial-shadow [s n] (loop [s (seq s) acc 0] (if s (recur (next s) (+ acc n)) acc)))
         (partial-shadow (map gc-id [1 2 3]) 10)",
        "30",
    ),
    // Destructured loop bindings are not retired; the param stays usable.
    (
        "(defn destr [p] (loop [[a b] p n 0] (if (< n 2) (recur [b a] (inc n)) [a b])))
         (destr [1 2])",
        "[1 2]",
    ),
    // Closures made before the final loop keep their own reference.
    (
        "(defn early [s] (let [f (fn [] (count s))] (loop [s s] (if (seq s) (recur (next s)) (f)))))
         (early (map gc-id (range 5)))",
        "5",
    ),
    // Initializers still see the outer binding they shadow.
    (
        "(defn init-sees [s] (loop [n (count s) s (seq s)] (if s (recur n (next s)) n)))
         (init-sees [1 2 3])",
        "3",
    ),
];

#[test]
fn obsolete_call_frame_references_do_not_pin_lazy_prefixes() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _mutator = cljrs_gc::register_mutator();
            for mode in [ExecutionMode::TreeWalk, ExecutionMode::TieredNoJit] {
                let samples = Arc::new(Mutex::new(Vec::new()));
                let mut env = setup(mode, &samples);
                for (name, src, expected) in BOUNDED_CASES {
                    let grew = growth(src, *expected, &mut env, &samples);
                    assert!(
                        grew < BOUNDED,
                        "{mode:?} {name}: live objects grew by {grew} while consuming"
                    );
                }
                for (name, src, expected) in RETAINED_CASES {
                    let grew = growth(src, *expected, &mut env, &samples);
                    assert!(
                        grew >= RETAINED,
                        "{mode:?} {name}: a reachable head must be retained (grew {grew})"
                    );
                }
                for (src, expected) in SEMANTIC_CASES {
                    assert_eq!(eval(src, &mut env).to_string(), *expected, "{mode:?} {src}");
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
