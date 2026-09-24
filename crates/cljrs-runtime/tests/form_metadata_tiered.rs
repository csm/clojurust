//! Reader metadata answers the same in every execution tier.
//!
//! `ExecutionMode::Tiered` is the default and what the CLI uses, so a rule the
//! tree-walker applies and IR lowering does not makes `meta` depend on how hot
//! the code got: `(defn f [] (meta ^{:a 1} [1]))` answered `{:a 1}` cold and
//! `nil` once promoted, and AOT — which lowers through the same path — answered
//! `nil` unconditionally.
//!
//! Every case here runs the *same expression* twice: once tree-walked, once
//! through the IR interpreter, and asserts the two agree. Lowering is forced at
//! definition time rather than waited for, and [`assert_actually_lowered`]
//! fails the test if the IR tier never ran — an agreement between two
//! tree-walks would otherwise pass while proving nothing.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env(mode: cljrs_runtime::ExecutionMode) -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_all(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::tiered::eval(&form, env)
            .unwrap_or_else(|e| panic!("{src}\neval: {e:?}"));
    }
    result
}

/// The IR the tier-1 interpreter would run for `user/<name>`'s only arity.
///
/// `None` means the function was never lowered, so a tier comparison against it
/// is vacuous.
fn lowered_ir(globals: &Arc<GlobalEnv>, name: &str) -> Option<Arc<cljrs_ir::IrFunction>> {
    let var = globals.lookup_var_in_ns("user", name)?;
    let value = var.get().value.lock().unwrap().clone()?;
    let Value::Fn(f) = value.unwrap_meta() else {
        return None;
    };
    let id = f.get().arities.first()?.ir_arity_id;
    globals.ir_cache().get(id)
}

fn assert_actually_lowered(globals: &Arc<GlobalEnv>, name: &str, expr: &str) {
    assert!(
        lowered_ir(globals, name).is_some(),
        "`{expr}` never reached the IR tier — the tier comparison would be vacuous"
    );
}

/// Evaluate `expr` inside a function body in both tiers and assert they agree
/// on `expected`.
fn assert_same_in_every_tier(expr: &str, expected: &str) {
    let defn = format!("(defn probe [] (pr-str {expr}))");

    let (_g, mut tree_walk) = make_env(cljrs_runtime::ExecutionMode::TreeWalk);
    eval_all(&defn, &mut tree_walk);
    let walked = eval_all("(probe)", &mut tree_walk);

    // Eager lowering makes the IR tier deterministic: no waiting on the
    // background worker, no threshold to cross.
    cljrs_runtime::tiered::force_eager_lowering();
    let (globals, mut tiered) = make_env(cljrs_runtime::ExecutionMode::Tiered);
    eval_all(&defn, &mut tiered);
    let lowered = eval_all("(probe)", &mut tiered);
    assert_actually_lowered(&globals, "probe", expr);

    let walked = pr_str_content(&walked, expr);
    let lowered = pr_str_content(&lowered, expr);
    assert_eq!(
        walked, lowered,
        "`{expr}` disagreed between tiers: tree-walk {walked}, IR {lowered}"
    );
    assert_eq!(walked, expected, "`{expr}` in tree-walk");
}

/// Evaluate `expr` inside a function body in both tiers, asserting the IR tier
/// *refused* to lower it and that the results agree on `expected`.
///
/// For a form only the tree-walker can build faithfully (an anonymous
/// `^:async` fn), agreement comes from staying off the IR tier; a body that
/// did lower would build a synchronous closure.
fn assert_kept_on_tree_walker(expr: &str, expected: &str) {
    let defn = format!("(defn probe [] (pr-str {expr}))");

    let (_g, mut tree_walk) = make_env(cljrs_runtime::ExecutionMode::TreeWalk);
    eval_all(&defn, &mut tree_walk);
    let walked = pr_str_content(&eval_all("(probe)", &mut tree_walk), expr);

    cljrs_runtime::tiered::force_eager_lowering();
    let (globals, mut tiered) = make_env(cljrs_runtime::ExecutionMode::Tiered);
    eval_all(&defn, &mut tiered);
    let tiered_result = pr_str_content(&eval_all("(probe)", &mut tiered), expr);
    assert!(
        lowered_ir(&globals, "probe").is_none(),
        "`{expr}` was lowered — its closure would be synchronous once promoted"
    );

    assert_eq!(walked, tiered_result, "`{expr}` disagreed between tiers");
    assert_eq!(walked, expected, "`{expr}` in tree-walk");
}

/// The probe returns `(pr-str …)`, so its result is always a string; compare
/// its contents rather than its own printed form.
fn pr_str_content(value: &Value, expr: &str) -> String {
    match value {
        Value::Str(s) => s.get().to_string(),
        other => panic!("`{expr}` probe returned a non-string: {other:?}"),
    }
}

// ── The reported repro ──────────────────────────────────────────────────────

#[test]
fn a_vector_literal_keeps_its_annotation_once_promoted() {
    assert_same_in_every_tier("(meta ^{:a 1} [1])", "{:a 1}");
}

