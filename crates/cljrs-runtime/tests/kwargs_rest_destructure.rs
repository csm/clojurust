//! Issue #368 — a map-shaped rest pattern (`& {:keys [...]}`) is Clojure's
//! keyword-argument convention, in every tier.
//!
//! A rest parameter holds the trailing arguments as a *list*.  When its
//! destructuring pattern is a map, the list is first folded into a map, so
//! `(f :a 1)` binds `a` to `1`.  The tree-walker does that in `bind_fn_params`;
//! the IR lowerer used to hand the pattern the list itself, so every key
//! lowered to `(get '(:a 1) :a)` — nil.  A kwargs function therefore started
//! answering `nil` the moment it tiered up.
//!
//! A sequential rest pattern (`& [a b]`) really does destructure the list, and
//! must keep doing so; it is pinned here alongside.
//!
//! This file flips the process-wide eager-lowering switch, so it lives in its
//! own test binary.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env(mode: cljrs_runtime::ExecutionMode) -> (Arc<GlobalEnv>, Env) {
    // Process-wide, and the reason this test is its own binary.
    cljrs_runtime::tiered::force_eager_lowering();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_pr(mode: cljrs_runtime::ExecutionMode, src: &str) -> String {
    let (_globals, mut env) = make_env(mode);
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        // The type name, not `{:?}`: which type came back instead of a string
        // is what the assertion is about, and a `Value` may hold a secret.
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

/// Run `src` through the tree-walker and through the IR tier and assert both
/// produce `expected` — the divergence in #368 is exactly a disagreement here.
fn assert_both_tiers(src: &str, expected: &str) {
    assert_eq!(
        eval_pr(cljrs_runtime::ExecutionMode::TreeWalk, src),
        expected,
        "tree-walking interpreter"
    );
    assert_eq!(
        eval_pr(cljrs_runtime::ExecutionMode::TieredNoJit, src),
        expected,
        "IR tier"
    );
}

#[test]
fn keyword_arguments_bind_in_both_tiers() {
    assert_both_tiers(
        "(defn f [& {:keys [a b]}] [a b])
         (pr-str (f :a 1 :b 2))",
        "[1 2]",
    );
}

#[test]
fn keyword_arguments_bind_after_fixed_parameters() {
    assert_both_tiers(
        "(defn f [x & {:keys [a]}] [x a])
         (pr-str (f 0 :a 3))",
        "[0 3]",
    );
}

#[test]
fn a_supplied_key_wins_over_its_or_default() {
    // The `:or` default made the miss look like "the caller passed nothing":
    // `b` was right while `a` was nil, so the bug read as a caller error.
    assert_both_tiers(
        "(defn f [& {:keys [a b] :or {b 5}}] [a b])
         (pr-str [(f :a 1) (f :a 1 :b 9) (f)])",
        "[[1 5] [1 9] [nil 5]]",
    );
}

#[test]
fn keyword_arguments_bind_in_an_anonymous_fn() {
    // Inner `fn*` arities lower through `lower_fn_arity`, a separate path from
    // a `defn`'s arity — both have to perform the conversion.
    assert_both_tiers(
        "(def f (fn [& {:keys [a]}] a))
         (pr-str (f :a 9))",
        "9",
    );
}

#[test]
fn keyword_arguments_survive_tiering_up_mid_run() {
    // The shape the divergence actually took: a function answers correctly for
    // its first calls and switches to nil once it is lowered.
    assert_both_tiers(
        "(defn f [& {:keys [a]}] a)
         (pr-str (into #{} (map (fn [_] (f :a 1)) (range 500))))",
        "#{1}",
    );
}

#[test]
fn a_trailing_map_is_accepted_as_keyword_arguments() {
    // Clojure accepts a map in place of the pairs, or after them; a later
    // entry wins, exactly as a repeated key does.  The tree-walker used to
    // panic on the odd argument count instead.
    assert_both_tiers(
        "(defn f [& {:keys [a b] :or {b 5}}] [a b])
         (pr-str [(f {:a 1}) (f :a 1 {:b 2}) (f :a 1 {:a 2}) (f nil)])",
        "[[1 5] [1 2] [2 5] [nil 5]]",
    );
}

#[test]
fn a_key_with_no_value_is_an_error_in_both_tiers() {
    assert_both_tiers(
        "(defn f [& {:keys [a]}] a)
         (pr-str (try (f :a 1 :b) (catch Exception e (ex-message e))))",
        "\"No value supplied for key: :b\"",
    );
}

#[test]
fn a_sequential_rest_pattern_still_destructures_the_list() {
    // `& [a b]` binds positionally: the rest value really is a seq there, and
    // must not be folded into a map.
    assert_both_tiers(
        "(defn f [& [a b]] [a b])
         (pr-str [(f 1 2) (f :a 1)])",
        "[[1 2] [:a 1]]",
    );
}

#[test]
fn a_plain_rest_parameter_still_binds_the_list() {
    assert_both_tiers(
        "(defn f [& xs] xs)
         (pr-str (f :a 1))",
        "(:a 1)",
    );
}

#[test]
fn a_kwargs_arity_coexists_with_other_arities() {
    assert_both_tiers(
        "(defn f ([] :none) ([x] x) ([x & {:keys [a]}] [x a]))
         (pr-str [(f) (f 1) (f 1 :a 2)])",
        "[:none 1 [1 2]]",
    );
}
