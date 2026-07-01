//! Regression tests for the `declare` macro (bootstrap.cljrs): it must def
//! each supplied name with no binding, enabling forward references, the way
//! `(def name)` does when written out by hand.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_gc::GcPtr;
use cljrs_reader::Parser;
use cljrs_value::{Keyword, Value};

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_src(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn declare_allows_forward_reference() {
    let result = eval_src("(declare foo) (defn baz [] (foo)) (defn foo [] 42) (baz)");
    assert_eq!(result, Value::Long(42));
}

#[test]
fn declare_accepts_multiple_names() {
    let result = eval_src("(declare a b c) [a b c]");
    // Each name is bound (unbound var, no throw); the vector should contain
    // three vars' unbound placeholders without erroring.
    assert!(matches!(result, Value::Vector(_)));
}

#[test]
fn if_not_multi_arity() {
    // test is falsy -> the `then` branch runs.
    assert_eq!(
        eval_src("(if-not false :a :b)"),
        Value::Keyword(GcPtr::new(Keyword::simple("a")))
    );
    // test is truthy -> the `else` branch runs.
    assert_eq!(
        eval_src("(if-not true :a :b)"),
        Value::Keyword(GcPtr::new(Keyword::simple("b")))
    );
}