#[test]
fn every_collection_literal_keeps_its_annotation() {
    assert_same_in_every_tier("(meta ^{:a 1} {:k 1})", "{:a 1}");
    assert_same_in_every_tier("(meta ^{:a 1} #{1})", "{:a 1}");
}

#[test]
fn a_fn_keeps_its_annotation() {
    assert_same_in_every_tier("(meta ^{:a 1} (fn [] 1))", "{:a 1}");
}

#[test]
fn shorthand_annotations_expand_in_both_tiers() {
    assert_same_in_every_tier("(meta ^:dyn [1])", "{:dyn true}");
    assert_same_in_every_tier("(meta ^String [1])", "{:tag String}");
    assert_same_in_every_tier("(meta ^\"String\" [1])", "{:tag \"String\"}");
}

#[test]
fn stacked_annotations_merge_in_both_tiers() {
    assert_same_in_every_tier("(meta ^:a ^:b [1])", "{:b true, :a true}");
    assert_same_in_every_tier("(meta ^{:a 1} ^{:a 2} [1])", "{:a 1}");
}

#[test]
fn an_evaluated_annotation_sees_the_enclosing_scope_in_both_tiers() {
    assert_same_in_every_tier("(let [x 5] (meta ^{:x x} [1]))", "{:x 5}");
}

// ── The quoted position ─────────────────────────────────────────────────────

#[test]
fn quote_keeps_the_annotation_as_data_in_both_tiers() {
    // Inside `quote` the annotation is data in every tier: `form_to_value` is
    // shared, and IR lowering routes a quoted form through `lower_quote`.
    assert_same_in_every_tier("(meta '^{:a 1} [1])", "{:a 1}");
    assert_same_in_every_tier("(meta '^:dyn sym)", "{:dyn true}");
    assert_same_in_every_tier("(meta '^{:x (+ 1 2)} [1])", "{:x (+ 1 2)}");
}

#[test]
fn an_empty_list_keeps_its_annotation() {
    // `()` is the empty-list literal, not a call, so it attaches like `[]` —
    // and like the quoted position already did.
    assert_same_in_every_tier("(meta ^{:a 1} ())", "{:a 1}");
    assert_same_in_every_tier("(meta '^{:a 1} ())", "{:a 1}");
    assert_same_in_every_tier("^{:a 1} ()", "()");
}

// ── `^:async (fn …)` ─────────────────────────────────────────────────────────

#[test]
fn an_async_fn_keeps_its_annotation_in_every_tier() {
    // Issue #359: the IR tier consumed `:async` and dropped the annotation, so
    // this answered `{:async true}` cold and `nil` once promoted.
    assert_kept_on_tree_walker("(meta ^:async (fn [] 1))", "{:async true}");
    assert_kept_on_tree_walker("(meta ^:async ^:a (fn [] 1))", "{:a true, :async true}");
    assert_kept_on_tree_walker("(meta (fn ^:async [] 1))", "nil");
}

#[test]
fn a_false_async_annotation_is_plain_metadata() {
    assert_same_in_every_tier("(meta ^{:async false} (fn [] 1))", "{:async false}");
}

// ── Forms that take the annotation as a hint ────────────────────────────────

#[test]
fn a_call_carries_no_annotation_in_either_tier() {
    assert_same_in_every_tier("(meta ^{:a 1} (list 1))", "nil");
}

#[test]
fn a_local_carries_no_annotation_in_either_tier() {
    assert_same_in_every_tier("(let [x [1]] (meta ^{:a 1} x))", "nil");
}

#[test]
fn a_scalar_carries_no_annotation_in_either_tier() {
    assert_same_in_every_tier("(meta ^{:a 1} 42)", "nil");
}

// ── The annotated value itself is unchanged ─────────────────────────────────

#[test]
fn the_annotated_value_is_unchanged_in_both_tiers() {
    assert_same_in_every_tier("^{:a 1} [1 2]", "[1 2]");
    assert_same_in_every_tier("(= [1] ^{:a 1} [1])", "true");
    assert_same_in_every_tier("(count ^{:a 1} [1 2])", "2");
    assert_same_in_every_tier("(type ^{:a 1} [1])", "Vector");
}

// ── Predicates see through an annotation in both tiers ──────────────────────

#[test]
fn type_predicates_see_through_an_annotation_in_both_tiers() {
    assert_same_in_every_tier("(vector? ^{:a 1} [1])", "true");
    assert_same_in_every_tier("(map? ^{:a 1} {})", "true");
    assert_same_in_every_tier("(seq? (with-meta '(1 2) {:a 1}))", "true");
    assert_same_in_every_tier("(symbol? (with-meta 'x {:a 1}))", "true");
    assert_same_in_every_tier("(string? (with-meta \"s\" {:a 1}))", "true");
    assert_same_in_every_tier("(keyword? (with-meta :k {:a 1}))", "true");
    assert_same_in_every_tier("(number? (with-meta 1 {:a 1}))", "true");
    assert_same_in_every_tier("(nil? (with-meta nil {:a 1}))", "true");
}
