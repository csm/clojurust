//! Reproducer for the per-block region scoping bug surfaced by
//! `clojure.core-test.min-key` once `lower_and_optimize_arity` produced
//! `RegionStart` / `RegionAlloc` / `RegionEnd` instructions for the
//! variadic `min-key:3+` arity.
//!
//! ## The bug
//!
//! `cljrs.compiler.optimize` wraps every non-escaping allocation in a
//! region scope **whose lifetime is the enclosing basic block** —
//! `RegionStart` is inserted at the head of the block, `RegionEnd` is
//! inserted just before the terminator.  When the allocation flows out of
//! the block via control flow (a phi at a join point, or any subsequent
//! block referencing the value), the region is freed before the consumer
//! ever runs and the resulting `GcPtr` is dangling.
//!
//! ## The hand-rolled repro
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
//! Running this through `interpret_ir` reproduces the use-after-free
//! deterministically.  In `debug_assertions` builds it panics in
//! `GcPtr::get()` with `magic=…` mismatching `GC_MAGIC_ALIVE`; in release
//! builds it returns garbage or segfaults.
//!
//! Marked `#[should_panic]` so the test fails (and alerts us to flip it
//! back to expecting a successful result of `1`) when the underlying bug
//! is fixed.

use std::sync::Arc;

use cljrs_eval::{Env, ir_interp::interpret_ir};
use cljrs_ir::{BlockId, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Terminator, VarId};
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
    }
}

/// Run the synthetic IR with `cond=true`, which steers control through
/// block 1 and triggers the use-after-free on the `[42]` vector.
///
/// The expected behaviour, *if the bug were fixed*, is `Ok(Value::Long(1))`
/// — a one-element vector has count 1.  Today the call panics inside
/// `GcPtr::get()`'s magic-word assertion (debug builds) or returns
/// garbage / segfaults (release).
///
/// Gated to `debug_assertions` because the panic message we match on is
/// emitted only by the `GcPtr::get()` magic-word check, which is compiled
/// out in release builds.  In release the same defective IR may segfault,
/// return a stale value, or appear to succeed — none of which is a clean
/// `should_panic` signal.
#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "GcPtr::get() on freed object")]
fn region_phi_uaf_reproduces_under_interpreter() {
    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_stdlib::standard_env();
    let mut env = Env::new(globals.clone(), "user");

    let ir = build_phi_over_regions_ir();
    let ns: Arc<str> = Arc::from("user");

    // CallKnown dispatches through the eval-context-aware callback path,
    // which requires a context to be active.
    cljrs_env::callback::push_eval_context(&env);
    let result = interpret_ir(&ir, vec![Value::Bool(true)], &globals, &ns, &mut env);
    cljrs_env::callback::pop_eval_context();

    // Should be unreachable with the bug present.  If we ever get here,
    // either the bug is fixed (great — flip the test) or the panic surfaced
    // as an error result instead.  Either way, fail loudly.
    panic!(
        "expected use-after-free panic from per-block region scoping; \
         got result = {result:?}",
    );
}
