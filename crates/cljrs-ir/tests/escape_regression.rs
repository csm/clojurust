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

use cljrs_ir::lower::{
    EscapeContext, EscapeState, analyze, lower_fn_body, make_analysis_context, optimize,
};
use cljrs_ir::{Inst, IrFunction};
use cljrs_reader::Parser;
use std::sync::Arc;

fn lower(source: &str) -> IrFunction {
    let mut parser = Parser::new(source.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    lower_fn_body(Some("test"), "user", &[], &forms, false).expect("lower")
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
    // The vec is the function's return value.  After stage 2 the analyser
    // classifies it as `Returns` (not `Escapes`) — the caller decides whether
    // it truly escapes.
    let ir = lower("[1 2 3]");
    let dst = first_alloc_vec(&ir).expect("alloc-vec");
    let ctx = make_analysis_context(&ir);
    let analysis = analyze(&ir, Some(&ctx));
    assert_eq!(
        analysis.states.get(&dst).copied(),
        Some(EscapeState::Returns),
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
    // A vec allocated fresh inside the loop body (not a phi-flowing loop
    // variable) should be region-promoted: it's consumed within the same block
    // before the RecurJump back-edge fires, so the region is safe.
    let ir = lower(
        "(loop [i 5 acc 0] (if (= i 0) acc (let [tmp [i]] (recur (- i 1) (+ acc (count tmp))))))",
    );
    let optimized = optimize(ir);
    assert!(
        region_alloc_count(&optimized) >= 1,
        "optimizer should promote the body-local temp vec; IR was:\n{}",
        optimized
    );
}

// ── Inlining tests ───────────────────────────────────────────────────────────

/// Count `Inst::Call` instructions (non-known dynamic calls) in the IR tree.
fn dynamic_call_count(ir: &IrFunction) -> usize {
    let mut n = 0;
    for block in &ir.blocks {
        for inst in &block.insts {
            if matches!(inst, cljrs_ir::Inst::Call(..)) {
                n += 1;
            }
        }
    }
    for sub in &ir.subfunctions {
        n += dynamic_call_count(sub);
    }
    n
}

#[test]
fn inlined_callee_alloc_promoted_to_region() {
    // make-pair returns [a b]; caller only calls count on the result.
    // Without inlining: the AllocVector escapes via Return → GC.
    // With inlining:    the AllocVector is local to caller → NoEscape → RegionAlloc.
    let ir = lower(
        "(do
           (defn make-pair [a b] [a b])
           (defn count-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);
    assert!(
        region_alloc_count(&optimized) >= 1,
        "inlined callee alloc should be region-promoted; IR:\n{optimized}"
    );
}

#[test]
fn eligible_call_is_eliminated_after_inline() {
    // After inlining make-pair into count-pair, the dynamic Call instruction
    // should be gone from count-pair's body.
    let ir = lower(
        "(do
           (defn make-pair [a b] [a b])
           (defn count-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);
    assert_eq!(
        dynamic_call_count(&optimized),
        0,
        "dynamic call to make-pair should be eliminated by inlining; IR:\n{optimized}"
    );
}

#[test]
fn non_escaping_inline_result_stays_no_escape() {
    // Nested: make-triple wraps make-pair; both should be inlined and
    // the allocation promoted.
    let ir = lower(
        "(do
           (defn make-triple [a b c] [a b c])
           (defn sum-triple [x] (count (make-triple x x x))))",
    );
    let optimized = optimize(ir);
    assert!(
        region_alloc_count(&optimized) >= 1,
        "inlined triple alloc should be region-promoted; IR:\n{optimized}"
    );
}

// ── Stage-3: caller-context propagation tests ────────────────────────────────

/// Walk `ir` and every subfunction recursively; return true if any function's
/// `analyze` result has a non-empty `cross_fn_no_escape` map.
fn any_has_cross_fn_no_escape(ir: &cljrs_ir::IrFunction, ctx: &EscapeContext) -> bool {
    if !analyze(ir, Some(ctx)).cross_fn_no_escape.is_empty() {
        return true;
    }
    ir.subfunctions
        .iter()
        .any(|sub| any_has_cross_fn_no_escape(sub, ctx))
}

#[test]
fn cross_fn_no_escape_populated_when_return_value_is_local() {
    // The top-level expression calls make-pair and passes the result straight
    // to `count` — only an inspection use, so call_dst is NoEscape.
    // make-pair's AllocVector is classified Returns in isolation (stage 2).
    // Stage-3 pass-2 should record it in cross_fn_no_escape.
    //
    // Note: we do NOT go through `optimize` here so the Call instruction is
    // still present (inlining would replace it and pass-2 would see nothing).
    let ir = lower(
        "(do
           (defn make-pair [a b] [a b])
           (count (make-pair 1 2)))",
    );
    let ctx = make_analysis_context(&ir);
    assert!(
        any_has_cross_fn_no_escape(&ir, &ctx),
        "make-pair's AllocVector should appear in cross_fn_no_escape; IR:\n{ir}"
    );
}

#[test]
fn cross_fn_no_escape_empty_when_return_value_escapes() {
    // The top-level returns make-pair's result directly — call_dst is Returns,
    // not NoEscape, so pass-2 should not record anything.
    let ir = lower(
        "(do
           (defn make-pair [a b] [a b])
           (make-pair 1 2))",
    );
    let ctx = make_analysis_context(&ir);
    assert!(
        !any_has_cross_fn_no_escape(&ir, &ctx),
        "when the call result itself escapes, cross_fn_no_escape must be empty; IR:\n{ir}"
    );
}

#[test]
fn cross_fn_no_escape_covers_all_returns_allocs() {
    // make-triple returns a 3-element vector; use-triple passes it to count.
    // All Returns-tagged allocs in make-triple should be captured.
    let ir = lower(
        "(do
           (defn make-triple [a b c] [a b c])
           (count (make-triple 1 2 3)))",
    );
    let ctx = make_analysis_context(&ir);
    // Check that the analysis (across the tree) picks up make-triple's alloc.
    assert!(
        any_has_cross_fn_no_escape(&ir, &ctx),
        "make-triple's AllocVector should appear in cross_fn_no_escape; IR:\n{ir}"
    );
}

