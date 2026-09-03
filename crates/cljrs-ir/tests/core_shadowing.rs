//! Regression tests for issue #337: a call-position symbol is lowered as a
//! `clojure.core` builtin only when it actually resolves to core.
//!
//! Before the fix `lower_call` matched on the head symbol's *text*, so a
//! `let`-bound `inc`, a parameter named `inc`, a locally-`def`ed `inc` and even
//! an explicitly qualified `other.ns/count` were inlined (or turned into
//! `CallKnown`) with core's semantics — while the tree-walking interpreter
//! called the real binding.  The answer then depended on which tier was
//! running, changing mid-run at the promotion boundary.

use std::sync::Arc;

use cljrs_ir::lower::{CoreShadows, lower_fn_body, lower_fn_body_shadowed};
use cljrs_ir::{Inst, IrFunction, KnownFn};
use cljrs_reader::Parser;

fn parse(source: &str) -> Vec<cljrs_reader::Form> {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    parser.parse_all().expect("parse")
}

fn lower(source: &str) -> IrFunction {
    lower_fn_body(Some("test"), "user", &[], &parse(source), false).expect("lower")
}

fn lower_in(ns: &str, params: &[&str], source: &str, shadows: &CoreShadows) -> IrFunction {
    let params: Vec<Arc<str>> = params.iter().map(|p| Arc::from(*p)).collect();
    lower_fn_body_shadowed(
        Some("test"),
        ns,
        &params,
        &[],
        None,
        &parse(source),
        false,
        shadows,
    )
    .expect("lower")
}

/// Every instruction in `ir` and its subfunctions.
fn insts(ir: &IrFunction) -> Vec<Inst> {
    let mut out = Vec::new();
    for block in &ir.blocks {
        out.extend(block.phis.iter().cloned());
        out.extend(block.insts.iter().cloned());
    }
    for sub in &ir.subfunctions {
        out.extend(insts(sub));
    }
    out
}

fn uses_known(ir: &IrFunction, kf: KnownFn) -> bool {
    insts(ir)
        .iter()
        .any(|i| matches!(i, Inst::CallKnown(_, k, _) if *k == kf))
}

/// Like [`uses_known`], but only in `ir`'s own blocks — a call inside a nested
/// `fn*` body lives in a subfunction.
fn body_uses_known(ir: &IrFunction, kf: KnownFn) -> bool {
    ir.blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .any(|i| matches!(i, Inst::CallKnown(_, k, _) if *k == kf))
}

fn has_dynamic_call(ir: &IrFunction) -> bool {
    insts(ir).iter().any(|i| matches!(i, Inst::Call(..)))
}

// ── Locals ──────────────────────────────────────────────────────────────────

#[test]
fn let_bound_name_is_not_core() {
    // Repro 1: `(let [inc (fn [x] (* x 100))] (inc n))` must call the local.
    let ir = lower_in(
        "shadow.test",
        &["n"],
        "(let [inc (fn [x] (* x 100))] (inc n))",
        &CoreShadows::none(),
    );
    assert!(
        !uses_known(&ir, KnownFn::Add),
        "the let-bound `inc` was inlined as core's `inc`"
    );
    assert!(
        has_dynamic_call(&ir),
        "expected a dynamic call to the local"
    );
}

#[test]
fn parameter_name_is_not_core() {
    // Repro 2: `inc` is just a parameter here.
    let ir = lower_in(
        "shadow.two",
        &["inc", "n"],
        "(inc (inc n))",
        &CoreShadows::none(),
    );
    assert!(
        !uses_known(&ir, KnownFn::Add),
        "the parameter `inc` was inlined as core's `inc`"
    );
    assert!(has_dynamic_call(&ir));
}

#[test]
fn known_fn_name_bound_as_a_local_is_not_core() {
    // The same for a `KnownFn` (rather than an inline expansion).
    let ir = lower_in(
        "shadow.known",
        &["coll"],
        "(let [count (fn [c] :local)] (count coll))",
        &CoreShadows::none(),
    );
    assert!(!uses_known(&ir, KnownFn::Count));
    assert!(has_dynamic_call(&ir));
}

#[test]
fn a_local_does_not_shadow_an_explicitly_qualified_core_call() {
    let ir = lower_in(
        "shadow.test",
        &["n"],
        "(let [inc (fn [x] (* x 100))] (clojure.core/inc n))",
        &CoreShadows::none(),
    );
    assert!(
        uses_known(&ir, KnownFn::Add),
        "`clojure.core/inc` is core's"
    );
}

#[test]
fn locals_only_shadow_inside_their_scope() {
    let ir = lower_in(
        "shadow.test",
        &["n"],
        "(do (let [inc (fn [x] x)] (inc n)) (inc n))",
        &CoreShadows::none(),
    );
    // The second `(inc n)` is outside the `let`, so it is core's.
    assert!(uses_known(&ir, KnownFn::Add));
    assert!(has_dynamic_call(&ir));
}

