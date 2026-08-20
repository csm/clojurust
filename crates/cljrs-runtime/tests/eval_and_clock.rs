//! `eval` and the host clock.
//!
//! Both were unresolved symbols, which is what blocks portable libraries whose
//! timeout/retry code needs a clock and whose data-driven code needs `eval`.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .map_err(|e| format!("eval: {e:?}"))?;
    }
    Ok(result)
}

fn eval_ok(src: &str) -> Value {
    eval_fresh(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

// ── eval ─────────────────────────────────────────────────────────────────────

#[test]
fn eval_evaluates_a_form_value() {
    assert_eq!(eval_ok("(eval '(+ 1 2))"), Value::Long(3));
    assert_eq!(eval_ok("(eval (list '+ 1 2))"), Value::Long(3));
    assert_eq!(eval_ok("(eval (read-string \"(* 6 7)\"))"), Value::Long(42));
}

#[test]
fn eval_returns_self_evaluating_values_unchanged() {
    assert_eq!(eval_ok("(eval 42)"), Value::Long(42));
    assert_eq!(format!("{}", eval_ok("(eval [1 2])")), "[1 2]");
    assert_eq!(format!("{}", eval_ok("(eval ''foo)")), "foo");
}

#[test]
fn eval_sees_vars() {
    assert_eq!(eval_ok("(def y 7) (eval 'y)"), Value::Long(7));
    assert_eq!(eval_ok("(eval '(do (def z 9) z))"), Value::Long(9));
}

#[test]
fn eval_does_not_see_the_enclosing_lexical_scope() {
    // As on the JVM: `eval` runs at top level, so a local is unresolvable.
    let err =
        eval_fresh("(let [x 1] (eval 'x))").expect_err("eval must not resolve the caller's locals");
    assert!(
        err.contains('x'),
        "expected an unbound-symbol error, got: {err}"
    );
}

#[test]
fn eval_runs_macros_and_builds_functions() {
    assert_eq!(format!("{}", eval_ok("(eval '(-> 1 inc inc))")), "3");
    assert_eq!(eval_ok("((eval '(fn [a] (inc a))) 41)"), Value::Long(42));
}

// ── clock ────────────────────────────────────────────────────────────────────

#[test]
fn current_time_millis_is_a_wall_clock() {
    let Value::Long(now) = eval_ok("(System/currentTimeMillis)") else {
        panic!("expected a long");
    };
    // Later than 2020-01-01 and before 2100-01-01: a real epoch reading rather
    // than 0 or a nanosecond count.
    assert!(
        (1_577_836_800_000..4_102_444_800_000).contains(&now),
        "implausible wall-clock reading: {now}"
    );
}

#[test]
fn nano_time_is_monotonic() {
    let v = eval_ok("(let [a (System/nanoTime) b (System/nanoTime)] [a (- b a)])");
    let s = format!("{v}");
    let mut parts = s.trim_matches(['[', ']']).split(' ');
    let a: i64 = parts.next().unwrap().parse().unwrap();
    let delta: i64 = parts.next().unwrap().parse().unwrap();
    assert!(a >= 0, "nanoTime must count from a fixed origin, got {a}");
    assert!(delta >= 0, "nanoTime went backwards by {}", -delta);
}

#[test]
fn thread_sleep_waits() {
    let v = eval_ok(
        "(let [a (System/currentTimeMillis)] (Thread/sleep 20) (- (System/currentTimeMillis) a))",
    );
    let Value::Long(elapsed) = v else {
        panic!("expected a long");
    };
    assert!(elapsed >= 15, "sleep 20 returned after only {elapsed}ms");
}
