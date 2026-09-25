//! An evaluated-position `^meta` annotation must lower to the same runtime
//! attach the tree-walker performs.
//!
//! Before the fix `lower_form`'s `FormKind::Meta` arm recursed into the
//! annotated form, discarding the annotation. `ExecutionMode::Tiered` is the default, so
//! `(defn f [] (meta ^{:a 1} [1]))` answered `{:a 1}` until the function was
//! promoted and `nil` afterwards — a result that depended on how hot the code
//! got — and AOT, which lowers through the same path, answered `nil` always.
//!
//! These tests assert the *shape of the emitted IR*, so they hold without
//! waiting on the background lowering worker to publish.

use cljrs_ir::lower::lower_fn_body;
use cljrs_ir::{Const, Inst, IrFunction};
use cljrs_reader::Parser;

fn parse(source: &str) -> Vec<cljrs_reader::Form> {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    parser.parse_all().expect("parse")
}

fn lower(source: &str) -> IrFunction {
    lower_fn_body(Some("test"), "user", &[], &parse(source), false).expect("lower")
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

/// Does the lowered body load `clojure.core/<name>`?
fn loads_core(ir: &IrFunction, name: &str) -> bool {
    insts(ir)
        .iter()
        .any(|i| matches!(i, Inst::LoadGlobal(_, ns, n) if &**ns == "clojure.core" && &**n == name))
}

/// The metadata attach is `meta` → `merge` → `with-meta`; `merge` is what makes
/// stacked annotations both survive.
fn attaches_meta(ir: &IrFunction) -> bool {
    loads_core(ir, "with-meta") && loads_core(ir, "merge") && loads_core(ir, "meta")
}

fn has_const(ir: &IrFunction, c: Const) -> bool {
    insts(ir)
        .iter()
        .any(|i| matches!(i, Inst::Const(_, k) if *k == c))
}

// ── Forms that take the annotation as runtime metadata ──────────────────────

#[test]
fn vector_literal_attaches_the_annotation() {
    assert!(
        attaches_meta(&lower("^{:a 1} [1]")),
        "the annotation on a vector literal was discarded by IR lowering"
    );
}

#[test]
fn map_set_and_fn_literals_attach_the_annotation() {
    for src in ["^{:a 1} {:k 1}", "^{:a 1} #{1}", "^{:a 1} (fn [] 1)"] {
        assert!(
            attaches_meta(&lower(src)),
            "the annotation on `{src}` was discarded by IR lowering"
        );
    }
}

// `#(…)` is expanded to `(fn* …)` before lowering — `lower_form` rejects an
// un-expanded reader macro — so its annotation arrives here already in `fn*`
// form. The tree-walk side of that case is pinned in
// `cljrs-runtime/tests/form_metadata.rs`.

#[test]
fn stacked_annotations_still_attach() {
    // `^:a ^:b [1]` reads as Meta(:a, Meta(:b, [1])); both layers attach, and
    // the `merge` in each is what keeps the inner one alive.
    let ir = lower("^:a ^:b [1]");
    assert!(attaches_meta(&ir));
}

// ── Forms that take it as a hint and drop it ────────────────────────────────

#[test]
fn a_call_does_not_attach_the_annotation() {
    // `(meta ^{:a 1} (list 1))` is nil on the JVM: metadata attaches to a form
    // that *constructs* an IObj, not to the result of an arbitrary expression.
    assert!(
        !attaches_meta(&lower("^{:a 1} (list 1)")),
        "an annotation on a call must be a hint, not runtime metadata"
    );
}

#[test]
fn a_symbol_does_not_attach_the_annotation() {
    assert!(!attaches_meta(&lower("^{:a 1} x")));
}

#[test]
fn a_type_hint_on_a_call_costs_nothing() {
    // The common `(.method ^Foo obj)` / `(f ^long n)` shape must not grow three
    // global calls per annotation.
    assert!(!attaches_meta(&lower("(inc ^long n)")));
}

#[test]
fn an_empty_list_attaches_like_an_empty_vector() {
    // `()` has no head, so it is the empty-list literal, not a call.
    assert!(attaches_meta(&lower("^{:a 1} ()")));
    assert!(attaches_meta(&lower("^{:a 1} []")));
}

#[test]
fn an_anonymous_async_fn_is_not_lowered() {
    // A lowered closure cannot be dispatched as async, so lowering one would
    // make calling it synchronous once the body was promoted. Both spellings
    // are refused, which keeps the enclosing body on the tree-walker.
    for src in [
        "^:async (fn [] 1)",
        "^{:async true} (fn* [] 1)",
        "^:async ^:a (fn [] 1)",
        "^:a (fn ^:async [] 1)",
        "(fn ^:async [] 1)",
        "(fn ^:async named [] 1)",
        "(let [f ^:async (fn [] 1)] f)",
    ] {
        let result = lower_fn_body(Some("test"), "user", &[], &parse(src), false);
        assert!(result.is_err(), "`{src}` lowered to a synchronous closure");
    }
}

#[test]
fn a_non_async_annotation_on_a_fn_still_lowers() {
    for src in ["^{:async false} (fn [] 1)", "^:a (fn [] 1)"] {
        assert!(attaches_meta(&lower(src)), "`{src}`");
    }
}

// ── The annotation itself ───────────────────────────────────────────────────

#[test]
fn a_symbol_tag_is_a_symbol_constant_not_a_var_load() {
    // `^String [1]` denotes `{:tag String}` — the symbol, never whatever the
    // symbol resolves to. A `LoadGlobal` for `String` would mean the tag was
    // evaluated.
    let ir = lower("^String [1]");
    assert!(has_const(&ir, Const::Symbol("String".into())));
    assert!(has_const(&ir, Const::Keyword("tag".into())));
    assert!(
        !loads_core(&ir, "String"),
        "the tag symbol must not be resolved as a var"
    );
}

#[test]
fn a_string_tag_is_a_string_constant() {
    let ir = lower("^\"String\" [1]");
    assert!(has_const(&ir, Const::Str("String".into())));
    assert!(has_const(&ir, Const::Keyword("tag".into())));
}

#[test]
fn a_keyword_shorthand_expands_to_a_map() {
    // `^:dyn` → `{:dyn true}`: a keyword constant, a `true` constant, one map.
    let ir = lower("^:dyn [1]");
    assert!(has_const(&ir, Const::Keyword("dyn".into())));
    assert!(has_const(&ir, Const::Bool(true)));
    assert!(
        insts(&ir).iter().any(|i| matches!(i, Inst::AllocMap(..))),
        "the shorthand must expand to a map, not stay a bare keyword"
    );
}

// ── Inside `quote` the annotation is data ───────────────────────────────────

#[test]
fn a_quoted_annotation_attaches_to_a_form_that_can_carry_it() {
    // `lower_quote` had no `FormKind::Meta` arm, so a body containing quoted
    // metadata failed to lower and the whole function silently fell back to the
    // tree-walker — correct, but never compiled.
    for src in ["'^{:a 1} [1]", "'^:dyn sym", "'^{:a 1} (1 2)", "'^String x"] {
        assert!(
            attaches_meta(&lower(src)),
            "`{src}` did not attach its annotation in the quoted position"
        );
    }
}

#[test]
fn a_quoted_annotation_on_a_scalar_attaches_nothing() {
    // Decided statically: inside `quote` every form is a literal, so whether
    // the value can carry metadata is a question about the form.
    for src in [
        "'^{:a 1} 42",
        "'^{:a 1} \"s\"",
        "'^{:a 1} :kw",
        "'^{:a 1} nil",
    ] {
        assert!(
            !attaches_meta(&lower(src)),
            "`{src}` attached metadata to a value that cannot carry it"
        );
    }
}

#[test]
fn a_quoted_body_lowers_at_all() {
    // The regression this arm exists to prevent: an `Err` here means the
    // enclosing function declines IR lowering entirely.
    for src in ["'^{:a 1} [1]", "'^::kw [1]", "(list '^{:a 1} [1] 2)"] {
        assert!(
            lower_fn_body(Some("test"), "user", &[], &parse(src), false).is_ok(),
            "`{src}` still refuses to lower"
        );
    }
}
