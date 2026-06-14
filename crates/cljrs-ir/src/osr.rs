//! On-stack replacement (OSR) entry construction — Phase 10.4.
//!
//! A script or REPL form is often a *single* call containing one very hot
//! `loop*` / `recur`.  Such a function never returns to re-dispatch, so
//! invocation-count tiering never promotes it.  OSR promotes it *mid-run*:
//! the Tier-1 interpreter counts loop back-edges and, when a loop header gets
//! hot, the JIT compiles an **OSR-entry variant** of the function built here —
//! a copy of the function whose entry block jumps straight to the loop header
//! and whose live-in values arrive as parameters.  The interpreter then
//! transfers its register file into the native frame at the next loop-header
//! entry and the rest of the call (the remaining iterations *and* everything
//! after the loop) runs natively.
//!
//! ## The transform
//!
//! Given a function and a loop-header block `H` (the target of a `RecurJump`):
//!
//! 1. Compute `R`, the set of blocks reachable from `H`.  Only those blocks
//!    are kept; everything before the loop has already executed in the
//!    interpreter.
//! 2. Compute the **live-ins**: values produced outside `R` but read inside
//!    it, plus the loop variables themselves (the φ destinations at `H`).
//! 3. Build a new entry block that jumps to `H`.  Each φ at `H` gets one new
//!    incoming edge from that entry block, fed by a fresh parameter; φ edges
//!    from blocks outside `R` (the original loop-entry path) are dropped.
//!    Non-φ live-ins become parameters bound to their original `VarId`s.
//!
//! The result is a self-contained [`IrFunction`] compiled by the ordinary JIT
//! backend.  [`OsrFunction::live_ins`] tells the interpreter which of *its*
//! registers to pass, in parameter order.
//!
//! ## Scratch regions opened before the loop
//!
//! The optimizer wraps lowered function bodies in `RegionStart`/`RegionEnd`,
//! so the loop's continuation usually contains a `RegionEnd` whose matching
//! `RegionStart` already executed *in the interpreter*.  The interpreter owns
//! that region (its frame closes it on unwind), so the OSR variant must not
//! close it again: such unmatched `RegionEnd`s are dropped, and `RegionAlloc`s
//! that name an interpreter-owned handle are rewritten to the corresponding
//! plain `Alloc*` instructions.  In compiled code a region handle is a real
//! `*mut Region` (returned by `rt_region_start` or arriving as the hidden
//! `RegionParam` argument), and the interpreter's handle register holds nil —
//! so it cannot be forwarded into native code.  The plain-alloc bridges are
//! region-aware (they consult the thread-local region stack, where the
//! interpreter's region is still active), so the allocations still land in
//! the interpreter-owned region; in the worst case they fall back to the GC
//! heap, which only extends lifetimes and is always sound.  Region pairs
//! entirely inside the loop are kept as-is.  A `CallWithRegion` naming an
//! interpreter-owned handle aborts the transform (the loop stays at Tier 1);
//! the stage-4 rewrite never produces one whose region scope crosses a loop
//! back-edge, so this is defensive.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::{Block, BlockId, Inst, IrFunction, RegionAllocKind, Terminator, VarId};

/// Hard cap on OSR-function parameters: the dispatch shim
/// (`cljrs_eval::jit_state::dispatch_jit_call`) supports at most 8 native
/// arguments.
pub const MAX_OSR_PARAMS: usize = 8;

/// An OSR-entry variant of a function, ready for JIT compilation.
pub struct OsrFunction {
    /// The transformed function: entry block jumps to the loop header, loop
    /// state arrives via parameters.
    pub func: IrFunction,
    /// For each parameter of `func` (in order), the *original* `VarId` whose
    /// current value the interpreter must pass when transferring into the
    /// native frame.
    pub live_ins: Vec<VarId>,
}

/// Successor block IDs of a terminator.
fn successors(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Jump(t) => vec![*t],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::RecurJump { target, .. } => vec![*target],
        Terminator::Return(_) | Terminator::Unreachable => vec![],
    }
}