// ── Stage-4: cross-function region promotion tests ─────────────────────────

/// Count `Inst::CallWithRegion` insts across the IR tree.
fn call_with_region_count(ir: &IrFunction) -> usize {
    let mut n = 0;
    for block in &ir.blocks {
        for inst in &block.insts {
            if matches!(inst, Inst::CallWithRegion(..)) {
                n += 1;
            }
        }
    }
    for sub in &ir.subfunctions {
        n += call_with_region_count(sub);
    }
    n
}

/// Count `Inst::RegionParam` insts across the IR tree.
fn region_param_count(ir: &IrFunction) -> usize {
    let mut n = 0;
    for block in &ir.blocks {
        for inst in &block.insts {
            if matches!(inst, Inst::RegionParam(_)) {
                n += 1;
            }
        }
    }
    for sub in &ir.subfunctions {
        n += region_param_count(sub);
    }
    n
}

#[test]
fn stage4_promotes_non_inlineable_callee() {
    // make-pair has a nested `fn` (a subfunction), making it ineligible for
    // inlining.  Yet its `[a b]` allocation is `Returns` and the call site
    // only feeds `count` — so the result is `NoEscape` in the caller.  Stage
    // 4 should clone make-pair into a region-parameterised variant and
    // rewrite the call to `CallWithRegion`.
    let ir = lower(
        "(do
           (defn make-pair [a b]
             (let [f (fn [x] x)]
               [a b]))
           (defn use-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);
    assert!(
        call_with_region_count(&optimized) >= 1,
        "stage 4 should rewrite the call to CallWithRegion; IR:\n{optimized}"
    );
    assert!(
        region_param_count(&optimized) >= 1,
        "the cloned variant should carry a RegionParam marker; IR:\n{optimized}"
    );
    assert!(
        region_alloc_count(&optimized) >= 1,
        "the cloned variant's vector alloc should be region-promoted; IR:\n{optimized}"
    );
}

#[test]
fn stage4_skipped_when_callee_result_escapes() {
    // The caller returns make-pair's result directly — call_dst is `Returns`,
    // not `NoEscape`.  Stage 4 must NOT rewrite the call site.
    let ir = lower(
        "(do
           (defn make-pair [a b]
             (let [f (fn [x] x)]
               [a b]))
           (defn pass-thru [x] (make-pair x x)))",
    );
    let optimized = optimize(ir);
    assert_eq!(
        call_with_region_count(&optimized),
        0,
        "no CallWithRegion should be emitted when the call result escapes; IR:\n{optimized}"
    );
}

