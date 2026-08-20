//! Multimethod dispatch through the ad-hoc hierarchy.
//!
//! The `derive` / `isa?` value semantics are covered by the vendored
//! clojure-test-suite (`clojure.core-test.derive` / `.underive`); what is
//! exercised here is the dispatch seam, which has no upstream coverage.

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

#[test]
fn dispatch_finds_a_parents_method() {
    let v = eval_ok("(defmulti f identity) (defmethod f ::b [_] :parent) (derive ::a ::b) (f ::a)");
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("parent")));
}

#[test]
fn dispatch_walks_transitive_ancestors() {
    let v = eval_ok(
        "(derive ::square ::rect) (derive ::rect ::shape) \
         (defmulti area identity) (defmethod area ::shape [_] :shape) (area ::square)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("shape")));
}

#[test]
fn the_most_specific_method_wins() {
    let v = eval_ok(
        "(derive ::square ::rect) (derive ::rect ::shape) (defmulti area identity) \
         (defmethod area ::shape [_] :shape) (defmethod area ::rect [_] :rect) (area ::square)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("rect")));
}

#[test]
fn an_exact_method_beats_an_inherited_one() {
    let v = eval_ok(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] :parent) \
         (defmethod f ::a [_] :exact) (f ::a)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("exact")));
}

#[test]
fn default_still_applies_to_unrelated_values() {
    let v = eval_ok(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] :parent) \
         (defmethod f :default [_] :fallback) (f ::unrelated)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("fallback")));
}

#[test]
fn unrelated_matches_are_ambiguous_until_preferred() {
    let err = eval_fresh(
        "(derive ::d ::b) (derive ::d ::c) (defmulti g identity) \
         (defmethod g ::b [_] :b) (defmethod g ::c [_] :c) (g ::d)",
    )
    .expect_err("two unrelated matching methods must not silently pick one");
    assert!(
        err.contains("Multiple methods"),
        "expected an ambiguity error, got: {err}"
    );

    let v = eval_ok(
        "(derive ::d ::b) (derive ::d ::c) (defmulti g identity) \
         (defmethod g ::b [_] :b) (defmethod g ::c [_] :c) (prefer-method g ::b ::c) (g ::d)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("b")));
}

#[test]
fn vector_dispatch_values_match_element_wise() {
    let v = eval_ok(
        "(derive ::sq ::rect) (defmulti h (fn [a b] [a b])) \
         (defmethod h [::rect ::rect] [_ _] :rects) (h ::sq ::sq)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("rects")));
}

#[test]
fn remove_method_drops_the_inherited_match() {
    let err = eval_fresh(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] :parent) \
         (remove-method f ::b) (f ::a)",
    )
    .expect_err("a removed method must not keep answering through the hierarchy");
    assert!(
        err.contains("No method in multimethod"),
        "expected a no-method error, got: {err}"
    );
}

#[test]
fn underive_retracts_the_inherited_match() {
    let err = eval_fresh(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] :parent) \
         (underive ::a ::b) (f ::a)",
    )
    .expect_err("underive must break the dispatch path it created");
    assert!(
        err.contains("No method in multimethod"),
        "expected a no-method error, got: {err}"
    );
}
