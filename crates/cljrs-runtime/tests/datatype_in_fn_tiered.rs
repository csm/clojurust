//! `deftype`/`defrecord`/`reify` written *inside* a function body must keep
//! working once that function is IR-promoted.
//!
//! Each of the three is a macro over the `deftype*` primitive, and lowering
//! runs on the **expanded** body — so the name the ANF lowerer must decline on
//! is `deftype*`, not the surface name. Miss it and the form lowers as a
//! generic call, resolves nothing, and the function silently returns `nil`
//! (or fails with `var not found user/deftype*`) only after the tier flips.
//!
//! Nothing pins that from a tree-walking test: with lowering off, the decline
//! is unreachable. This file is its own binary so it can flip the process-wide
//! eager-lowering switch, and drives each function far past the warm threshold
//! so the tier is genuinely entered.

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
fn a_hot_fn_returning_a_reify_keeps_its_captured_locals() {
    // The idiomatic case: `reify` in a function body, closing over a parameter.
    assert_eq!(
        eval_pr(
            "(defprotocol P (pval [x]))
             (defn mk [n] (pval (reify P (pval [_] n))))
             (pr-str (loop [i 0 acc 0]
                       (if (< i 2000) (recur (inc i) (+ acc (mk 1))) acc)))"
        ),
        "2000"
    );
}

#[test]
fn each_evaluation_of_a_hot_reify_is_a_fresh_instance() {
    // One anonymous type per *form*, one instance per *evaluation*: two calls
    // with different arguments must not share captured state.
    assert_eq!(
        eval_pr(
            "(defprotocol P (pval [x]))
             (defn mk [n] (reify P (pval [_] n)))
             (dotimes [_ 2000] (mk 0))
             (pr-str [(pval (mk 1)) (pval (mk 2))])"
        ),
        "[1 2]"
    );
}

#[test]
fn a_hot_fn_defining_a_deftype_still_constructs_it() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (pval [x]))
             (defn mk [n]
               (deftype Box [v] P (pval [_] v))
               (pval (->Box n)))
             (pr-str (loop [i 0 acc 0]
                       (if (< i 2000) (recur (inc i) (+ acc (mk 1))) acc)))"
        ),
        "2000"
    );
}

#[test]
fn a_hot_fn_defining_a_defrecord_still_constructs_it() {
    assert_eq!(
        eval_pr(
            "(defprotocol P (pval [x]))
             (defn mk [n]
               (defrecord Pt [v] P (pval [_] v))
               (pval (->Pt n)))
             (pr-str (loop [i 0 acc 0]
                       (if (< i 2000) (recur (inc i) (+ acc (mk 1))) acc)))"
        ),
        "2000"
    );
}
