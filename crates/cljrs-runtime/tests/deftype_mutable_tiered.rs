//! PR #356's tiered-safety half: the IR lowerer must **decline** to lower a
//! `set!` whose target is a local binding.
//!
//! A `deftype` method body binds each field as a `let*` local, so a mutable
//! field write reads as `(set! n (inc n))` over a local. The IR var-store path
//! cannot express that — it would emit a store to the global var `n` and lose
//! the write — so `lower_set_bang` returns `UnsupportedForm` and the method
//! tree-walks, where `eval_set_bang` updates the instance's interior cell.
//!
//! Nothing pins that from a tree-walking test: with lowering off, the decline
//! is unreachable. This file lives on its own so it can flip the process-wide
//! eager-lowering switch without disturbing any other test binary, and drives
//! each method far past the warm threshold so the tier is genuinely entered.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    // Process-wide, and the reason this test is its own binary.
    cljrs_runtime::tiered::force_eager_lowering();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TieredNoJit)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

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

#[test]
fn a_hot_mutable_field_method_keeps_every_write() {
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump [this]) (peek-n [this]))
             (deftype C [^:unsynchronized-mutable n]
               Counter
               (bump [_] (set! n (inc n)))
               (peek-n [_] n))
             (let [c (->C 0)]
               (dotimes [_ 1000] (bump c))
               (pr-str (peek-n c)))"
        ),
        "1000"
    );
}

#[test]
fn a_hot_method_does_not_leak_the_write_to_a_global_var() {
    // The failure mode the decline exists to prevent: lowering `(set! n ...)`
    // as a var store would define/overwrite `user/n` and leave the instance
    // untouched. `n` must still be unresolvable afterwards.
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump [this]))
             (deftype C [^:unsynchronized-mutable n] Counter (bump [_] (set! n (inc n))))
             (let [c (->C 0)] (dotimes [_ 1000] (bump c)))
             (pr-str (resolve 'n))"
        ),
        "nil"
    );
}

#[test]
fn a_hot_read_after_write_within_one_method_is_consistent() {
    assert_eq!(
        eval_pr(
            "(defprotocol Counter (bump-twice [this]))
             (deftype C [^:unsynchronized-mutable n]
               Counter
               (bump-twice [_] (set! n (inc n)) (set! n (inc n)) n))
             (let [c (->C 0)]
               (pr-str (last (map (fn [_] (bump-twice c)) (range 500)))))"
        ),
        "1000"
    );
}

#[test]
fn a_hot_immutable_deftype_method_still_lowers_and_agrees() {
    // The control: an immutable-field method has no `set!` to decline on, so
    // it may lower freely and must produce the same answers.
    assert_eq!(
        eval_pr(
            "(defprotocol P (scale [this k]))
             (deftype T [a] P (scale [_ k] (* a k)))
             (let [t (->T 3)]
               (pr-str (reduce + (map (fn [i] (scale t i)) (range 100)))))"
        ),
        "14850"
    );
}
