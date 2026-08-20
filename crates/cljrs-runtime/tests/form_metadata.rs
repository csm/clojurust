//! Reader metadata survives from source to value.
//!
//! `^m form` is data inside `quote` and an annotation on the evaluated value
//! outside it; either way `(meta …)` must see it, including through a macro
//! that passes the annotated form along.

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

fn eval_str(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    format!("{result}")
}

#[test]
fn quote_keeps_the_annotation_as_data() {
    assert_eq!(eval_str("(meta (quote ^{:a 1} [1]))"), "{:a 1}");
    assert_eq!(eval_str("(meta '^{:a 1} [1])"), "{:a 1}");
}

#[test]
fn quoted_metadata_is_not_evaluated() {
    assert_eq!(eval_str("(meta (quote ^{:x (+ 1 2)} [1]))"), "{:x (+ 1 2)}");
}

#[test]
fn auto_keywords_inside_metadata_resolve() {
    assert_eq!(eval_str("(meta (quote ^{:x ::k} [1]))"), "{:x :user/k}");
}

#[test]
fn metadata_survives_a_macro_round_trip() {
    // The reported repro: the macro receives the annotated form as a value and
    // hands it back inside `quote`.
    assert_eq!(
        eval_str("(defmacro q [f] (list 'quote f)) (meta (q ^{:x ::k} [1]))"),
        "{:x :user/k}"
    );
}

#[test]
fn shorthand_annotations_expand() {
    assert_eq!(eval_str("(meta (quote ^:dyn sym))"), "{:dyn true}");
    assert_eq!(eval_str("(meta (quote ^String s))"), "{:tag String}");
    assert_eq!(eval_str("(meta ^:dyn [1])"), "{:dyn true}");
    assert_eq!(eval_str("(meta ^String [1])"), "{:tag String}");
}

#[test]
fn stacked_annotations_merge_with_the_outer_one_winning() {
    assert_eq!(eval_str("(meta (quote ^:a ^:b [1]))"), "{:b true, :a true}");
    assert_eq!(eval_str("(meta (quote ^{:a 1} ^{:a 2} [1]))"), "{:a 1}");
    assert_eq!(eval_str("(meta ^:a ^:b [1])"), "{:b true, :a true}");
}

#[test]
fn evaluated_annotations_see_the_enclosing_scope() {
    assert_eq!(eval_str("(let [x 5] (meta ^{:x x} [1]))"), "{:x 5}");
}

#[test]
fn the_annotated_value_is_unchanged() {
    assert_eq!(eval_str("(quote ^{:a 1} [1])"), "[1]");
    assert_eq!(eval_str("(= [1] ^{:a 1} [1])"), "true");
    assert_eq!(eval_str("(count ^{:a 1} [1 2])"), "2");
    assert_eq!(eval_str("(conj ^{:a 1} [1] 2)"), "[1 2]");
}

#[test]
fn scalars_carry_no_metadata() {
    assert_eq!(eval_str("(meta (quote ^{:a 1} 42))"), "nil");
    assert_eq!(eval_str("(meta ^{:a 1} 42)"), "nil");
    assert_eq!(eval_str("(inc ^{:a 1} 41)"), "42");
}

#[test]
fn a_def_name_tag_is_a_symbol() {
    assert_eq!(
        eval_str("(def ^String x 1) (:tag (meta (var x)))"),
        "String"
    );
}

#[test]
fn nil_metadata_leaves_no_wrapper() {
    // `->` threads with `(with-meta … (meta form))`, and `(meta form)` is nil for
    // an unannotated form. A nil annotation must carry nothing: a stored
    // nil-meta wrapper survives into `type` and breaks `identical?` on a clone.
    assert_eq!(eval_str("(meta (with-meta [1] nil))"), "nil");
    assert_eq!(eval_str("(type (with-meta {} nil))"), "Map");
    assert_eq!(
        eval_str("(let [y (with-meta {} nil)] (identical? y y))"),
        "true"
    );
    assert_eq!(eval_str("(type (-> (hash-map :a 1) (dissoc :a)))"), "Map");
    assert_eq!(
        eval_str("(let [y (-> (hash-map :a 1) (dissoc :a))] (identical? y y))"),
        "true"
    );
    assert_eq!(
        eval_str("(-> {} (with-meta {:foo 42}) (conj [:k :v]) meta)"),
        "{:foo 42}"
    );
}

#[test]
fn type_sees_through_an_annotation() {
    assert_eq!(eval_str("(type ^{:a 1} [1])"), "Vector");
    assert_eq!(eval_str("(type ^{:a 1} {})"), "Map");
    assert_eq!(eval_str("(type (with-meta '(1) {:a 1}))"), "List");
}
