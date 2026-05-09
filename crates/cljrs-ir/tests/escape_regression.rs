//! Escape-analysis regression tests.
//!
//! Pin the high-level behaviour exposed by the `analyze` API:
//!
//! * Loop-local allocations whose only "escape" is via `recur` reach
//!   `NoEscape` (and hence get promoted by the optimizer).
//! * Allocations consumed by inspection-only known fns (`empty?`, `peek`,
//!   `count`, `nth`) don't get bumped to `ArgEscape`.
//! * Allocations that genuinely escape (via `Return`, `def`, etc.) still
//!   reach `Escapes`.
//!
//! These tests use the public Rust ANF lowerer + analyzer, so they run
//! quickly and don't depend on the embedded Clojure compiler.

use cljrs_ir::lower::{EscapeState, analyze, lower_fn_body, make_analysis_context, optimize};
use cljrs_ir::{Inst, IrFunction};
use cljrs_reader::Parser;
use std::sync::Arc;

fn lower(source: &str) -> IrFunction {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    lower_fn_body(Some("test"), "user", &[], &forms).expect("lower")
}

/// Count `Inst::RegionAlloc` insts in `ir` plus all subfunctions.
fn region_alloc_count(ir: &IrFunction) -> usize {
    let mut n = 0;
    for block in &ir.blocks {
        for inst in &block.insts {
            if matches!(inst, Inst::RegionAlloc(..)) {
                n += 1;
            }
        }
    }
    for sub in &ir.subfunctions {
        n += region_alloc_count(sub);
    }
    n
}

/// Find the first allocation of a given kind in any block of `ir`'s top
/// function and return its dst VarId.
fn first_alloc_vec(ir: &IrFunction) -> Option<cljrs_ir::VarId> {
    ir.blocks.iter().find_map(|b| {
        b.insts.iter().find_map(|i| match i {
            Inst::AllocVector(dst, _) => Some(*dst),
            _ => None,
        })
    })
}

#[test]
fn loop_local_empty_vec_is_no_escape_through_recur() {
    // The empty `[]` flows into `conj`, the conj result feeds the loop's
    // `queue` phi, and the phi is recur'd via `pop queue` at every step.
    // With the Recur-aware analyzer the verdict should reach `NoEscape`.
    let ir = lower("(loop [queue [] n 5] (if (empty? queue) n (recur (pop queue) (- n 1))))");
    let dst = first_alloc_vec(&ir).expect("alloc-vec for []");
    let ctx = make_analysis_context(&ir);
    let analysis = analyze(&ir, Some(&ctx));
    assert_eq!(
        analysis.states.get(&dst).copied(),
        Some(EscapeState::NoEscape),
        "empty `[]` in a loop with empty?/pop/recur should not escape"
    );
}

#[test]
fn returned_vector_escapes() {
    // The vec is the function's return value — must be classified as
    // Escapes regardless of analyser improvements.
    let ir = lower("[1 2 3]");
    let dst = first_alloc_vec(&ir).expect("alloc-vec");
    let ctx = make_analysis_context(&ir);
    let analysis = analyze(&ir, Some(&ctx));
    assert_eq!(
        analysis.states.get(&dst).copied(),
        Some(EscapeState::Escapes),
    );
}

#[test]
fn empty_q_and_count_dont_escape_arg() {
    // Inspection-only known fns should leave an alloc at NoEscape when
    // they're its only use.
    let ir = lower("(let [v [1 2 3]] (count v))");
    let dst = first_alloc_vec(&ir).expect("alloc-vec");
    let ctx = make_analysis_context(&ir);
    let analysis = analyze(&ir, Some(&ctx));
    assert_eq!(
        analysis.states.get(&dst).copied(),
        Some(EscapeState::NoEscape),
        "vec consumed only by `count` should not escape"
    );
}

#[test]
fn loop_local_alloc_gets_promoted_to_region() {
    // End-to-end: optimizer should turn the loop-local empty vec into a
    // RegionAlloc.
    let ir = lower("(loop [queue [] n 5] (if (empty? queue) n (recur (pop queue) (- n 1))))");
    let optimized = optimize(ir);
    assert!(
        region_alloc_count(&optimized) >= 1,
        "optimizer should promote the loop-local empty vec; IR was:\n{}",
        optimized
    );
}

// Suppress an unused-import lint if Arc isn't picked up by every test.
#[allow(dead_code)]
fn _arc_witness() -> Arc<str> {
    Arc::from("x")
}
