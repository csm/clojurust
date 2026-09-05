//! Regression tests pinning PR #356's intent: `deftype` is a real named type.
//!
//! `deftype` used to be a special form wired to a "not implemented" error. The
//! feature landed a positional `->T` constructor, protocol/interface method
//! bodies with the fields in scope, `.-field` access, and mutable fields
//! (`^:unsynchronized-mutable` / `^:volatile-mutable`) writable with `set!`.
//!
//! The implementation was then silently lost across a `main` merge: the
//! conflict resolution kept the old stub alongside a truncated copy of the new
//! `eval_deftype`, which is a compile error the moment anything builds the
//! crate. These tests exist so the *behaviour* — not just compilation — is
//! pinned the next time the branch takes a merge.

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
        // The type name, not `{:?}`: a `Value` may be a `Uuid`, and CodeQL
        // reads Debug-formatting one into a panic as logging it in cleartext.
        // Which type came back instead of a string is what this assertion is
        // actually about.
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

/// Evaluate `src`, expecting it to fail, and return the error rendered.
fn eval_err(src: &str) -> String {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
        if last.is_err() {
            break;
        }
    }
    match last {
        Err(e) => format!("{e:?}"),
        Ok(v) => panic!("expected an error, got a {}", v.type_name()),
    }
}

// ── The type itself ──────────────────────────────────────────────────────────

#[test]
fn deftype_is_implemented() {
    // The old stub errored with "deftype is not implemented"; the whole point
    // of the feature is that this evaluates.
    assert_eq!(eval_pr("(deftype T [x y]) (pr-str (.-x (->T 1 2)))"), "1");
}

#[test]
fn positional_constructor_binds_fields_in_order() {
    assert_eq!(
        eval_pr("(deftype Point [x y]) (let [p (->Point 3 4)] (pr-str [(.-x p) (.-y p)]))"),
        "[3 4]"
    );
}

#[test]
fn type_name_is_interned_so_instance_resolves() {
    assert_eq!(
        eval_pr("(deftype T [x]) (pr-str (instance? T (->T 1)))"),
        "true"
    );
}

#[test]
fn deftype_has_no_map_constructor() {
    // Clojure reserves `map->T` for defrecord; deftype must not generate one.
    assert!(
        eval_err("(deftype T [x]) (map->T {:x 1})").contains("map->T"),
        "expected map->T to be unresolvable for a deftype"
    );
}

#[test]
fn a_field_vector_may_carry_its_own_metadata() {
    // `as_vector` reports the shape under any `^meta`, so a marker on the field
    // vector is transparent — as it is for defrecord.
    assert_eq!(
        eval_pr("(deftype T ^:marker [x]) (pr-str (.-x (->T 7)))"),
        "7"
    );
}

// ── Protocol method bodies ───────────────────────────────────────────────────

#[test]
fn immutable_fields_are_in_scope_in_a_method_body() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (describe [this]))
             (deftype T [a b] P (describe [_] [a b]))
             (pr-str (describe (->T 1 2)))"
        ),
        "[1 2]"
    );
}

#[test]
fn one_type_can_implement_several_protocols() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (p-of [this]))
             (defprotocol Q (q-of [this]))
             (deftype T [a] P (p-of [_] (* a 10)) Q (q-of [_] (* a 100)))
             (let [t (->T 2)] (pr-str [(p-of t) (q-of t)]))"
        ),
        "[20 200]"
    );
}

// ── Mutable fields ───────────────────────────────────────────────────────────

#[test]
fn set_bang_on_a_bare_field_name_inside_a_method() {
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump [this]) (peek-n [this]))
             (deftype C [^:unsynchronized-mutable n]
               Counter
               (bump [_] (set! n (inc n)))
               (peek-n [_] n))
             (let [c (->C 0)] (bump c) (bump c) (pr-str (peek-n c)))"
        ),
        "2"
    );
}

#[test]
fn a_write_is_visible_to_a_later_read_in_the_same_method() {
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump-twice [this]))
             (deftype C [^:unsynchronized-mutable n]
               Counter
               (bump-twice [_] (set! n (inc n)) (set! n (inc n)) n))
             (pr-str (bump-twice (->C 5)))"
        ),
        "7"
    );
}

#[test]
fn volatile_mutable_behaves_like_unsynchronized_mutable() {
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump [this]) (peek-n [this]))
             (deftype C [^:volatile-mutable n]
               Counter
               (bump [_] (set! n (inc n)))
               (peek-n [_] n))
             (let [c (->C 41)] (bump c) (pr-str (peek-n c)))"
        ),
        "42"
    );
}

#[test]
fn set_bang_on_an_explicit_field_target() {
    assert_eq!(
        eval_pr(
            "(deftype Box [^:unsynchronized-mutable v])
             (let [b (->Box :old)] (set! (.-v b) :new) (pr-str (.-v b)))"
        ),
        ":new"
    );
}

#[test]
fn mutating_one_instance_does_not_touch_another() {
    assert_eq!(
        eval_pr(
            "(deftype Box [^:unsynchronized-mutable v])
             (let [a (->Box 1) b (->Box 1)]
               (set! (.-v a) 99)
               (pr-str [(.-v a) (.-v b)]))"
        ),
        "[99 1]"
    );
}

#[test]
fn immutable_and_mutable_fields_coexist() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (report [this]))
             (deftype T [label ^:unsynchronized-mutable n]
               P
               (report [_] (set! n (inc n)) [label n]))
             (let [t (->T \"hits\" 0)] (report t) (pr-str (report t)))"
        ),
        "[\"hits\" 2]"
    );
}

#[test]
fn a_hot_mutable_field_method_accumulates_every_write() {
    // Tree-walk only — `deftype_mutable_tiered.rs` runs the same script with IR
    // lowering forced on, which is where the `set!`-on-a-local decline matters.
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump [this]) (peek-n [this]))
             (deftype C [^:unsynchronized-mutable n]
               Counter
               (bump [_] (set! n (inc n)))
               (peek-n [_] n))
             (let [c (->C 0)]
               (dotimes [_ 500] (bump c))
               (pr-str (peek-n c)))"
        ),
        "500"
    );
}

#[test]
fn set_bang_rejects_a_field_the_type_did_not_declare_mutable() {
    let err = eval_err("(deftype T [^:unsynchronized-mutable a b]) (set! (.-b (->T 1 2)) 3)");
    assert!(
        err.contains("not a mutable field"),
        "expected a mutable-field error, got {err}"
    );
}

// ── set! still means what it meant ───────────────────────────────────────────

#[test]
fn set_bang_on_a_dynamic_var_is_unaffected() {
    assert_eq!(
        eval_pr(
            "(def ^:dynamic *v* 1)
             (binding [*v* 2] (set! *v* 3) (pr-str *v*))"
        ),
        "3"
    );
}
