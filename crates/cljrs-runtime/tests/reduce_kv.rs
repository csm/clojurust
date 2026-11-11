//! `reduce-kv` over vectors and maps (issue #478).
//!
//! The bootstrap definition destructured every element with `first`/`second`,
//! so a vector of pairs silently produced the pairs' contents instead of
//! `[index element]`, and a vector of scalars threw. On a vector the reducer
//! must receive the zero-based index and the whole element.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
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

fn shows(src: &str) -> String {
    let wrapped = format!("(pr-str {src})");
    match eval_fresh(&wrapped).unwrap_or_else(|e| panic!("{src}\n{e}")) {
        Value::Str(s) => s.get().clone(),
        other => panic!("{src}: expected a string, got {other:?}"),
    }
}

#[test]
fn vector_of_pairs_gets_index_and_whole_element() {
    assert_eq!(
        shows("(reduce-kv (fn [out k v] (conj out [k v])) [] [[:a :b] [:c :d]])"),
        "[[0 [:a :b]] [1 [:c :d]]]"
    );
}

#[test]
fn vector_of_scalars() {
    assert_eq!(
        shows("(reduce-kv (fn [out k v] (conj out [k v])) [] [7 8])"),
        "[[0 7] [1 8]]"
    );
}

#[test]
fn subvec_indices_start_at_zero() {
    assert_eq!(
        shows("(reduce-kv (fn [out k v] (conj out [k v])) [] (subvec [1 2 3 4] 2))"),
        "[[0 3] [1 4]]"
    );
}

#[test]
fn map_entries() {
    assert_eq!(
        shows("(reduce-kv (fn [out k v] (conj out [k v])) [] {:a 7})"),
        "[[:a 7]]"
    );
    assert_eq!(
        shows("(reduce-kv (fn [m k v] (assoc m k [v])) {} {:a [1 2]})"),
        "{:a [[1 2]]}"
    );
}

#[test]
fn empty_and_nil_return_init() {
    assert_eq!(
        shows("(reduce-kv (fn [a k v] (conj a k)) [:init] [])"),
        "[:init]"
    );
    assert_eq!(
        shows("(reduce-kv (fn [a k v] (conj a k)) [:init] {})"),
        "[:init]"
    );
    assert_eq!(
        shows("(reduce-kv (fn [a k v] (conj a k)) [:init] nil)"),
        "[:init]"
    );
}

#[test]
fn honours_reduced() {
    assert_eq!(
        shows(
            "(reduce-kv (fn [a k v] (if (= k 1) (reduced (conj a :stop)) (conj a v))) [] [[:x] [:y] [:z]])"
        ),
        "[[:x] :stop]"
    );
    assert_eq!(
        shows("(reduce-kv (fn [a k v] (reduced [k v])) nil {:a 1})"),
        "[:a 1]"
    );
}
