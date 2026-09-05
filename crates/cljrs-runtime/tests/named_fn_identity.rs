//! Regression tests for issue #194: named anonymous functions' self-reference
//! should be pointer-equal to the function value itself.

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

fn eval_in(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).expect("eval error");
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

// ── A param shadows the function's own name (PR #353) ────────────────────────
//
// The self-reference and the params bind into the SAME frame, and the
// self-reference used to be bound last — so it overwrote a param that shared
// the function's name. `(defn text [text] {:text text})` returned the function
// as its own `:text`. Binding the self-reference first lets the param shadow
// it, as Clojure does, while the name stays visible in a body that does not
// shadow it (the tests above).

#[test]
fn param_shadows_the_fns_own_name() {
    let (_, mut env) = make_env();
    eval_in("(defn text [text] {:text text})", &mut env);
    let result = eval_in(r#"(= "hi" (:text (text "hi")))"#, &mut env);
    assert_eq!(
        result,
        Value::Bool(true),
        "the param, not the fn itself, should be the :text"
    );
}

#[test]
fn param_shadows_the_fns_own_name_in_a_named_anon_fn() {
    let result = eval_str("(let [f (fn g [g] g)] (= 7 (f 7)))");
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn param_shadows_the_fns_own_name_in_a_rest_param() {
    let (_, mut env) = make_env();
    eval_in("(defn xs [& xs] xs)", &mut env);
    let result = eval_in("(= '(1 2) (xs 1 2))", &mut env);
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn param_shadows_the_fns_own_name_in_one_arity_only() {
    // Arity 1 shadows the name; arity 0 does not and must still see the fn.
    let (_, mut env) = make_env();
    eval_in("(defn f ([] f) ([f] f))", &mut env);
    assert_eq!(eval_in("(= f (f))", &mut env), Value::Bool(true));
    assert_eq!(eval_in("(= 3 (f 3))", &mut env), Value::Bool(true));
}
