//! Regression tests for issue #194: named anonymous functions' self-reference
//! should be pointer-equal to the function value itself.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_in(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

fn eval_str(src: &str) -> Value {
    let (_, mut env) = make_env();
    eval_in(src, &mut env)
}

// ── Self-reference identity ───────────────────────────────────────────────────

#[test]
fn named_fn_self_ref_is_identical() {
    // (fn g [] g) should return itself: (= f (f)) => true
    let result = eval_str("(let [f (fn g [] g)] (= f (f)))");
    assert_eq!(result, Value::Bool(true), "(= f (f)) should be true");
}

#[test]
fn named_fn_self_ref_recursive_countdown() {
    // A recursive named fn should still work correctly.
    let result = eval_str(
        "(let [count-down (fn countdown [n] (if (= n 0) :done (countdown (- n 1))))]
           (count-down 5))",
    );
    assert!(
        matches!(&result, Value::Keyword(p) if p.get().name.as_ref() == "done"),
        "expected :done, got {:?}",
        result
    );
}

#[test]
fn named_fn_self_ref_multi_call() {
    // Repeated calls to (f) each return the same f.
    let result = eval_str(
        "(let [f (fn g [] g)]
           (and (= f (f)) (= f ((f))) (= (f) ((f)))))",
    );
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn defn_self_ref_identity() {
    // Top-level defn: the function returned from the body should also
    // be identical to the global binding.
    let (_, mut env) = make_env();
    eval_in("(defn self-ref [] self-ref)", &mut env);
    let result = eval_in("(= self-ref (self-ref))", &mut env);
    assert_eq!(result, Value::Bool(true));
}
