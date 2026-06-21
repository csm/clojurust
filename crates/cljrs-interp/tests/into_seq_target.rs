//! Regression tests for issue #198: `into` must accept a lazy-seq or cons as
//! its target collection (the first argument), just as `conj` does.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_fresh(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

// ── into with lazy-seq target ────────────────────────────────────────────────

#[test]
fn into_lazy_seq_target_produces_cons() {
    // (into (map inc [1 2]) [9]) => (9 2 3)
    let result = eval_fresh("(into (map inc [1 2]) [9])");
    // Result must be seqable and contain the right elements.
    // The exact representation is Cons; verify via seq conversion.
    let elems = eval_fresh("(vec (into (map inc [1 2]) [9]))");
    if let Value::Vector(v) = elems {
        let v = v.get();
        assert_eq!(v.count(), 3, "expected 3 elements");
        assert_eq!(v.nth(0), Some(&Value::Long(9)));
        assert_eq!(v.nth(1), Some(&Value::Long(2)));
        assert_eq!(v.nth(2), Some(&Value::Long(3)));
    } else {
        panic!("expected Vector after (vec ...), got {:?}", elems);
    }
    // Ensure it doesn't throw (the original bug returned an error)
    assert!(
        !matches!(result, Value::Nil),
        "into should not return nil for a non-empty lazy-seq target"
    );
}

#[test]
fn into_lazy_seq_target_multiple_items() {
    // (into (map inc [10 20]) [1 2 3]) => (3 2 1 11 21)
    let elems = eval_fresh("(vec (into (map inc [10 20]) [1 2 3]))");
    if let Value::Vector(v) = elems {
        let v = v.get();
        assert_eq!(v.count(), 5);
        // items are prepended one by one: 1 first, then 2, then 3
        assert_eq!(v.nth(0), Some(&Value::Long(3)));
        assert_eq!(v.nth(1), Some(&Value::Long(2)));
        assert_eq!(v.nth(2), Some(&Value::Long(1)));
        assert_eq!(v.nth(3), Some(&Value::Long(11)));
        assert_eq!(v.nth(4), Some(&Value::Long(21)));
    } else {
        panic!("expected Vector, got {:?}", elems);
    }
}

#[test]
fn into_lazy_seq_target_empty_source() {
    // (into (map inc [1 2]) []) => (2 3) — unchanged lazy-seq
    let cnt = eval_fresh("(count (into (map inc [1 2]) []))");
    assert_eq!(cnt, Value::Long(2));
}

// ── into with cons target ────────────────────────────────────────────────────

#[test]
fn into_cons_target_produces_cons() {
    // (into (seq [1 2]) [9]) => (9 1 2)
    let elems = eval_fresh("(vec (into (seq [1 2]) [9]))");
    if let Value::Vector(v) = elems {
        let v = v.get();
        assert_eq!(v.count(), 3);
        assert_eq!(v.nth(0), Some(&Value::Long(9)));
        assert_eq!(v.nth(1), Some(&Value::Long(1)));
        assert_eq!(v.nth(2), Some(&Value::Long(2)));
    } else {
        panic!("expected Vector, got {:?}", elems);
    }
}

// ── into with lazy-seq as SOURCE still works (regression guard) ──────────────

#[test]
fn into_lazy_seq_as_source_still_works() {
    // (into [] (map inc [1 2])) => [2 3]  — lazy seq as source is unaffected
    let result = eval_fresh("(into [] (map inc [1 2]))");
    if let Value::Vector(v) = result {
        let v = v.get();
        assert_eq!(v.count(), 2);
        assert_eq!(v.nth(0), Some(&Value::Long(2)));
        assert_eq!(v.nth(1), Some(&Value::Long(3)));
    } else {
        panic!("expected Vector, got {:?}", result);
    }
}

// ── into with list target is still correct ────────────────────────────────────

#[test]
fn into_list_target_still_correct() {
    let elems = eval_fresh("(vec (into '(1 2) [9]))");
    if let Value::Vector(v) = elems {
        let v = v.get();
        assert_eq!(v.count(), 3);
        assert_eq!(v.nth(0), Some(&Value::Long(9)));
        assert_eq!(v.nth(1), Some(&Value::Long(1)));
        assert_eq!(v.nth(2), Some(&Value::Long(2)));
    } else {
        panic!("expected Vector, got {:?}", elems);
    }
}
