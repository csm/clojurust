//! Metadata on `ns` forms: `(ns ^{:doc "..."} my.ns ...)`, `(ns ^:no-doc my.ns)`,
//! and the optional attr-map form `(ns my.ns "doc" {:author "x"})`.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_all(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

#[test]
fn ns_with_map_metadata() {
    let (_, mut env) = make_env();
    let result = eval_all(&mut env, r#"(ns ^{:doc "A namespace"} my.meta.ns)"#);
    let Value::Namespace(ns) = result else {
        panic!("expected Namespace, got {result:?}");
    };
    let meta = ns.get().get_meta().expect("ns should have metadata");
    let Value::Map(m) = meta else {
        panic!("expected map metadata, got {meta:?}");
    };
    let doc = m
        .get(&Value::keyword(cljrs_value::Keyword::parse("doc")))
        .expect("expected :doc key");
    assert_eq!(doc, Value::string("A namespace".to_string()));
}

#[test]
fn ns_with_keyword_shorthand_metadata() {
    let (_, mut env) = make_env();
    let result = eval_all(&mut env, "(ns ^:no-doc my.meta.ns2)");
    let Value::Namespace(ns) = result else {
        panic!("expected Namespace, got {result:?}");
    };
    let meta = ns.get().get_meta().expect("ns should have metadata");
    let Value::Map(m) = meta else {
        panic!("expected map metadata, got {meta:?}");
    };
    let v = m
        .get(&Value::keyword(cljrs_value::Keyword::parse("no-doc")))
        .expect("expected :no-doc key");
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn ns_without_metadata_still_works() {
    let (_, mut env) = make_env();
    let result = eval_all(&mut env, "(ns my.plain.ns)");
    let Value::Namespace(ns) = result else {
        panic!("expected Namespace, got {result:?}");
    };
    assert!(ns.get().get_meta().is_none());
}

#[test]
fn ns_docstring_and_requires_clause_still_work_with_metadata() {
    let (_, mut env) = make_env();
    let result = eval_all(
        &mut env,
        r#"(ns ^{:doc "top"} my.meta.ns3
             "a docstring"
             (:require [clojure.core]))"#,
    );
    let Value::Namespace(ns) = result else {
        panic!("expected Namespace, got {result:?}");
    };
    let meta = ns.get().get_meta().expect("ns should have metadata");
    let Value::Map(m) = meta else {
        panic!("expected map metadata, got {meta:?}");
    };
    let doc = m
        .get(&Value::keyword(cljrs_value::Keyword::parse("doc")))
        .expect("expected :doc key");
    assert_eq!(doc, Value::string("top".to_string()));
}
