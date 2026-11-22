//! Function-reference integrity across repeated stage-4 specialisation
//! (issue #509).
//!
//! Mirrors the script/REPL flow: each defn arity is lowered and optimised on
//! its own, then handed to later lowerings as an [`ExternalDefn`].  Stage 4
//! clones an already-specialised callee (which owns its own `__rgN`
//! variants), so references must survive more than one specialisation layer
//! and more than one sibling clone of the same callee.

use std::sync::Arc;

use cljrs_ir::fnref::for_each_fn_ref;
use cljrs_ir::lower::{ExternalDefn, lower_fn_body, optimize_with_externals};
use cljrs_ir::verify::{FnRefProblem, verify_fn_refs};
use cljrs_ir::{Inst, IrFunction};
use cljrs_reader::Parser;

fn lower_arity(name: &str, params: &[&str], body: &str, externals: &[ExternalDefn]) -> IrFunction {
    let mut parser = Parser::new(body.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let params: Vec<Arc<str>> = params.iter().map(|p| Arc::from(*p)).collect();
    let ir = lower_fn_body(Some(name), "user", &params, &forms, false).expect("lower");
    // `optimize_with_externals` itself verifies at every pass boundary in
    // debug builds; verify explicitly too so release test runs check it.
    let (ir, _) = optimize_with_externals(ir, externals);
    if let Err(errs) = verify_fn_refs(&ir) {
        panic!(
            "{name}: {}\nIR:\n{ir}",
            cljrs_ir::verify::format_errors(&errs)
        );
    }
    ir
}

fn external(name: &str, param_count: usize, ir: &IrFunction) -> ExternalDefn {
    ExternalDefn {
        ns: Arc::from("user"),
        name: Arc::from(name),
        arity_fn_names: vec![Arc::from(format!("user/{name}#{param_count}").as_str())],
        param_counts: vec![param_count],
        is_variadic: vec![false],
        arity_irs: vec![Arc::new(ir.clone())],
    }
}

fn count_insts(ir: &IrFunction, pred: &impl Fn(&Inst) -> bool) -> usize {
    ir.blocks
        .iter()
        .flat_map(|b| b.insts.iter())
        .filter(|i| pred(i))
        .count()
        + ir.subfunctions
            .iter()
            .map(|s| count_insts(s, pred))
            .sum::<usize>()
}

fn region_calls(ir: &IrFunction) -> usize {
    count_insts(ir, &|i| matches!(i, Inst::CallWithRegion(..)))
}

/// Depth of the deepest chain of nested `CallWithRegion` targets.
fn specialisation_depth(ir: &IrFunction) -> usize {
    let own = usize::from(
        ir.blocks
            .iter()
            .any(|b| b.insts.iter().any(|i| matches!(i, Inst::RegionParam(_)))),
    );
    own + ir
        .subfunctions
        .iter()
        .map(specialisation_depth)
        .max()
        .unwrap_or(0)
}

#[test]
fn nested_and_sibling_specialisations_keep_targets_valid() {
    let leaf = lower_arity("leaf", &["x"], "[x]", &[]);
    let leaf_ext = external("leaf", 1, &leaf);

    // Layer 1: middle owns `leaf__rg1` and calls it via CallWithRegion.
    let middle = lower_arity(
        "middle",
        &["x"],
        "[(count (leaf x))]",
        std::slice::from_ref(&leaf_ext),
    );
    assert!(
        region_calls(&middle) >= 1,
        "middle should region-call a leaf variant; IR:\n{middle}"
    );
    let middle_ext = external("middle", 1, &middle);

    // Layer 2, with sibling specialisations: two separate call sites of
    // `middle` in two separate caller fns, each cloning middle (and with it
    // middle's own `leaf__rg1`).
    let outer = lower_arity(
        "outer",
        &["x"],
        "(let [f (fn [y] (count (middle y)))
               g (fn [z] (count (middle z)))]
           (+ (f x) (g x) (count (middle x))))",
        &[middle_ext.clone(), leaf_ext.clone()],
    );
    assert!(
        region_calls(&outer) >= 2,
        "outer should region-call several middle variants; IR:\n{outer}"
    );
    assert!(
        specialisation_depth(&outer) >= 2,
        "a cloned middle variant should itself own a leaf variant; IR:\n{outer}"
    );

    // Layer 3: clone a function that already contains layer-2 clones.
    let outer_ext = external("outer", 1, &outer);
    let top = lower_arity(
        "top",
        &["x"],
        "[(count (outer x))]",
        &[outer_ext, middle_ext, leaf_ext],
    );
    // Every name stays unique and every target resolves (checked inside
    // `lower_arity`); names really were rewritten rather than dropped.
    let mut defs = Vec::new();
    for_each_fn_ref(&top, &mut |_, kind, name| {
        if kind == cljrs_ir::fnref::FnRefKind::Definition {
            defs.push(name.to_string());
        }
    });
    let unique: std::collections::HashSet<_> = defs.iter().collect();
    assert_eq!(unique.len(), defs.len(), "duplicate definitions: {defs:?}");
}

#[test]
fn verifier_rejects_a_deliberately_dangling_region_target() {
    let leaf = lower_arity("leaf", &["x"], "[x]", &[]);
    let mut middle = lower_arity(
        "middle",
        &["x"],
        "[(count (leaf x))]",
        &[external("leaf", 1, &leaf)],
    );

    // Rename the region variant's definition only — the bug class #506 hit.
    let variant = middle
        .subfunctions
        .iter_mut()
        .find(|s| s.name.as_deref().is_some_and(|n| n.contains("__rg")))
        .expect("middle owns a region variant");
    let old = variant.name.clone().unwrap();
    variant.name = Some(Arc::from(format!("{old}__stale").as_str()));

    let errs = verify_fn_refs(&middle).expect_err("dangling target must be rejected");
    let e = &errs[0];
    assert_eq!(e.function, "middle");
    assert!(e.location.is_some() && e.inst.is_some());
    assert!(
        matches!(&e.problem, FnRefProblem::Unresolved { target, .. } if *target == old),
        "{e}"
    );
}
