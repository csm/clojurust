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
    eval_in_env(src, &mut env)
}

fn eval_in_env(src: &str, env: &mut Env) -> Result<Value, String> {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, env).map_err(|e| format!("eval: {e:?}"))?;
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
        err.contains(
            "Multiple methods in multimethod 'g' match dispatch value :user/d: \
             :user/b and :user/c, and neither is preferred"
        ),
        "expected a stable ambiguity error, got: {err}"
    );

    let v = eval_ok(
        "(derive ::d ::b) (derive ::d ::c) (defmulti g identity) \
         (defmethod g ::b [_] :b) (defmethod g ::c [_] :c) (prefer-method g ::b ::c) (g ::d)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("b")));
}

#[test]
fn ambiguity_lists_three_matches_stably_and_grammatically() {
    let err = eval_fresh(
        "(derive ::d ::b) (derive ::d ::c) (derive ::d ::e) (defmulti g identity) \
         (defmethod g ::e [_] :e) (defmethod g ::c [_] :c) (defmethod g ::b [_] :b) (g ::d)",
    )
    .expect_err("three unrelated matching methods must be ambiguous");
    assert!(
        err.contains(":user/b and :user/c and :user/e, and none is preferred"),
        "expected sorted keys and plural wording, got: {err}"
    );
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
         (f ::a) (remove-method f ::b) (f ::a)",
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
         (f ::a) (underive ::a ::b) (f ::a)",
    )
    .expect_err("underive must break the dispatch path it created");
    assert!(
        err.contains("No method in multimethod"),
        "expected a no-method error, got: {err}"
    );
}

#[test]
fn derive_invalidates_a_cached_default_method() {
    let v = eval_ok(
        "(defmulti f identity) (defmethod f ::b [_] :parent) \
         (defmethod f :default [_] :fallback) (f ::a) (derive ::a ::b) (f ::a)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("parent")));
}

#[test]
fn defmethod_invalidates_a_cached_inherited_method() {
    let v = eval_ok(
        "(derive ::a ::c) (derive ::c ::b) (defmulti f identity) \
         (defmethod f ::b [_] :b) (f ::a) (defmethod f ::c [_] :c) (f ::a)",
    );
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("c")));
}

#[test]
fn inherited_resolution_populates_and_reuses_the_method_cache() {
    let (globals, mut env) = make_env();
    let v = eval_in_env(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] :parent) (f ::a)",
        &mut env,
    )
    .expect("first dispatch");
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("parent")));
    let Value::MultiFn(mf) = globals.lookup_in_ns("user", "f").expect("multifn") else {
        panic!("f was not a multimethod")
    };
    assert_eq!(mf.get().cached_method_count(), 1);

    let v = eval_in_env("(f ::a)", &mut env).expect("cached dispatch");
    assert_eq!(v, Value::keyword(cljrs_value::Keyword::parse("parent")));
    assert_eq!(mf.get().cached_method_count(), 1);
}

#[test]
fn methods_and_prefers_keep_dispatch_values_as_keys() {
    let v = eval_ok(
        "(defmulti f identity) (defmethod f ::b [_] :b) \
         (prefer-method f ::b ::c) \
         [(contains? (methods f) ::b) \
          (contains? (methods f) (str ::b)) \
          (contains? (prefers f) ::b) \
          (contains? (get (prefers f) ::b) ::c)]",
    );
    assert_eq!(format!("{v}"), "[true false true true]");
}

#[test]
fn prefer_method_rejects_the_reverse_preference() {
    let err = eval_fresh(
        "(defmulti f identity) (defmethod f ::b [_] :b) (defmethod f ::c [_] :c) \
         (prefer-method f ::b ::c) (prefer-method f ::c ::b)",
    )
    .expect_err("reverse preferences must conflict");
    assert!(
        err.contains("Preference conflict") && err.contains(":user/b is already preferred"),
        "unexpected conflict error: {err}"
    );
}

#[test]
fn clojure_isa_and_multimethod_dispatch_share_hierarchy_semantics() {
    let v = eval_ok(
        "(derive ::a ::b) (defmulti f identity) (defmethod f ::b [_] true) \
         (= (isa? [::a ::a] [::b ::b]) (f ::a))",
    );
    assert_eq!(v, Value::Bool(true));
}

#[test]
fn hierarchy_helpers_are_private_and_not_auto_referred() {
    for name in [
        "named?",
        "hierarchy?",
        "check-hierarchy!",
        "check-tag!",
        "extend-relation",
        "global-hierarchy",
    ] {
        let src = format!("(resolve '{name})");
        assert_eq!(eval_ok(&src), Value::Nil, "{name} leaked into user");
    }
}

#[test]
fn defn_private_vars_are_not_referred_by_refer_all() {
    let (globals, mut env) = make_env();
    eval_in_env(
        "(in-ns 'private-source) (defn- secret [] :secret)",
        &mut env,
    )
    .expect("define private var");
    globals.get_or_create_ns("private-client");
    globals.refer_all("private-client", "private-source");
    assert!(
        globals.lookup_in_ns("private-client", "secret").is_none(),
        "refer-all exposed a defn- var"
    );
}