#[test]
fn stage4_inline_first_falls_back_to_promotion() {
    // When the callee is small enough to inline, stage-1 inlining handles it
    // and stage 4 has nothing to do.  This test verifies that the small
    // `make-pair` (no subfunctions) still gets a region-promoted alloc but
    // *via inlining*, not via cross-fn region passing.
    let ir = lower(
        "(do
           (defn make-pair [a b] [a b])
           (defn use-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);
    // Inlining removes the dynamic call.  stage 4 isn't expected to fire
    // because there's nothing left to clone.
    assert_eq!(
        dynamic_call_count(&optimized),
        0,
        "small callee should be inlined; IR:\n{optimized}"
    );
    assert!(
        region_alloc_count(&optimized) >= 1,
        "after inlining, the alloc should be region-promoted; IR:\n{optimized}"
    );
}

/// Recursively collect every named function in the IR tree, including
/// subfunctions of subfunctions.
fn collect_all_fn_names(ir: &IrFunction, out: &mut Vec<Arc<str>>) {
    if let Some(name) = &ir.name {
        out.push(name.clone());
    }
    for sub in &ir.subfunctions {
        collect_all_fn_names(sub, out);
    }
}

#[test]
fn stage4_does_not_produce_duplicate_subfunction_names() {
    // Regression for the codegen DuplicateDefinition crash: when stage 4
    // clones a callee with inner closures, those inner subfunctions must be
    // renamed so they don't collide with the original's subfunctions.  Both
    // sides of the IR tree end up registered under `user_funcs` — duplicate
    // names crash the cranelift module.
    let ir = lower(
        "(do
           (defn make-pair [a b]
             (let [f (fn [x] x)]
               [a b]))
           (defn use-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);
    let mut names: Vec<Arc<str>> = Vec::new();
    collect_all_fn_names(&optimized, &mut names);
    let mut sorted = names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        names.len(),
        "every named function in the IR tree must be unique; got duplicates in {names:?}"
    );
}

#[test]
fn stage4_handles_callee_with_self_capture() {
    // Top-level `defn` produces a closure with a self-ref capture, so
    // `make-pair__arity2.params` has 3 entries (self, a, b) while the call
    // site only supplies 2 arguments.  Stage 4 must prepend the call's
    // `callee_var` so the rewritten `CallWithRegion` matches the callee's
    // signature — otherwise codegen verifies fail with "mismatched argument
    // count".
    let ir = lower(
        "(do
           (defn make-pair [a b]
             (let [f (fn [x] x)]
               [a b]))
           (defn use-pair [x] (count (make-pair x x))))",
    );
    let optimized = optimize(ir);

    // Find the rewritten CallWithRegion and verify its arg count matches the
    // target subfunction's params count.
    fn find_target_and_call<'a>(ir: &'a IrFunction) -> Option<(&'a Arc<str>, usize, usize)> {
        for block in &ir.blocks {
            for inst in &block.insts {
                if let Inst::CallWithRegion(_, name, args) = inst {
                    if let Some(target) = ir
                        .subfunctions
                        .iter()
                        .find(|sf| sf.name.as_deref() == Some(name.as_ref()))
                    {
                        return Some((name, args.len(), target.params.len()));
                    }
                }
            }
        }
        for sub in &ir.subfunctions {
            if let Some(r) = find_target_and_call(sub) {
                return Some(r);
            }
        }
        None
    }

    let (name, arg_count, param_count) =
        find_target_and_call(&optimized).expect("CallWithRegion in IR");
    assert_eq!(
        arg_count, param_count,
        "CallWithRegion to {name} passes {arg_count} args but the target expects {param_count}",
    );
}

