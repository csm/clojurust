//! The tree-walker memoizes a macro call's expansion on the call-site form
//! (`cljrs_reader::ExpansionCache`), so a call site inside a function body or
//! a `loop` is expanded once rather than on every evaluation (csm/clojurust#459).
//!
//! These tests count expansions with a macro whose body has a side effect.

use std::sync::Arc;

use cljrs_reader::{Form, Parser};
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn make_env() -> Env {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    Env::new(globals, "user")
}

fn parse(src: &str) -> Vec<Form> {
    Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse error")
}

fn eval_forms(forms: &[Form], env: &mut Env) -> Value {
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(form, env).expect("eval error");
    }
    result
}

fn eval_src(src: &str, env: &mut Env) -> Value {
    eval_forms(&parse(src), env)
}

const COUNTING_MACRO: &str = "
(def expansions (atom 0))
(defmacro counted [x] (swap! expansions inc) `(inc ~x))
";

#[test]
fn call_site_in_a_loop_expands_once() {
    let mut env = make_env();
    eval_src(COUNTING_MACRO, &mut env);
    let v = eval_src(
        "(loop [i 0 acc 0] (if (< i 100) (recur (inc i) (+ acc (counted i))) acc))",
        &mut env,
    );
    assert_eq!(v, Value::Long((1..=100).sum()));
    assert_eq!(eval_src("@expansions", &mut env), Value::Long(1));
}

#[test]
fn call_site_in_a_fn_body_expands_once_across_calls() {
    let mut env = make_env();
    eval_src(COUNTING_MACRO, &mut env);
    eval_src("(defn f [x] (when (pos? x) (counted x)))", &mut env);
    let v = eval_src("(reduce + (map f (range 1 50)))", &mut env);
    assert_eq!(v, Value::Long((2..=50).sum()));
    assert_eq!(eval_src("@expansions", &mut env), Value::Long(1));
}

#[test]
fn closures_created_from_one_fn_form_share_expansions() {
    let mut env = make_env();
    eval_src(COUNTING_MACRO, &mut env);
    // Each call to `mk` creates a new closure whose body is a copy of the
    // `(fn ...)` form's body; the expansion made by the first closure must
    // serve the rest.
    eval_src("(defn mk [n] (fn [] (counted n)))", &mut env);
    let v = eval_src("(reduce + (map (fn [i] ((mk i))) (range 10)))", &mut env);
    assert_eq!(v, Value::Long((1..=10).sum()));
    assert_eq!(eval_src("@expansions", &mut env), Value::Long(1));
}

#[test]
fn redefining_the_macro_invalidates_the_expansion() {
    let mut env = make_env();
    eval_src(
        "(defmacro m [] 1)
         (defn g [] (m))",
        &mut env,
    );
    assert_eq!(eval_src("(g)", &mut env), Value::Long(1));
    eval_src("(defmacro m [] 2)", &mut env);
    assert_eq!(eval_src("(g)", &mut env), Value::Long(2));
}

#[test]
fn freshly_read_forms_are_expanded_again() {
    // A REPL re-reads its input, so evaluating the same text twice must run
    // the macro twice, even if the macro's result depends on mutable state.
    let mut env = make_env();
    eval_src(COUNTING_MACRO, &mut env);
    eval_src("(counted 1)", &mut env);
    eval_src("(counted 1)", &mut env);
    assert_eq!(eval_src("@expansions", &mut env), Value::Long(2));
}

#[test]
fn expansion_is_keyed_by_namespace() {
    let mut env = make_env();
    // `::k` in a macro argument resolves against the calling namespace.
    eval_src("(defmacro kw-of [k] k)", &mut env);
    eval_src("(ns other) (in-ns 'user)", &mut env);
    // One form object, evaluated in two namespaces.
    let site = parse("(user/kw-of ::k)");
    env.current_ns = Arc::from("user");
    let in_user = eval_forms(&site, &mut env);
    env.current_ns = Arc::from("other");
    let in_other = eval_forms(&site, &mut env);
    assert_eq!(in_user, eval_src(":user/k", &mut env));
    assert_eq!(in_other, eval_src(":other/k", &mut env));
}

#[test]
fn macro_using_env_and_gensyms_still_evaluates_correctly() {
    let mut env = make_env();
    let v = eval_src(
        "(defmacro swap-add [a b] `(let [t# ~a] (+ ~b t#)))
         (defn h [x] (let [y (* 2 x)] (swap-add x y)))
         (mapv h [1 2 3])",
        &mut env,
    );
    assert_eq!(
        v,
        eval_src("[3 6 9]", &mut env),
        "expansion reused across calls must still bind fresh locals"
    );
}
