//! Async state-machine lowering (Phase H).
//!
//! [`lower_async`] rewrites an `^:async` [`IrFunction`] into an equivalent
//! *poll function* — a non-async `IrFunction` (`is_async_poll_fn = true`) whose
//! control flow is an explicit, resumable state machine, the way the Rust
//! compiler desugars `async fn` into a `Future::poll` state machine.
//!
//! ## Shape of the transform
//!
//! Every `await` is a *suspend point*.  We split each basic block so that each
//! suspend is the last operation of a *segment*, then thread the values that
//! are live across the suspend through a heap-resident slot array on the
//! `CljxStateMachine` (the GC traces those slots while the task is parked):
//!
//! ```text
//!   <pre-suspend insts>                      ; segment, state s
//!   state_store [slot] vLive   (× each live var)
//!   suspend.await [src] -> state k           ; register src, set state=k, return Pending
//!   ── (block ends; resumed via the codegen dispatch prologue) ──
//!   vAwaited = resume.await                   ; resume block, state k
//!   vLive = state_load [slot]  (× each live var)
//!   <continuation>
//! ```
//!
//! The poll function takes **no Clojure parameters**: the original parameters
//! arrive materialised in the state machine's slots (placed there by the
//! dispatcher when it builds the `CljxStateMachine`), so the entry block
//! `state_load`s each one.  Codegen emits a `switch(state)` prologue over
//! [`IrFunction::async_resume_blocks`] that jumps to the right resume block.
//!
//! ## Phis
//!
//! The IR keeps SSA phi nodes (codegen turns them into Cranelift block
//! parameters carried on terminator edges).  Resume blocks are created *fresh
//! and phi-free*, and they are reached only by the synthetic dispatch jump — so
//! no dispatch edge ever needs to supply phi arguments.  Loop-carried values
//! (which are phis at a loop header) are simply saved/restored as ordinary
//! live-across-suspend values; the normal `recur` back-edge keeps supplying the
//! header's phi arguments as before.
//!
//! ## Scope
//!
//! This pass currently lowers `await` only (phases H1–H3: straight-line,
//! `if`/`let`, and `loop`/`recur`).  Channel and `spawn` suspends return
//! [`AsyncLowerError::Unsupported`] so the enclosing function keeps its
//! tree-walking `eval_async` fallback.

use std::collections::{HashMap, HashSet};

use crate::{Block, BlockId, Inst, IrFunction, SuspendKind, Terminator, VarId};

/// Reason a function could not be lowered to a state machine; the caller keeps
/// the interpreter (`eval_async`) fallback for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsyncLowerError {
    /// A construct outside the current state-machine scope (channels, spawn).
    Unsupported(String),
}

impl std::fmt::Display for AsyncLowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AsyncLowerError::Unsupported(what) => {
                write!(f, "async state-machine lowering unsupported: {what}")
            }
        }
    }
}

/// The result of lowering an `^:async` function.
#[derive(Debug)]
pub struct AsyncLowering {
    /// The poll function (`is_async_poll_fn = true`), ready for codegen.
    pub poll_fn: IrFunction,
    /// Number of value slots the `CljxStateMachine` must allocate.  Slots
    /// `0..param_count` hold the original parameters (filled by the dispatcher);
    /// the rest hold values that cross a suspend.
    pub n_slots: usize,
    /// Number of original parameters (the leading slots).
    pub param_count: usize,
}