#[test]
fn stage4_co_promotes_container_contained_allocs() {
    // `make-grid` returns a vector of three inner coordinate vectors.  The
    // inner vectors are stored only into the returned outer vector, so they
    // are reachable solely through it.  When the call result feeds `count`
    // (a structural, non-extracting consumer) stage 4 should co-promote the
    // whole nest: the outer vector AND all three inner vectors.  This is the
    // `neighbours` showcase pattern in miniature.
    let ir = lower(
        "(do
           (defn make-grid [a]
             (let [f (fn [x] x)]
               [[a a] [a a] [a a]]))
           (defn use-grid [a] (count (make-grid a))))",
    );
    let optimized = optimize(ir);
    assert!(
        call_with_region_count(&optimized) >= 1,
        "stage 4 should rewrite the call to CallWithRegion; IR:\n{optimized}"
    );
    assert!(
        region_alloc_count(&optimized) >= 4,
        "outer vector + three inner vectors should all be region-promoted \
         (expected >= 4 RegionAllocs); IR:\n{optimized}"
    );
}

#[test]
fn stage4_deep_promotion_skipped_when_result_is_extracted() {
    // Same nest, but the call result is element-*extracted* via `first` before
    // being consumed.  The extracted inner vector could outlive the caller's
    // region, so deep co-promotion must be skipped: only the outer container
    // (the shallow `Returns` alloc) is region-promoted, leaving exactly one
    // RegionAlloc.
    let ir = lower(
        "(do
           (defn make-grid [a]
             (let [f (fn [x] x)]
               [[a a] [a a] [a a]]))
           (defn pick [a] (count (first (make-grid a)))))",
    );
    let optimized = optimize(ir);
    assert_eq!(
        region_alloc_count(&optimized),
        1,
        "only the outer container should be promoted when the result is \
         element-extracted; IR:\n{optimized}"
    );
}

// ── Eager HOF fusion tests ───────────────────────────────────────────────────

/// Count `CallKnown` insts whose known fn debug-prints as `name`, across tree.
fn known_call_count(ir: &IrFunction, name: &str) -> usize {
    let mut n = 0;
    for block in &ir.blocks {
        for inst in &block.insts {
            if let Inst::CallKnown(_, kfn, _) = inst
                && format!("{kfn:?}") == name
            {
                n += 1;
            }
        }
    }
    for sub in &ir.subfunctions {
        n += known_call_count(sub, name);
    }
    n
}

#[test]
fn count_filter_is_fused_to_count_filter() {
    // `count` is the sole consumer of `filter`, so the pair fuses into the
    // allocation-free `CountFilter` and the dead `filter` is removed.
    let ir = lower("(count (filter (fn [x] x) [1 2 3]))");
    let optimized = optimize(ir);
    assert_eq!(
        known_call_count(&optimized, "Filter"),
        0,
        "the fused filter should be removed; IR:\n{optimized}"
    );
    assert_eq!(
        known_call_count(&optimized, "Count"),
        0,
        "the count should become CountFilter; IR:\n{optimized}"
    );
    assert_eq!(known_call_count(&optimized, "CountFilter"), 1);
}

#[test]
fn into_filter_is_fused() {
    let ir = lower("(into [] (filter (fn [x] x) [1 2 3]))");
    let optimized = optimize(ir);
    assert_eq!(known_call_count(&optimized, "Filter"), 0);
    assert_eq!(known_call_count(&optimized, "Into"), 0);
    assert_eq!(known_call_count(&optimized, "IntoFilter"), 1);
}

#[test]
fn into_mapcat_is_fused() {
    let ir = lower("(into #{} (mapcat (fn [x] [x x]) [1 2 3]))");
    let optimized = optimize(ir);
    assert_eq!(known_call_count(&optimized, "Mapcat"), 0);
    assert_eq!(known_call_count(&optimized, "Into"), 0);
    assert_eq!(known_call_count(&optimized, "IntoMapcat"), 1);
}

#[test]
fn count_filter_not_fused_when_filter_escapes() {
    // The filter result is used twice (count + returned), so it must not fuse.
    let ir = lower("(let [s (filter (fn [x] x) [1 2 3])] [(count s) s])");
    let optimized = optimize(ir);
    assert_eq!(
        known_call_count(&optimized, "CountFilter"),
        0,
        "filter with a non-count use must not fuse; IR:\n{optimized}"
    );
}

// Suppress an unused-import lint if Arc isn't picked up by every test.
#[allow(dead_code)]
fn _arc_witness() -> Arc<str> {
    Arc::from("x")
}
