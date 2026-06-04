//! Closure-capture minimization regression tests.
//!
//! A closure must capture only the enclosing locals it actually references —
//! not every local in scope.  Over-capturing wastes a boxed allocation per
//! capture on every invocation and inflates the compiled arity (captures +
//! params), which previously tripped the call trampoline's fixed maximum and
//! silently returned nil.
//!
//! These use the public Rust ANF lowerer directly, so they run quickly and
//! don't depend on the embedded Clojure compiler.

use cljrs_ir::lower::lower_fn_body;
use cljrs_ir::{Inst, IrFunction};
use cljrs_reader::Parser;

fn lower(source: &str) -> IrFunction {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    lower_fn_body(Some("test"), "user", &[], &forms, false).expect("lower")
}

/// Capture counts of every `AllocClosure` in the tree (root + subfunctions).
fn capture_counts(ir: &IrFunction) -> Vec<usize> {
    let mut out = Vec::new();
    fn walk(ir: &IrFunction, out: &mut Vec<usize>) {
        for block in &ir.blocks {
            for inst in block.phis.iter().chain(block.insts.iter()) {
                if let Inst::AllocClosure(_, _, captures) = inst {
                    out.push(captures.len());
                }
            }
        }
        for sub in &ir.subfunctions {
            walk(sub, out);
        }
    }
    walk(ir, &mut out);
    out
}

#[test]
fn unused_enclosing_locals_are_not_captured() {
    // The inner `(fn [x] x)` references none of a/b/c/d/e — it must capture 0.
    let ir = lower("(let [a 1 b 2 c 3 d 4 e 5] (fn [x] x))");
    let counts = capture_counts(&ir);
    assert!(
        counts.iter().all(|&c| c == 0),
        "no closure should capture unused locals; got {counts:?}"
    );
}

#[test]
fn referenced_enclosing_local_is_captured() {
    // `(fn [x] (+ x a))` references `a`, so exactly one capture is expected.
    let ir = lower("(let [a 1 b 2 c 3] (fn [x] (+ x a)))");
    let counts = capture_counts(&ir);
    assert!(
        counts.contains(&1),
        "the closure referencing `a` must capture it; got {counts:?}"
    );
    assert!(
        counts.iter().all(|&c| c <= 1),
        "only the referenced local should be captured; got {counts:?}"
    );
}

#[test]
fn nested_closure_forces_transitive_capture() {
    // The middle `(fn [a] ...)` doesn't textually use `k` itself, but its
    // nested `(fn [b] (+ a b k))` does, so the middle closure must capture `k`
    // to pass it inward.  `collect_symbol_names` recurses into nested fn bodies
    // precisely so this transitive capture is never dropped.
    let ir = lower("(let [k 7] (fn [a] (fn [b] (+ a b k))))");
    let counts = capture_counts(&ir);
    assert!(
        counts.iter().any(|&c| c >= 1),
        "transitively-referenced `k` must be captured somewhere; got {counts:?}"
    );
}
