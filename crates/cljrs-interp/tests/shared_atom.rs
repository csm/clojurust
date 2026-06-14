//! End-to-end tests for the Phase B3 `shared-atom` Clojure surface.
//!
//! `shared-atom` is the cross-isolate tier of the two-tier atom ADR
//! (`docs/async-worker-pool-plan.md`): its contents live in a `Send + Sync`
//! `SharedValue` behind a lock-free `ArcSwap`, so the reference can cross the
//! isolate boundary and be mutated concurrently.  These tests pin the
//! Clojure-visible behaviour — construction, `deref`, `reset!`, `swap!`,
//! `compare-and-set!`, the `shared-atom?` predicate, and the promotability
//! restriction — through the tree-walking interpreter (the deterministic,
//! single-isolate path).

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate a multi-form source string in a shared env, returning the last value.
fn eval_src(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

fn eval_fresh(src: &str) -> Value {
    let (_, mut env) = make_env();
    eval_src(src, &mut env)
}

#[test]
fn shared_atom_construct_and_deref() {
    assert_eq!(eval_fresh("(deref (shared-atom 41))"), Value::Long(41));
    assert_eq!(eval_fresh("@(shared-atom 41)"), Value::Long(41));
}

#[test]
fn shared_atom_predicate() {
    assert_eq!(
        eval_fresh("(shared-atom? (shared-atom 1))"),
        Value::Bool(true)
    );
    assert_eq!(eval_fresh("(shared-atom? (atom 1))"), Value::Bool(false));
    assert_eq!(eval_fresh("(shared-atom? 1)"), Value::Bool(false));
    // A shared-atom is not a (local) atom and vice-versa.
    assert_eq!(eval_fresh("(atom? (shared-atom 1))"), Value::Bool(false));
}

#[test]
fn shared_atom_reset() {
    let mut env = make_env().1;
    eval_src("(def a (shared-atom 0))", &mut env);
    assert_eq!(eval_src("(reset! a 99)", &mut env), Value::Long(99));
    assert_eq!(eval_src("@a", &mut env), Value::Long(99));
}

#[test]
fn shared_atom_swap_counts() {
    let mut env = make_env().1;
    eval_src("(def a (shared-atom 0))", &mut env);
    eval_src("(swap! a inc)", &mut env);
    eval_src("(swap! a inc)", &mut env);
    assert_eq!(eval_src("(swap! a + 10)", &mut env), Value::Long(12));
    assert_eq!(eval_src("@a", &mut env), Value::Long(12));
}

#[test]
fn shared_atom_compare_and_set() {
    let mut env = make_env().1;
    eval_src("(def a (shared-atom 5))", &mut env);
    // Mismatched expected value: no change.
    assert_eq!(
        eval_src("(compare-and-set! a 4 100)", &mut env),
        Value::Bool(false)
    );
    assert_eq!(eval_src("@a", &mut env), Value::Long(5));
    // Matching expected value: swaps.
    assert_eq!(
        eval_src("(compare-and-set! a 5 100)", &mut env),
        Value::Bool(true)
    );
    assert_eq!(eval_src("@a", &mut env), Value::Long(100));
}

#[test]
fn shared_atom_promotes_strings_and_keywords() {
    let mut env = make_env().1;
    eval_src("(def a (shared-atom \"hi\"))", &mut env);
    assert_eq!(eval_src("@a", &mut env), Value::string("hi".to_string()));
    // Keyword identity survives the promote/demote round-trip by content.
    eval_src("(reset! a :foo)", &mut env);
    assert_eq!(eval_src("(= @a :foo)", &mut env), Value::Bool(true));
}

#[test]
fn shared_atom_rejects_non_promotable() {
    // A closure captures isolate-local state and cannot be published.
    let (_, mut env) = make_env();
    let mut parser = Parser::new("(shared-atom (fn [] 1))".to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_interp::eval::eval(&form, &mut env);
    }
    assert!(last.is_err(), "publishing a closure should fail");
}

#[test]
fn shared_atom_swap_rejects_non_promotable_result() {
    let mut env = make_env().1;
    eval_src("(def a (shared-atom 0))", &mut env);
    let mut parser = Parser::new(
        "(swap! a (fn [_] (fn [] 1)))".to_string(),
        "<test>".to_string(),
    );
    let forms = parser.parse_all().expect("parse error");
    let res = cljrs_interp::eval::eval(&forms[0], &mut env);
    assert!(res.is_err(), "swapping in a closure should fail");
    // The atom is unchanged after the failed swap.
    assert_eq!(eval_src("@a", &mut env), Value::Long(0));
}
