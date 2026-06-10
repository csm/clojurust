//! Phase 10.4 (OSR) — semantic equivalence of the OSR-entry transform.
//!
//! `cljrs_ir::osr::build_osr_function` produces a variant of a function whose
//! entry block jumps straight to a loop header, with the loop variables and
//! every pre-loop value the loop reads passed as parameters.  The runtime
//! transfer hands the interpreter's registers to that variant, so the variant
//! must compute *exactly* what the original call would have computed from that
//! point on — including everything after the loop exits.
//!
//! These tests check that contract using the Tier-1 interpreter on both sides
//! (no Cranelift involved): running the OSR variant from a mid-loop state must
//! equal the remainder of the original computation.

use std::sync::Arc;

use cljrs_eval::{Env, ir_interp::interpret_ir};
use cljrs_interp::standard_env_minimal;
use cljrs_ir::osr::build_osr_function;
use cljrs_ir::{Block, BlockId, Const, Inst, IrFunction, KnownFn, Terminator, VarId};
use cljrs_value::Value;

/// `(fn [n] (loop [i 0 acc 0] (if (< i n) (recur (+ i 1) (+ acc i)) acc)))`,
/// hand-lowered.  See `cljrs-ir/src/osr.rs` tests for the block layout.
fn sum_loop_fn() -> IrFunction {
    let mut f = IrFunction::new(Some(Arc::from("sum-to")), None);
    let v = |n: u32| VarId(n);
    f.params = vec![(Arc::from("n"), v(0))];
    f.next_var = 9;
    f.next_block = 4;
    f.blocks = vec![
        Block {
            id: BlockId(0),
            phis: vec![],
            insts: vec![
                Inst::Const(v(1), Const::Long(0)),
                Inst::Const(v(2), Const::Long(0)),
            ],
            terminator: Terminator::Jump(BlockId(1)),
        },
        Block {
            id: BlockId(1),
            phis: vec![
                Inst::Phi(v(3), vec![(BlockId(0), v(1)), (BlockId(2), v(6))]),
                Inst::Phi(v(4), vec![(BlockId(0), v(2)), (BlockId(2), v(7))]),
            ],
            insts: vec![Inst::CallKnown(v(5), KnownFn::Lt, vec![v(3), v(0)])],
            terminator: Terminator::Branch {
                cond: v(5),
                then_block: BlockId(2),
                else_block: BlockId(3),
            },
        },
        Block {
            id: BlockId(2),
            phis: vec![],
            insts: vec![
                Inst::Const(v(8), Const::Long(1)),
                Inst::CallKnown(v(6), KnownFn::Add, vec![v(3), v(8)]),
                Inst::CallKnown(v(7), KnownFn::Add, vec![v(4), v(3)]),
            ],
            terminator: Terminator::RecurJump {
                target: BlockId(1),
                args: vec![v(6), v(7)],
            },
        },
        Block {
            id: BlockId(3),
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(v(4)),
        },
    ];
    f
}

fn run(ir: &IrFunction, args: Vec<Value>) -> Value {
    let _mutator = cljrs_gc::register_mutator();
    let globals = standard_env_minimal(None, None, None);
    let mut env = Env::new(globals.clone(), "user");
    let ns: Arc<str> = Arc::from("user");
    cljrs_env::callback::push_eval_context(&env);
    let result = interpret_ir(ir, args, &globals, &ns, &mut env);
    cljrs_env::callback::pop_eval_context();
    result.expect("interpret")
}

#[test]
fn osr_variant_resumes_mid_loop_state() {
    let orig = sum_loop_fn();
    // Whole-call baseline: sum 0..10 = 45.
    assert_eq!(run(&orig, vec![Value::Long(10)]), Value::Long(45));

    let osr = build_osr_function(&orig, BlockId(1)).expect("transform");
    // live-ins: [i (v3), acc (v4), n (v0)] — see the transform's ordering
    // contract (header φ destinations first, then outer values by VarId).
    assert_eq!(osr.live_ins, vec![VarId(3), VarId(4), VarId(0)]);

    // Resume from i=5, acc=10 (the state after 5 interpreted iterations):
    // remaining iterations add 5+6+7+8+9 → 45.
    let resumed = run(
        &osr.func,
        vec![Value::Long(5), Value::Long(10), Value::Long(10)],
    );
    assert_eq!(resumed, Value::Long(45));
}

#[test]
fn osr_variant_matches_original_for_every_entry_point() {
    let orig = sum_loop_fn();
    let osr = build_osr_function(&orig, BlockId(1)).expect("transform");
    let n = 12i64;
    let expected = run(&orig, vec![Value::Long(n)]);

    // Entering the OSR variant at any iteration boundary k must finish with
    // the same answer the original produces.
    for k in 0..=n {
        let acc: i64 = (0..k).sum();
        let resumed = run(
            &osr.func,
            vec![Value::Long(k), Value::Long(acc), Value::Long(n)],
        );
        assert_eq!(resumed, expected, "diverged when resuming at i={k}");
    }
}

#[test]
fn osr_variant_runs_post_loop_code() {
    // Wrap the loop exit in extra post-loop work — `(+ acc n)` after the loop —
    // to check the OSR variant carries the continuation, not just the loop.
    let mut orig = sum_loop_fn();
    let result_var = VarId(9);
    orig.next_var = 10;
    orig.blocks[3] = Block {
        id: BlockId(3),
        phis: vec![],
        insts: vec![Inst::CallKnown(
            result_var,
            KnownFn::Add,
            vec![VarId(4), VarId(0)],
        )],
        terminator: Terminator::Return(result_var),
    };
    assert_eq!(run(&orig, vec![Value::Long(10)]), Value::Long(55));

    let osr = build_osr_function(&orig, BlockId(1)).expect("transform");
    let resumed = run(
        &osr.func,
        vec![Value::Long(5), Value::Long(10), Value::Long(10)],
    );
    assert_eq!(resumed, Value::Long(55));
}