/// Lower an `^:async` `IrFunction` into a poll function, or report why it can't
/// be lowered (the caller then keeps the `eval_async` fallback).
pub fn lower_async(f: &IrFunction) -> Result<AsyncLowering, AsyncLowerError> {
    reject_unsupported(f)?;

    let mut pf = f.clone();
    pf.is_async = false;
    pf.is_async_poll_fn = true;

    let entry_id = pf
        .blocks
        .first()
        .map(|b| b.id)
        .ok_or_else(|| AsyncLowerError::Unsupported("empty function body".into()))?;

    // ── 1. Split blocks at each `await`. ────────────────────────────────────
    let orig_blocks = std::mem::take(&mut pf.blocks);
    let mut new_blocks: Vec<Block> = Vec::new();
    // resume_block per suspend, in creation order; state `k` (1-based) maps to
    // resume_states[k-1].
    let mut resume_states: Vec<BlockId> = Vec::new();
    // (suspend_block_id, resume_block_id) so we can attach save/restore later.
    let mut suspend_resume: Vec<(BlockId, BlockId)> = Vec::new();
    // Original block id → id of its *last* segment.  When a block is split, its
    // outgoing edges (and so its identity as a phi predecessor) move to the last
    // segment, so phi entries referencing the original must be remapped.
    let mut last_segment: HashMap<BlockId, BlockId> = HashMap::new();

    for b in &orig_blocks {
        let mut current_id = b.id;
        let mut current_insts: Vec<Inst> = Vec::new();

        for inst in &b.insts {
            match inst {
                Inst::Await { src, dst } => {
                    let resume_id = pf.fresh_block();
                    let state = (resume_states.len() + 1) as u32;
                    resume_states.push(resume_id);
                    suspend_resume.push((current_id, resume_id));

                    current_insts.push(Inst::AsyncSuspend {
                        kind: SuspendKind::Await,
                        operands: vec![*src],
                        next_state: state,
                    });
                    new_blocks.push(Block {
                        id: current_id,
                        // Only the first segment of a split inherits the
                        // original block's phis; they execute before any suspend.
                        phis: if current_id == b.id {
                            b.phis.clone()
                        } else {
                            Vec::new()
                        },
                        insts: std::mem::take(&mut current_insts),
                        // The suspend returns Pending; codegen never reaches this.
                        terminator: Terminator::Unreachable,
                    });

                    current_id = resume_id;
                    current_insts.push(Inst::AsyncResume {
                        dst: *dst,
                        kind: SuspendKind::Await,
                    });
                }
                other => current_insts.push(other.clone()),
            }
        }

        new_blocks.push(Block {
            id: current_id,
            phis: if current_id == b.id {
                b.phis.clone()
            } else {
                Vec::new()
            },
            insts: current_insts,
            terminator: b.terminator.clone(),
        });
        last_segment.insert(b.id, current_id);
    }

    // Remap phi predecessor ids: a phi entry `(P, v)` whose predecessor `P` was
    // split now arrives from `P`'s last segment.
    for block in &mut new_blocks {
        for phi in &mut block.phis {
            if let Inst::Phi(_, entries) = phi {
                for (pred, _) in entries.iter_mut() {
                    if let Some(seg) = last_segment.get(pred) {
                        *pred = *seg;
                    }
                }
            }
        }
    }

    // ── 2. Live-variable analysis over the split CFG (SSA phi-edge form). ─────
    let live_in = compute_live_in(&new_blocks);

    // ── 3. Slot assignment: parameters first (slots 0..param_count), then
    //       every value that is live into a resume block. ────────────────────
    let param_count = pf.params.len();
    let mut slots: HashMap<VarId, u32> = HashMap::new();
    for (i, (_, v)) in pf.params.iter().enumerate() {
        slots.insert(*v, i as u32);
    }
    let mut next_slot = param_count as u32;
    let mut save_sets: HashMap<BlockId, Vec<VarId>> = HashMap::new();
    for (_suspend_id, resume_id) in &suspend_resume {
        let mut saved: Vec<VarId> = live_in
            .get(resume_id)
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default();
        // Deterministic order keeps slot assignment and tests stable.
        saved.sort_by_key(|v| v.0);
        for v in &saved {
            slots.entry(*v).or_insert_with(|| {
                let s = next_slot;
                next_slot += 1;
                s
            });
        }
        save_sets.insert(*resume_id, saved);
    }

    // ── 4. Insert state_store / state_load and load parameters at entry. ─────
    let resume_of: HashMap<BlockId, BlockId> = suspend_resume.iter().copied().collect();
    for block in &mut new_blocks {
        // Save live values immediately before the suspend.
        if let Some(resume_id) = resume_of.get(&block.id) {
            let saved = &save_sets[resume_id];
            let stores: Vec<Inst> = saved
                .iter()
                .map(|v| Inst::StateStore {
                    slot: slots[v],
                    val: *v,
                })
                .collect();
            // AsyncSuspend is the last inst; splice stores in just before it.
            let at = block.insts.len() - 1;
            splice(&mut block.insts, at, stores);
        }
        // Restore live values immediately after the resume.
        if let Some(saved) = save_sets.get(&block.id) {
            let loads: Vec<Inst> = saved
                .iter()
                .map(|v| Inst::StateLoad {
                    dst: *v,
                    slot: slots[v],
                })
                .collect();
            // AsyncResume is the first inst; splice loads in just after it.
            splice(&mut block.insts, 1, loads);
        }
    }

    // Entry block: load every parameter from its slot before any body code.
    if let Some(entry) = new_blocks.iter_mut().find(|b| b.id == entry_id) {
        let loads: Vec<Inst> = pf
            .params
            .iter()
            .map(|(_, v)| Inst::StateLoad {
                dst: *v,
                slot: slots[v],
            })
            .collect();
        splice(&mut entry.insts, 0, loads);
    }

    // ── 5. Finalise the poll function. ──────────────────────────────────────
    pf.blocks = new_blocks;
    // The poll-fn ABI is `(state, out) -> i32`; parameters live in slots now.
    pf.params = Vec::new();
    pf.seed_reprs = Vec::new();
    pf.local_seed_reprs = Vec::new();

    let mut resume_blocks = Vec::with_capacity(resume_states.len() + 1);
    resume_blocks.push(entry_id);
    resume_blocks.extend(resume_states);
    pf.async_resume_blocks = resume_blocks;

    Ok(AsyncLowering {
        poll_fn: pf,
        n_slots: next_slot as usize,
        param_count,
    })
}