/// VarIds read by a terminator.
fn terminator_uses(term: &Terminator) -> Vec<VarId> {
    match term {
        Terminator::Jump(_) | Terminator::Unreachable => vec![],
        Terminator::Branch { cond, .. } => vec![*cond],
        Terminator::Return(v) => vec![*v],
        Terminator::RecurJump { args, .. } => args.clone(),
    }
}

/// The plain (GC-heap / thread-local-region) allocation equivalent of a
/// `RegionAlloc`, used when the region handle is interpreter-owned and so
/// cannot be materialised in compiled code.
fn plain_alloc(dst: VarId, kind: RegionAllocKind, ops: Vec<VarId>) -> Inst {
    match kind {
        RegionAllocKind::Vector => Inst::AllocVector(dst, ops),
        RegionAllocKind::Set => Inst::AllocSet(dst, ops),
        RegionAllocKind::List => Inst::AllocList(dst, ops),
        RegionAllocKind::Map => {
            Inst::AllocMap(dst, ops.chunks(2).map(|pair| (pair[0], pair[1])).collect())
        }
        RegionAllocKind::Cons => Inst::AllocCons(dst, ops[0], ops[1]),
    }
}

/// Build the OSR-entry variant of `orig` for the loop header `header`.
///
/// Returns an error (leaving the caller at Tier 1) when the loop cannot be
/// soundly or practically OSR-compiled:
/// - `header` is not a block of `orig`;
/// - more than [`MAX_OSR_PARAMS`] live-in values would be required.
pub fn build_osr_function(orig: &IrFunction, header: BlockId) -> Result<OsrFunction, String> {
    let blocks_by_id: HashMap<BlockId, &Block> = orig.blocks.iter().map(|b| (b.id, b)).collect();
    if !blocks_by_id.contains_key(&header) {
        return Err(format!("OSR: header block {header} not found"));
    }

    // 1. Blocks reachable from the header.
    let mut reach: HashSet<BlockId> = HashSet::new();
    let mut queue: VecDeque<BlockId> = VecDeque::new();
    reach.insert(header);
    queue.push_back(header);
    while let Some(bid) = queue.pop_front() {
        let block = blocks_by_id[&bid];
        for succ in successors(&block.terminator) {
            if reach.insert(succ) {
                queue.push_back(succ);
            }
        }
    }

    // Region handles created before the loop (RegionStart outside the
    // reachable set).  References to them inside the loop are rewritten — see
    // the module docs.
    let outer_handles: HashSet<VarId> = orig
        .blocks
        .iter()
        .filter(|b| !reach.contains(&b.id))
        .flat_map(|b| &b.insts)
        .filter_map(|inst| match inst {
            Inst::RegionStart(dst) => Some(*dst),
            _ => None,
        })
        .collect();

    // 2. Defs and uses within the reachable region.  φ entries whose
    //    predecessor lies outside the region are edges that can never be taken
    //    in the OSR function, so their sources do not count as uses.
    //    Interpreter-owned region handles never count as uses: the
    //    instructions naming them are dropped or rewritten in step 4.
    let mut defs: HashSet<VarId> = HashSet::new();
    let mut uses: HashSet<VarId> = HashSet::new();
    for block in orig.blocks.iter().filter(|b| reach.contains(&b.id)) {
        for phi in &block.phis {
            if let Inst::Phi(dst, entries) = phi {
                defs.insert(*dst);
                for (pred, src) in entries {
                    if reach.contains(pred) {
                        uses.insert(*src);
                    }
                }
            }
        }
        for inst in &block.insts {
            if let Some(d) = inst.dst() {
                defs.insert(d);
            }
            for u in inst.uses() {
                if !outer_handles.contains(&u) {
                    uses.insert(u);
                }
            }
        }
        for u in terminator_uses(&block.terminator) {
            uses.insert(u);
        }
    }

    // 3. Parameters.  Loop variables (φ destinations at the header) come
    //    first, fed through *fresh* VarIds wired into the φs; then every value
    //    defined before the loop but read inside it, bound to its original
    //    VarId directly.
    let header_block = blocks_by_id[&header];
    let phi_dsts: Vec<VarId> = header_block
        .phis
        .iter()
        .filter_map(|p| match p {
            Inst::Phi(dst, _) => Some(*dst),
            _ => None,
        })
        .collect();

    let mut outer_live: Vec<VarId> = uses.difference(&defs).copied().collect();
    outer_live.sort_by_key(|v| v.0);

    if phi_dsts.len() + outer_live.len() > MAX_OSR_PARAMS {
        return Err(format!(
            "OSR: {} live-in values exceed the {MAX_OSR_PARAMS}-parameter dispatch limit",
            phi_dsts.len() + outer_live.len()
        ));
    }

    let mut next_var = orig.next_var;
    let mut params: Vec<(Arc<str>, VarId)> = Vec::new();
    let mut live_ins: Vec<VarId> = Vec::new();
    let mut phi_params: Vec<VarId> = Vec::new();
    for (i, dst) in phi_dsts.iter().enumerate() {
        let p = VarId(next_var);
        next_var += 1;
        params.push((Arc::from(format!("__osr_phi{i}")), p));
        live_ins.push(*dst);
        phi_params.push(p);
    }
    for v in &outer_live {
        params.push((Arc::from(format!("__osr_v{}", v.0)), *v));
        live_ins.push(*v);
    }

    // 4. Blocks: a fresh entry that jumps to the header, then the reachable
    //    blocks in their original order with φ edges rewired and
    //    interpreter-owned region references dropped/rewritten.
    //
    //    A `CallWithRegion` threading an interpreter-owned handle cannot be
    //    compiled: native code needs a real `*mut Region` for the hidden
    //    argument and the interpreter's handle register holds nil.  The
    //    stage-4 rewrite never lets a region scope cross a loop back-edge, so
    //    this bail-out is defensive.
    if orig
        .blocks
        .iter()
        .filter(|b| reach.contains(&b.id))
        .flat_map(|b| &b.insts)
        .any(|inst| matches!(inst, Inst::CallWithRegion(_, _, _, r) if outer_handles.contains(r)))
    {
        return Err("OSR: CallWithRegion references an interpreter-owned region".into());
    }

    let entry_id = BlockId(orig.next_block);
    let mut blocks = Vec::with_capacity(reach.len() + 1);
    blocks.push(Block {
        id: entry_id,
        phis: vec![],
        insts: Vec::new(),
        terminator: Terminator::Jump(header),
    });
    for block in orig.blocks.iter().filter(|b| reach.contains(&b.id)) {
        let mut b = block.clone();
        for phi in &mut b.phis {
            if let Inst::Phi(_, entries) = phi {
                entries.retain(|(pred, _)| reach.contains(pred));
            }
        }
        if b.id == header {
            for (phi, param) in b.phis.iter_mut().zip(&phi_params) {
                if let Inst::Phi(_, entries) = phi {
                    entries.push((entry_id, *param));
                }
            }
        }
        // The interpreter owns regions opened before the loop and closes them
        // when its frame unwinds after the transfer returns; the OSR variant
        // must not close them again.
        b.insts
            .retain(|inst| !matches!(inst, Inst::RegionEnd(r) if outer_handles.contains(r)));
        // A compiled region handle is a real `*mut Region`, which the
        // interpreter cannot supply for regions it opened itself — rewrite
        // those `RegionAlloc`s to plain allocs (the region-aware bridges pick
        // the interpreter's still-active region off the thread-local stack).
        for inst in &mut b.insts {
            if let Inst::RegionAlloc(dst, r, kind, ops) = inst
                && outer_handles.contains(r)
            {
                *inst = plain_alloc(*dst, *kind, std::mem::take(ops));
            }
        }
        blocks.push(b);
    }

    let name = format!(
        "{}__osr_bb{}",
        orig.name.as_deref().unwrap_or("<anon>"),
        header.0
    );
    Ok(OsrFunction {
        func: IrFunction {
            name: Some(Arc::from(name)),
            params,
            blocks,
            next_var,
            next_block: orig.next_block + 1,
            span: orig.span.clone(),
            subfunctions: orig.subfunctions.clone(),
            is_async: orig.is_async,
            is_async_poll_fn: orig.is_async_poll_fn,
            async_resume_blocks: orig.async_resume_blocks.clone(),
            // The OSR variant rebinds live-ins as params, so the original
            // positional seeds no longer align; leave it unseeded.
            seed_reprs: Vec::new(),
            local_seed_reprs: Vec::new(),
        },
        live_ins,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Const, KnownFn};

    /// Build the canonical counting loop:
    ///
    /// ```clojure
    /// (fn [n] (loop [i 0 acc 0] (if (< i n) (recur (+ i 1) (+ acc i)) acc)))
    /// ```
    ///
    /// bb0: v1=0; v2=0; jump bb1
    /// bb1: v3=φ[(bb0,v1),(bb2,v6)]; v4=φ[(bb0,v2),(bb2,v7)]
    ///      v5=(< v3 v0); branch v5 bb2 bb3
    /// bb2: v8=1; v6=(+ v3 v8); v7=(+ v4 v3); recur_jump bb1 [v6,v7]
    /// bb3: return v4
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

    #[test]
    fn live_ins_are_loop_vars_then_outer_values() {
        let orig = sum_loop_fn();
        let osr = build_osr_function(&orig, BlockId(1)).unwrap();
        // φ destinations (i, acc) first, then the outer param n.
        assert_eq!(osr.live_ins, vec![VarId(3), VarId(4), VarId(0)]);
        assert_eq!(osr.func.params.len(), 3);
        // The init constants v1/v2 are pre-loop only and must not be live-ins.
        assert!(!osr.live_ins.contains(&VarId(1)));
        assert!(!osr.live_ins.contains(&VarId(2)));
    }

    #[test]
    fn entry_block_jumps_to_header_and_phis_are_rewired() {
        let orig = sum_loop_fn();
        let osr = build_osr_function(&orig, BlockId(1)).unwrap();
        // blocks[0] is the fresh OSR entry.
        let entry = &osr.func.blocks[0];
        assert_eq!(entry.id, BlockId(4));
        assert!(entry.phis.is_empty() && entry.insts.is_empty());
        assert!(matches!(entry.terminator, Terminator::Jump(BlockId(1))));
        // The pre-loop block bb0 is dropped.
        assert!(osr.func.blocks.iter().all(|b| b.id != BlockId(0)));

        // Header φs: the bb0 edge is gone, the back-edge stays, and a new edge
        // from the OSR entry feeds each loop variable from its fresh param.
        let header = osr.func.blocks.iter().find(|b| b.id == BlockId(1)).unwrap();
        let phi_i = match &header.phis[0] {
            Inst::Phi(dst, entries) => {
                assert_eq!(*dst, VarId(3));
                entries.clone()
            }
            other => panic!("expected phi, got {other}"),
        };
        assert!(!phi_i.iter().any(|(pred, _)| *pred == BlockId(0)));
        assert!(phi_i.contains(&(BlockId(2), VarId(6))));
        let osr_edge = phi_i
            .iter()
            .find(|(pred, _)| *pred == BlockId(4))
            .expect("OSR entry edge");
        // The φ param VarId is fresh and matches params[0].
        assert_eq!(osr_edge.1, osr.func.params[0].1);
        assert!(osr_edge.1.0 >= 9);
    }

    #[test]
    fn header_must_exist() {
        let orig = sum_loop_fn();
        assert!(build_osr_function(&orig, BlockId(99)).is_err());
    }

    #[test]
    fn interpreter_owned_region_end_is_dropped() {
        // Mirror the optimizer's function-level region wrap: RegionStart in
        // the pre-loop entry, RegionEnd in the post-loop exit.
        let mut orig = sum_loop_fn();
        let handle = VarId(20);
        orig.next_var = 21;
        orig.blocks[0].insts.insert(0, Inst::RegionStart(handle));
        orig.blocks[3].insts.push(Inst::RegionEnd(handle));

        let osr = build_osr_function(&orig, BlockId(1)).unwrap();
        // The handle must not leak into the live-ins (it would waste a param
        // and the interpreter binds it to nil anyway)…
        assert!(!osr.live_ins.contains(&handle));
        // …and the unmatched RegionEnd is gone: the interpreter closes the
        // region when its frame unwinds after the transfer returns.
        let exit = osr.func.blocks.iter().find(|b| b.id == BlockId(3)).unwrap();
        assert!(
            !exit.insts.iter().any(|i| matches!(i, Inst::RegionEnd(_))),
            "unmatched RegionEnd must be dropped from the OSR variant"
        );
    }

    #[test]
    fn interpreter_owned_region_alloc_is_rewritten_to_plain_alloc() {
        // RegionStart in the pre-loop entry, RegionAlloc inside the loop body
        // naming that handle.  Compiled region handles are real `*mut Region`
        // values the interpreter cannot supply, so the OSR variant must
        // rewrite the alloc to a plain (region-stack-aware) AllocVector.
        let mut orig = sum_loop_fn();
        let handle = VarId(20);
        let dst = VarId(21);
        orig.next_var = 22;
        orig.blocks[0].insts.insert(0, Inst::RegionStart(handle));
        orig.blocks[2].insts.insert(
            0,
            Inst::RegionAlloc(dst, handle, crate::RegionAllocKind::Vector, vec![VarId(3)]),
        );
        orig.blocks[3].insts.push(Inst::RegionEnd(handle));

        let osr = build_osr_function(&orig, BlockId(1)).unwrap();
        assert!(!osr.live_ins.contains(&handle));
        let body = osr.func.blocks.iter().find(|b| b.id == BlockId(2)).unwrap();
        assert!(
            body.insts.iter().any(
                |i| matches!(i, Inst::AllocVector(d, ops) if *d == dst && ops == &vec![VarId(3)])
            ),
            "interpreter-owned RegionAlloc must become a plain AllocVector"
        );
        assert!(
            !body
                .insts
                .iter()
                .any(|i| matches!(i, Inst::RegionAlloc(..))),
            "no RegionAlloc naming an interpreter-owned handle may survive"
        );
    }

    #[test]
    fn call_with_region_naming_outer_handle_declines() {
        // A CallWithRegion threading an interpreter-owned handle cannot be
        // compiled (native code needs a real region pointer for the hidden
        // argument) — the transform must decline so the loop stays at Tier 1.
        let mut orig = sum_loop_fn();
        let handle = VarId(20);
        let dst = VarId(21);
        orig.next_var = 22;
        orig.blocks[0].insts.insert(0, Inst::RegionStart(handle));
        orig.blocks[2].insts.insert(
            0,
            Inst::CallWithRegion(dst, Arc::from("callee__rg1"), vec![VarId(3)], handle),
        );
        orig.blocks[3].insts.push(Inst::RegionEnd(handle));

        assert!(build_osr_function(&orig, BlockId(1)).is_err());
    }

    #[test]
    fn loop_local_region_pairs_are_kept() {
        // A region opened and closed entirely inside the loop body must keep
        // both ends — native code manages it like any other instruction.
        let mut orig = sum_loop_fn();
        let handle = VarId(20);
        orig.next_var = 21;
        orig.blocks[2].insts.insert(0, Inst::RegionStart(handle));
        orig.blocks[2].insts.push(Inst::RegionEnd(handle));

        let osr = build_osr_function(&orig, BlockId(1)).unwrap();
        let body = osr.func.blocks.iter().find(|b| b.id == BlockId(2)).unwrap();
        assert!(body.insts.iter().any(|i| matches!(i, Inst::RegionStart(_))));
        assert!(body.insts.iter().any(|i| matches!(i, Inst::RegionEnd(_))));
    }

    #[test]
    fn too_many_live_ins_decline() {
        // A self-looping header with 9 φs blows the 8-arg dispatch limit.
        let mut f = IrFunction::new(Some(Arc::from("wide")), None);
        let phis = (0..9u32)
            .map(|i| Inst::Phi(VarId(i), vec![(BlockId(0), VarId(i + 16))]))
            .collect();
        f.next_var = 32;
        f.next_block = 1;
        f.blocks = vec![Block {
            id: BlockId(0),
            phis,
            insts: (0..9u32)
                .map(|i| Inst::Const(VarId(i + 16), Const::Long(0)))
                .collect(),
            terminator: Terminator::RecurJump {
                target: BlockId(0),
                args: (16..25).map(VarId).collect(),
            },
        }];
        assert!(build_osr_function(&f, BlockId(0)).is_err());
    }
}