// ── Namespace-level shadowing ───────────────────────────────────────────────

#[test]
fn namespace_shadow_is_not_core() {
    let shadows = CoreShadows::new([Arc::from("count")]);
    let ir = lower_in("my.ns", &["coll"], "(count coll)", &shadows);
    assert!(
        !uses_known(&ir, KnownFn::Count),
        "a name the namespace binds elsewhere was lowered as core's"
    );
    assert!(has_dynamic_call(&ir));
    // Unshadowed names in the same namespace are unaffected.
    let ir = lower_in("my.ns", &["coll"], "(first coll)", &shadows);
    assert!(uses_known(&ir, KnownFn::First));
}

#[test]
fn a_namespace_shadow_does_not_reach_a_qualified_core_call() {
    let shadows = CoreShadows::new([Arc::from("count")]);
    let ir = lower_in("my.ns", &["coll"], "(clojure.core/count coll)", &shadows);
    assert!(uses_known(&ir, KnownFn::Count));
}

#[test]
fn a_def_in_the_lowered_unit_shadows_core() {
    // Repro 3: AOT compiles the whole file without evaluating its `def`s, so
    // the shadow has to be read off the forms themselves.
    let ir = lower_fn_body(
        Some("__cljrs_main"),
        "x",
        &[],
        &parse("(def inc (fn [n] (+ n 7))) (inc 1)"),
        false,
    )
    .expect("lower");
    // `(+ n 7)` lives in the defined function; `(inc 1)` is in the body here.
    assert!(
        !body_uses_known(&ir, KnownFn::Add),
        "the locally-defined `inc` was inlined as core's"
    );
    assert!(
        has_dynamic_call(&ir),
        "expected `(inc 1)` to become a dynamic call to x/inc"
    );
}

#[test]
fn every_lowerable_def_head_shadows() {
    // The interpreter-only heads (`defn-`, `defonce`, `defmulti`, …) are
    // rejected outright by `lower_list`, so a unit containing one never
    // lowers; these are the def-like heads a successful lowering can contain.
    for form in [
        "(def count 1)",
        "(defn count [c] 1)",
        "(declare other count)",
        // Nested inside a form that is itself lowered.
        "(when true (def count 1))",
    ] {
        let ir = lower_fn_body(
            Some("__cljrs_main"),
            "x",
            &["coll".into()],
            &parse(&format!("{form} (count coll)")),
            false,
        )
        .expect("lower");
        assert!(
            !uses_known(&ir, KnownFn::Count),
            "`{form}` did not shadow core's `count`"
        );
    }
}

#[test]
fn stacked_metadata_on_a_def_name_still_shadows() {
    // `^:private ^:dynamic x` nests as Meta(_, Meta(_, Symbol)); peeling only
    // one layer would drop the shadow and inline core's `count`.
    let ir = lower_fn_body(
        Some("__cljrs_main"),
        "x",
        &["coll".into()],
        &parse("(def ^:private ^:dynamic count (fn [c] 1)) (count coll)"),
        false,
    )
    .expect("lower");
    assert!(!uses_known(&ir, KnownFn::Count));
    assert!(has_dynamic_call(&ir));
}

#[test]
fn a_quoted_def_is_data_not_a_definition() {
    // Both spellings of quote: the reader form and the explicit head.
    for form in ["'(def count 1)", "(quote (def count 1))"] {
        let ir = lower_fn_body(
            Some("__cljrs_main"),
            "x",
            &["coll".into()],
            &parse(&format!("{form} (count coll)")),
            false,
        )
        .expect("lower");
        assert!(
            uses_known(&ir, KnownFn::Count),
            "`{form}` is data; it must not shadow core's `count`"
        );
    }
}

#[test]
fn clojure_cores_own_defs_are_still_core() {
    // Core defining `inc` *is* core's `inc`; lowering must keep inlining it.
    let ir = lower_fn_body(
        Some("__cljrs_main"),
        "clojure.core",
        &[],
        &parse("(def inc (fn [n] (+ n 1))) (inc 1)"),
        false,
    )
    .expect("lower");
    assert!(body_uses_known(&ir, KnownFn::Add));
}

// ── Qualified symbols ───────────────────────────────────────────────────────

#[test]
fn another_namespaces_var_is_not_core() {
    let ir = lower("(my.ns/count [1 2 3])");
    assert!(
        !uses_known(&ir, KnownFn::Count),
        "`my.ns/count` is not `clojure.core/count`"
    );
    assert!(has_dynamic_call(&ir));
}

#[test]
fn qualified_core_names_still_lower_as_core() {
    assert!(uses_known(
        &lower("(clojure.core/count [1 2 3])"),
        KnownFn::Count
    ));
    assert!(uses_known(&lower("(clojure.core/inc 1)"), KnownFn::Add));
    // `clojure.core//` is division written qualified.
    assert!(uses_known(&lower("(clojure.core// 6 2)"), KnownFn::Div));
    assert!(uses_known(&lower("(/ 6 2)"), KnownFn::Div));
}
