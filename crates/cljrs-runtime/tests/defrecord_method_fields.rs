//! Regression tests for PR #353: a defrecord's fields must be in scope as bare
//! symbols inside an inline protocol method body, and a parameter sharing a
//! field's name must shadow the field.
//!
//! `(defrecord R [sha] P (mutable? [_] (valid-sha? sha)))` threw
//! "unbound symbol: sha": a method impl is built as an ordinary fn whose only
//! bindings are its own params, while Clojure compiles the fields as instance
//! fields of the generated class, so they are simply in scope.
//!
//! These cover both halves of `build_impl_fn`'s field plumbing — the synthesised
//! `(let* [f (:f this) ...] body)` and the cases that must NOT get one (reify,
//! extend-type, a param that already takes the field's name).

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

/// Evaluate `src` and return the last value rendered with `pr-str`.
fn eval_pr(src: &str) -> String {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string from pr-str, got {:?}", other),
    }
}

// ── Fields are in scope as bare symbols ──────────────────────────────────────

#[test]
fn bare_field_symbols_resolve_in_a_method_body() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (describe [this]))
             (defrecord R [a b] P (describe [_] [a b]))
             (pr-str (describe (->R 1 2)))"
        ),
        "[1 2]"
    );
}

#[test]
fn field_and_this_lookup_agree() {
    // The synthesised binding is `(:x this)`, so both spellings must match.
    assert_eq!(
        eval_pr(
            "(defprotocol P (both [this]))
             (defrecord R [x] P (both [this] [x (:x this)]))
             (pr-str (both (->R :v)))"
        ),
        "[:v :v]"
    );
}

#[test]
fn fields_are_read_off_the_instance_not_captured_at_definition() {
    // assoc returns a NEW record; the method must see the updated value, which
    // it only does because the field is bound from `this` per call.
    assert_eq!(
        eval_pr(
            "(defprotocol P (describe [this]))
             (defrecord R [a b] P (describe [_] [a b]))
             (pr-str (describe (assoc (->R 1 2) :a 9)))"
        ),
        "[9 2]"
    );
}

#[test]
fn a_record_with_no_fields_still_works() {
    // `synth_field_scope` returns None here — no `let*` wrapper is synthesised.
    assert_eq!(
        eval_pr(
            "(defprotocol P (tag [this]))
             (defrecord R [] P (tag [_] :ok))
             (pr-str (tag (->R)))"
        ),
        ":ok"
    );
}

// ── A param shadows a field of the same name ─────────────────────────────────

#[test]
fn a_param_shadows_the_field_it_shares_a_name_with() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (pick [this a]))
             (defrecord R [a] P (pick [_ a] a))
             (pr-str (pick (->R :field) :param))"
        ),
        ":param"
    );
}

#[test]
fn shadowing_one_field_leaves_the_others_bound() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (pick [this a]))
             (defrecord R [a b] P (pick [_ a] [a b]))
             (pr-str (pick (->R :field :other) :param))"
        ),
        "[:param :other]"
    );
}

// ── Forms that must NOT get a field scope ────────────────────────────────────

#[test]
fn reify_closes_over_its_environment_and_gains_no_fields() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (describe [this]))
             (pr-str (let [x 42] (describe (reify P (describe [_] x)))))"
        ),
        "42"
    );
}

#[test]
fn extend_type_still_reads_fields_through_this() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (describe [this]))
             (defrecord R [a b])
             (extend-type R P (describe [s] [(:a s) (:b s)]))
             (pr-str (describe (->R 1 2)))"
        ),
        "[1 2]"
    );
}
