//! Lock-in test for the IR interpreter's per-block region scoping constraint.
//!
//! ## What this test documents
//!
//! `RegionStart` / `RegionAlloc` / `RegionEnd` instructions in the IR
//! interpreter give every allocation an explicit lifetime ending at its
//! matching `RegionEnd`. Any register holding a direct result from that
//! region is invalid after the region closes.
//!
//! The optimizer pass (`cljrs.compiler.optimize`) used to produce IR that
//! violated this constraint — it would wrap any non-escaping allocation in
//! a per-block region scope, including allocations whose value flowed out
//! of the block via a phi (`min-key:3+` was the canonical example).  That
//! caused a use-after-free in `clojure.core-test.min-key` and a
//! corresponding AOT panic.  The fix landed in the escape analysis
//! (`cljrs.compiler.escape`): allocations are now classified `:escapes`
//! whenever any transitive use lives in a different block from the
//! definition, so the optimizer never produces this shape from real code.
//!
//! This Rust test bypasses the optimizer and hand-builds the dangerous shape
//! directly. The interpreter must reject the expired register before touching
//! its freed allocation. This keeps the diagnostic deterministic across host
//! allocators and prevents the test itself from performing an undefined read.
//!
//! ## The hand-rolled IR
//!
//! ```text
//! fn(cond):
//!   block 0:
//!     %1 = LoadLocal "cond"           ; the param
//!     branch %1 -> block 1, block 2
//!   block 1:
//!     RegionStart %r1
//!     %2 = Const 42
//!     %3 = RegionAlloc %r1 Vector [%2]
//!     RegionEnd %r1                    ; <-- frees the [42] vector!
//!     jump block 3
//!   block 2:
//!     RegionStart %r2
//!     %4 = Const 99
//!     %5 = RegionAlloc %r2 Vector [%4]
//!     RegionEnd %r2                    ; <-- frees the [99] vector!
//!     jump block 3
//!   block 3:
//!     %6 = phi[(b1, %3), (b2, %5)]    ; both inputs are dangling
//!     %7 = CallKnown Count [%6]        ; touches freed memory
//!     return %7
//! ```
//!
//! The assertion is independent of GC mode and build profile: register
//! invalidation happens before the region's backing memory is released.

use std::sync::Arc;

use cljrs_ir::{BlockId, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Terminator, VarId};
use cljrs_runtime::tiered::{Env, ir_interp::interpret_ir};
use cljrs_value::Value;

/// Build the IR sketched in the module docs.
fn build_phi_over_regions_ir() -> IrFunction {
    use cljrs_ir::Block;

    // VarId allocation — kept tight so the IR stays readable.
    let cond = VarId(0);
    let r1 = VarId(1);
    let c42 = VarId(2);
    let v42 = VarId(3);
    let r2 = VarId(4);
    let c99 = VarId(5);
    let v99 = VarId(6);
    let phi = VarId(7);
    let count = VarId(8);
    let next_var = 9u32;

    let b0 = BlockId(0);
    let b1 = BlockId(1);
    let b2 = BlockId(2);
    let b3 = BlockId(3);

    let block0 = Block {
        id: b0,
        phis: vec![],
        insts: vec![],
        terminator: Terminator::Branch {
            cond,
            then_block: b1,
            else_block: b2,
        },
    };

    // Block 1: region-allocate `[42]`, end the region, jump.
    let block1 = Block {
        id: b1,
        phis: vec![],
        insts: vec![
            Inst::RegionStart(r1),
            Inst::Const(c42, Const::Long(42)),
            Inst::RegionAlloc(v42, r1, RegionAllocKind::Vector, vec![c42]),
            Inst::RegionEnd(r1),
        ],
        terminator: Terminator::Jump(b3),
    };

    // Block 2: region-allocate `[99]`, end the region, jump.
    let block2 = Block {
        id: b2,
        phis: vec![],
        insts: vec![
            Inst::RegionStart(r2),
            Inst::Const(c99, Const::Long(99)),
            Inst::RegionAlloc(v99, r2, RegionAllocKind::Vector, vec![c99]),
            Inst::RegionEnd(r2),
        ],
        terminator: Terminator::Jump(b3),
    };

    // Block 3: phi over the two now-dangling vectors, count the result.
    let block3 = Block {
        id: b3,
        phis: vec![Inst::Phi(phi, vec![(b1, v42), (b2, v99)])],
        insts: vec![Inst::CallKnown(count, KnownFn::Count, vec![phi])],
        terminator: Terminator::Return(count),
    };

    IrFunction {
        name: Some(Arc::from("phi-over-regions")),
        params: vec![(Arc::from("cond"), cond)],
        blocks: vec![block0, block1, block2, block3],
        next_var,
        next_block: 4,
        span: None,
        subfunctions: vec![],
        is_async: false,
        is_async_poll_fn: false,
        async_resume_blocks: vec![],
        seed_reprs: vec![],
        local_seed_reprs: vec![],
    }
}

/// Run the synthetic IR with `cond=true`, which steers control through
/// block 1 and verifies that the expired register is rejected before the
/// `[42]` vector can be read.
#[test]
#[should_panic(expected = "outlived region")]
fn region_phi_uaf_reproduces_under_interpreter() {
    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals.clone(), "user");

    let ir = build_phi_over_regions_ir();
    let ns: Arc<str> = Arc::from("user");

    // CallKnown dispatches through the eval-context-aware callback path,
    // which requires a context to be active.
    cljrs_runtime::env::callback::push_eval_context(&env);
    let result = interpret_ir(&ir, vec![Value::Bool(true)], &globals, &ns, &mut env);
    cljrs_runtime::env::callback::pop_eval_context();

    // Should be unreachable with the bug present.  If we ever get here,
    // either the bug is fixed (great — flip the test) or the panic surfaced
    // as an error result instead.  Either way, fail loudly.
    panic!(
        "expected use-after-free panic from per-block region scoping; \
         got result = {result:?}",
    );
}