/// Reject suspends outside the current scope so the caller keeps the
/// interpreter fallback.
fn reject_unsupported(f: &IrFunction) -> Result<(), AsyncLowerError> {
    for b in &f.blocks {
        for inst in &b.insts {
            match inst {
                Inst::Spawn { .. } => {
                    return Err(AsyncLowerError::Unsupported("spawn".into()));
                }
                Inst::ChanTake { .. } => {
                    return Err(AsyncLowerError::Unsupported("channel take (<!)".into()));
                }
                Inst::ChanPut { .. } => {
                    return Err(AsyncLowerError::Unsupported("channel put (>!)".into()));
                }
                // `throw` across a suspend needs the poll-fn exception path
                // (deferred with try/catch, H5); keep such fns interpreted.
                Inst::Throw(_) => {
                    return Err(AsyncLowerError::Unsupported("throw".into()));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Insert `items` into `v` at index `at`, preserving order.
fn splice(v: &mut Vec<Inst>, at: usize, items: Vec<Inst>) {
    if items.is_empty() {
        return;
    }
    let tail = v.split_off(at);
    v.extend(items);
    v.extend(tail);
}

/// Successor blocks of a terminator.
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

/// `VarId`s used by a terminator (live at the very end of the block).
fn terminator_uses(term: &Terminator) -> Vec<VarId> {
    match term {
        Terminator::Branch { cond, .. } => vec![*cond],
        Terminator::Return(v) => vec![*v],
        // `recur` args feed the loop header's phis; keep them live at the edge.
        Terminator::RecurJump { args, .. } => args.clone(),
        Terminator::Jump(_) | Terminator::Unreachable => vec![],
    }
}

/// Backward live-variable analysis over SSA-with-phis, returning the set of
/// `VarId`s live on entry to each block.  Phi operands are attributed to the
/// matching predecessor *edge* (not the phi's own block), so a loop's initial
/// value is not spuriously kept live around the back-edge.
fn compute_live_in(blocks: &[Block]) -> HashMap<BlockId, HashSet<VarId>> {
    let by_id: HashMap<BlockId, &Block> = blocks.iter().map(|b| (b.id, b)).collect();
    let phi_defs: HashMap<BlockId, HashSet<VarId>> = blocks
        .iter()
        .map(|b| {
            let defs = b
                .phis
                .iter()
                .filter_map(|p| p.dst())
                .collect::<HashSet<_>>();
            (b.id, defs)
        })
        .collect();

    let mut live_in: HashMap<BlockId, HashSet<VarId>> =
        blocks.iter().map(|b| (b.id, HashSet::new())).collect();

    let mut changed = true;
    while changed {
        changed = false;
        for b in blocks.iter().rev() {
            // live_out: from each successor S, take live_in(S) minus S's phi
            // defs, plus the phi operands that this block supplies to S.
            let mut live: HashSet<VarId> = HashSet::new();
            for succ in successors(&b.terminator) {
                if let Some(s_live) = live_in.get(&succ) {
                    let s_phi_defs = phi_defs.get(&succ);
                    for v in s_live {
                        if s_phi_defs.map(|d| !d.contains(v)).unwrap_or(true) {
                            live.insert(*v);
                        }
                    }
                }
                if let Some(sblock) = by_id.get(&succ) {
                    for phi in &sblock.phis {
                        if let Inst::Phi(_, entries) = phi {
                            for (pred, val) in entries {
                                if *pred == b.id {
                                    live.insert(*val);
                                }
                            }
                        }
                    }
                }
            }
            // Terminator operands are live at the end of the block.
            for u in terminator_uses(&b.terminator) {
                live.insert(u);
            }
            // Walk the body backwards.
            for inst in b.insts.iter().rev() {
                if let Some(d) = inst.dst() {
                    live.remove(&d);
                }
                for u in inst.uses() {
                    live.insert(u);
                }
            }
            // Phi results are defined at the top of this block.
            if let Some(defs) = phi_defs.get(&b.id) {
                for d in defs {
                    live.remove(d);
                }
            }
            let slot = live_in.get_mut(&b.id).unwrap();
            if *slot != live {
                *slot = live;
                changed = true;
            }
        }
    }
    live_in
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Const;
    use std::sync::Arc;

    fn count<F: Fn(&Inst) -> bool>(f: &IrFunction, pred: F) -> usize {
        f.blocks
            .iter()
            .flat_map(|b| b.insts.iter())
            .filter(|i| pred(i))
            .count()
    }

    fn is_suspend(i: &Inst) -> bool {
        matches!(i, Inst::AsyncSuspend { .. })
    }
    fn is_resume(i: &Inst) -> bool {
        matches!(i, Inst::AsyncResume { .. })
    }
    fn is_store(i: &Inst) -> bool {
        matches!(i, Inst::StateStore { .. })
    }
    fn is_load(i: &Inst) -> bool {
        matches!(i, Inst::StateLoad { .. })
    }

    /// `(defn ^:async f [] (inc (await x)))` shape: one straight-line await.
    #[test]
    fn single_await_splits_into_two_states() {
        let mut f = IrFunction::new(Some(Arc::from("f")), None);
        f.is_async = true;
        let v_fut = f.fresh_var();
        let v_res = f.fresh_var();
        let v_out = f.fresh_var();
        let entry = f.fresh_block();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![
                Inst::Const(v_fut, Const::Nil),
                Inst::Await {
                    src: v_fut,
                    dst: v_res,
                },
                Inst::CallKnown(v_out, crate::KnownFn::Add, vec![v_res, v_res]),
            ],
            terminator: Terminator::Return(v_out),
        });

        let low = lower_async(&f).expect("lowers");
        let pf = &low.poll_fn;
        assert!(pf.is_async_poll_fn);
        assert!(!pf.is_async);
        assert!(pf.params.is_empty());
        // state 0 (entry) + state 1 (resume).
        assert_eq!(pf.async_resume_blocks.len(), 2);
        assert_eq!(pf.async_resume_blocks[0], entry);
        assert_eq!(count(pf, is_suspend), 1);
        assert_eq!(count(pf, is_resume), 1);
        // The suspend records next_state = 1, and resume_blocks[1] is its block.
        let (susp_block, susp) = pf
            .blocks
            .iter()
            .find_map(|b| {
                b.insts.iter().find_map(|i| match i {
                    Inst::AsyncSuspend { next_state, .. } => Some((b.id, *next_state)),
                    _ => None,
                })
            })
            .unwrap();
        assert_eq!(susp, 1);
        // The suspend is the last instruction of its block.
        let sb = pf.blocks.iter().find(|b| b.id == susp_block).unwrap();
        assert!(is_suspend(sb.insts.last().unwrap()));
        assert!(matches!(sb.terminator, Terminator::Unreachable));
    }

    /// A value defined before the await and used after it must be saved/restored.
    #[test]
    fn value_live_across_await_is_saved_and_restored() {
        let mut f = IrFunction::new(Some(Arc::from("f")), None);
        f.is_async = true;
        let v_keep = f.fresh_var();
        let v_fut = f.fresh_var();
        let v_res = f.fresh_var();
        let v_out = f.fresh_var();
        let entry = f.fresh_block();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![
                Inst::Const(v_keep, Const::Long(7)),
                Inst::Const(v_fut, Const::Nil),
                Inst::Await {
                    src: v_fut,
                    dst: v_res,
                },
                // uses BOTH the awaited result and the pre-await value.
                Inst::CallKnown(v_out, crate::KnownFn::Add, vec![v_keep, v_res]),
            ],
            terminator: Terminator::Return(v_out),
        });

        let low = lower_async(&f).expect("lowers");
        let pf = &low.poll_fn;
        // v_keep is live across the suspend → exactly one store and one load.
        assert_eq!(count(pf, is_store), 1, "v_keep saved before suspend");
        assert_eq!(count(pf, is_load), 1, "v_keep restored after resume");
        // No parameters here, so the only slot is for v_keep.
        assert_eq!(low.param_count, 0);
        assert_eq!(low.n_slots, 1);
        // Store happens just before the suspend; load just after the resume.
        for b in &pf.blocks {
            for w in b.insts.windows(2) {
                if is_store(&w[0]) {
                    assert!(is_suspend(&w[1]));
                }
                if is_resume(&w[0]) {
                    assert!(is_load(&w[1]));
                }
            }
        }
    }

    /// Parameters are materialised from slots at the entry block, and the poll
    /// function exposes no Clojure parameters.
    #[test]
    fn parameters_are_loaded_from_slots_at_entry() {
        let mut f = IrFunction::new(Some(Arc::from("f")), None);
        f.is_async = true;
        let p = f.fresh_var();
        f.params.push((Arc::from("x"), p));
        let v_res = f.fresh_var();
        let entry = f.fresh_block();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![Inst::Await { src: p, dst: v_res }],
            terminator: Terminator::Return(v_res),
        });

        let low = lower_async(&f).expect("lowers");
        let pf = &low.poll_fn;
        assert!(pf.params.is_empty());
        assert_eq!(low.param_count, 1);
        // The entry block begins by loading the parameter from slot 0.
        let eb = pf.blocks.iter().find(|b| b.id == entry).unwrap();
        match &eb.insts[0] {
            Inst::StateLoad { dst, slot } => {
                assert_eq!(*dst, p);
                assert_eq!(*slot, 0);
            }
            other => panic!("expected entry to start with state_load, got {other:?}"),
        }
    }

    /// A function with no awaits still lowers to a (single-state) poll function.
    #[test]
    fn no_await_lowers_to_single_state() {
        let mut f = IrFunction::new(Some(Arc::from("f")), None);
        f.is_async = true;
        let v = f.fresh_var();
        let entry = f.fresh_block();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![Inst::Const(v, Const::Long(1))],
            terminator: Terminator::Return(v),
        });

        let low = lower_async(&f).expect("lowers");
        assert_eq!(low.poll_fn.async_resume_blocks.len(), 1);
        assert_eq!(count(&low.poll_fn, is_suspend), 0);
    }

    /// A loop whose body awaits: the loop-carried value (a phi at the header)
    /// must be saved/restored across the suspend, the header keeps its phi, and
    /// the freshly-created resume block carries no phis (so the dispatch jump
    /// never needs phi arguments).
    #[test]
    fn loop_carried_value_crosses_await() {
        let mut f = IrFunction::new(Some(Arc::from("f")), None);
        f.is_async = true;
        let v_init = f.fresh_var();
        let v_count = f.fresh_var(); // phi at header
        let v_fut = f.fresh_var();
        let v_res = f.fresh_var();
        let v_next = f.fresh_var();
        let b_entry = f.fresh_block();
        let b_head = f.fresh_block();
        let b_exit = f.fresh_block();

        f.blocks.push(Block {
            id: b_entry,
            phis: vec![],
            insts: vec![Inst::Const(v_init, Const::Long(0))],
            terminator: Terminator::Jump(b_head),
        });
        f.blocks.push(Block {
            id: b_head,
            phis: vec![Inst::Phi(
                v_count,
                vec![(b_entry, v_init), (b_head, v_next)],
            )],
            insts: vec![
                Inst::Const(v_fut, Const::Nil),
                Inst::Await {
                    src: v_fut,
                    dst: v_res,
                },
                // loop-carried value v_count is read AFTER the await.
                Inst::CallKnown(v_next, crate::KnownFn::Add, vec![v_count, v_res]),
            ],
            terminator: Terminator::Branch {
                cond: v_next,
                then_block: b_head,
                else_block: b_exit,
            },
        });
        f.blocks.push(Block {
            id: b_exit,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(v_count),
        });

        let low = lower_async(&f).expect("lowers");
        let pf = &low.poll_fn;
        assert_eq!(pf.async_resume_blocks.len(), 2);
        // The header keeps its phi.
        let head = pf.blocks.iter().find(|b| b.id == b_head).unwrap();
        assert_eq!(head.phis.len(), 1, "loop header keeps its phi");
        // The resume block (state 1) is phi-free.
        let resume_id = pf.async_resume_blocks[1];
        let resume = pf.blocks.iter().find(|b| b.id == resume_id).unwrap();
        assert!(resume.phis.is_empty(), "resume block must be phi-free");
        // v_count is live across the suspend → saved and restored.
        assert_eq!(count(pf, is_store), 1);
        assert_eq!(count(pf, is_load), 1);
    }

    /// Channel/spawn suspends are out of scope and keep the interpreter fallback.
    #[test]
    fn channel_and_spawn_are_unsupported() {
        for inst in [
            Inst::ChanTake {
                chan: VarId(0),
                dst: VarId(1),
            },
            Inst::ChanPut {
                chan: VarId(0),
                val: VarId(1),
            },
            Inst::Spawn {
                fn_reg: VarId(0),
                args: vec![],
                dst: VarId(1),
            },
        ] {
            let mut f = IrFunction::new(Some(Arc::from("f")), None);
            f.is_async = true;
            let entry = f.fresh_block();
            f.blocks.push(Block {
                id: entry,
                phis: vec![],
                insts: vec![inst],
                terminator: Terminator::Return(VarId(1)),
            });
            assert!(matches!(
                lower_async(&f),
                Err(AsyncLowerError::Unsupported(_))
            ));
        }
    }
}
