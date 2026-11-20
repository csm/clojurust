//! An exception thrown while a lazy sequence (or delay) is being realized
//! reaches `catch` as the original value.
//!
//! Regression: the thunk error path used to format `EvalError::Thrown(v)`
//! into a string, so `ex-message` returned the printed `#error {...}` (with a
//! Rust backtrace) and `ex-data` was lost. The lazy cell now caches and
//! re-throws the thrown value itself.

use cljrs_reader::Parser;
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_in(mode: ExecutionMode, src: &str) -> Value {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e}"));
    }
    result
}

/// Each consumer realizes a lazy seq whose element fn throws on `3`.
const CONSUMERS: &[(&str, &str)] = &[
    ("reduce", "(reduce + (map boom (range 10)))"),
    ("into-xform", "(into [] (map inc) (map boom (range 10)))"),
    ("doall", "(doall (map boom (range 10)))"),
    ("seq", "(seq (filter odd? (drop 3 (map boom (range 10)))))"),
    ("lazy-seq", "(seq (lazy-seq (boom 3)))"),
    ("delay", "(deref (delay (boom 3)))"),
];

const PRELUDE: &str = "(defn boom [x] (if (= x 3) (throw (ex-info \"boom\" {:x x :k :v})) x))";

#[test]
fn thrown_value_survives_lazy_realization() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::Tiered] {
        for (name, expr) in CONSUMERS {
            let src = format!(
                "{PRELUDE}
                 (try {expr} :no-throw
                   (catch :default e [(ex-message e) (ex-data e)]))"
            );
            let got = eval_in(mode, &src);
            let expected = eval_in(mode, "[\"boom\" {:x 3 :k :v}]");
            assert_eq!(got, expected, "{mode:?} / {name}: {expr}");
        }
    }
}

#[test]
fn issue_snippet_returns_exact_message() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::Tiered] {
        let got = eval_in(
            mode,
            "(try (reduce + (map (fn [x] (if (= x 3) (throw (ex-info \"boom\" {})) x)) (range 10)))
                  (catch :default e (ex-message e)))",
        );
        assert_eq!(got, Value::string("boom"), "{mode:?}");
    }
}

/// A failed realization is cached: every later reader re-throws the same
/// value, not a stringified copy.
#[test]
fn cached_failure_rethrows_identical_value() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::Tiered] {
        let got = eval_in(
            mode,
            &format!(
                "{PRELUDE}
                 (let [s (map boom (range 10))
                       a (try (doall s) (catch :default e e))
                       b (try (doall s) (catch :default e e))]
                   [(identical? a b) (ex-message b) (ex-data b)])"
            ),
        );
        let expected = eval_in(mode, "[true \"boom\" {:x 3 :k :v}]");
        assert_eq!(got, expected, "{mode:?}");
    }
}
