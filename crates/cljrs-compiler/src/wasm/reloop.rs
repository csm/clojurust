//! Relooper: recover structured control flow from the IR's CFG.
//!
//! wasm has only structured control flow — `block`/`loop`/`if` with labeled
//! `br`/`br_if`, no arbitrary `goto`.  The `cljrs-ir` CFG ([`IrFunction`],
//! [`Block`], [`Terminator`]) has arbitrary edges, so the emitter needs a
//! structured tree to walk.  This pass produces one.
//!
//! It is **wasm-private**: the Cranelift backend consumes the raw CFG directly
//! and would be pessimized by re-structuring, so this lives in the wasm backend
//! rather than in shared lowering.
//!
//! # Algorithm
//!
//! This implements the dominator-tree structuring of Norman Ramsey's *"Beyond
//! Relooper: Recursive Translation of Unstructured Control Flow to Structured
//! Control Flow"* (ICFP 2022), specialized to the two facts that hold for every
//! CFG this backend sees:
//!
//! - **Back edges are exactly [`Terminator::RecurJump`].**  Clojure has no
//!   `goto`; the only cyclic control flow is `loop`/`recur`, which lowering
//!   emits as a `RecurJump` to the loop header.  So loop headers are precisely
//!   the `RecurJump` targets, and every `Jump`/`Branch` edge is forward.
//! - **The CFG is reducible.**  Structured source + reducible-preserving
//!   inlining can't manufacture irreducibility, so the relooper never needs
//!   node-splitting or a dispatch variable.
//!
//! The translation is driven by the dominator tree:
//!
//! - A node `X` is translated by [`Relooper::do_tree`].  If `X` is a loop
//!   header it is wrapped in a [`Structured::Loop`]; a back edge to `X` becomes
//!   a `br`(continue) to that loop's label.
//! - A **merge node** (≥2 *forward* predecessors) cannot be inlined at a single
//!   branch, so it is emitted once, after the code that branches to it, wrapped
//!   in a labeled [`Structured::Labeled`] block placed at its immediate
//!   dominator.  Branches to it become `br`(break) to that block's label.
//! - Merge children of one node are nested in **ascending reverse-postorder**
//!   (largest RPO outermost), which guarantees every `br` jumps *forward* out of
//!   enclosing blocks — the only thing wasm permits.
//! - Any other forward edge targets a node with a single forward predecessor,
//!   so it is **inlined** directly ([`Relooper::do_branch`]).
//!
//! Each block is therefore emitted exactly once: merge nodes via their dominator
//! [`Structured::Labeled`], every other reachable node by inlining.
//!
//! # Status
//!
//! Implemented for reducible CFGs (the universal case here): straight-line code,
//! `if`/`cond` diamonds, sequential and nested merges, and `loop`/`recur` loops
//! including loops with conditional exits.  A forward edge that is actually a
//! back edge in reverse-postorder (the signature of irreducible control flow)
//! returns [`RelooperError::Irreducible`].

use std::collections::{HashMap, HashSet};

use crate::ir::{Block, BlockId, IrFunction, Terminator, VarId};

/// A structured control-flow tree — the relooper's output and the emitter's
/// input.  Each node maps to a small, fixed wasm shape.  `br` targets are block
/// ids; the emitter resolves them to `br` depths from its label stack while
/// walking (a forward target is an enclosing [`Structured::Labeled`]; a backward
/// target is an enclosing [`Structured::Loop`]).
#[derive(Debug, Clone)]
pub enum Structured {
    /// Emit one IR block's straight-line body (`phis` + `insts`), then `next`
    /// (the translation of its terminator).  The block's own terminator is
    /// represented by the surrounding nodes, never re-emitted here.
    Simple {
        block: BlockId,
        next: Box<Structured>,
    },
    /// A labeled `block` construct: a `br`(break) to `label` exits to `next`.
    /// Used to place a forward merge node `label` after the `body` that branches
    /// to it.  `label` is the merge block's id; `next` is its translation.
    Labeled {
        label: BlockId,
        body: Box<Structured>,
        next: Box<Structured>,
    },
    /// A `loop` construct headed by `header`.  A `br`(continue) to `header`
    /// re-enters the loop; falling off the end of `body` exits it (wasm `loop`
    /// only repeats on an explicit back-`br`).
    Loop {
        header: BlockId,
        body: Box<Structured>,
    },
    /// Structured two-way branch.  The arms are self-contained — each ends in a
    /// `Br`, `Return`, `Unreachable`, or an inlined subtree — so there is no
    /// fall-through merge here; a re-joining merge is handled by an enclosing
    /// [`Structured::Labeled`].
    If {
        cond: VarId,
        then_arm: Box<Structured>,
        else_arm: Box<Structured>,
    },
    /// `br` to `target`: a forward break to an enclosing labeled block, or a
    /// backward continue to an enclosing loop header.  The emitter distinguishes
    /// the two by which construct in its label stack carries `target`.
    Br(BlockId),
    /// Return a value from the function.
    Return(VarId),
    /// Unreachable terminator (e.g. after `throw`).
    Unreachable,
    /// Empty continuation.
    Nil,
}

/// Errors from the relooper.
#[derive(Debug)]
pub enum RelooperError {
    /// The function has no blocks.
    Empty,
    /// A terminator referenced a block not present in the function.
    DanglingTarget(BlockId),
    /// A forward (`Jump`/`Branch`) edge runs backward in reverse-postorder —
    /// the signature of irreducible control flow, which this backend does not
    /// support (and which Clojure source cannot produce).
    Irreducible { from: BlockId, to: BlockId },
}

impl std::fmt::Display for RelooperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelooperError::Empty => write!(f, "function has no blocks"),
            RelooperError::DanglingTarget(b) => write!(f, "terminator targets missing block {b}"),
            RelooperError::Irreducible { from, to } => {
                write!(
                    f,
                    "irreducible control flow: forward edge {from} -> {to} runs backward"
                )
            }
        }
    }
}

/// Structure the control flow of `func` into a [`Structured`] tree.
pub fn reloop(func: &IrFunction) -> Result<Structured, RelooperError> {
    if func.blocks.is_empty() {
        return Err(RelooperError::Empty);
    }
    let r = Relooper::analyze(func)?;
    r.do_tree(r.entry)
}

// ── Analysis ─────────────────────────────────────────────────────────────────

/// Dominator-tree analysis plus the structuring recursion over one
/// [`IrFunction`].
struct Relooper<'f> {
    func: &'f IrFunction,
    entry: BlockId,
    index: HashMap<BlockId, usize>,
    /// Reverse-postorder rank; smaller is closer to the entry.  Only reachable
    /// blocks appear.
    rpo_num: HashMap<BlockId, u32>,
    /// Number of *forward* (non-`RecurJump`) predecessors of each block.
    forward_preds: HashMap<BlockId, usize>,
    /// Loop headers: the set of `RecurJump` targets.
    loop_headers: HashSet<BlockId>,
    /// For each block, its dominator-tree children that are merge nodes, sorted
    /// by ascending `rpo_num` (largest RPO last → outermost block).
    merge_children: HashMap<BlockId, Vec<BlockId>>,
}

impl<'f> Relooper<'f> {
    fn analyze(func: &'f IrFunction) -> Result<Self, RelooperError> {
        let entry = func.blocks[0].id;
        let mut index = HashMap::new();
        for (i, b) in func.blocks.iter().enumerate() {
            index.insert(b.id, i);
        }

        // Validate all terminator targets exist up front.
        for b in &func.blocks {
            for s in successors(&b.terminator) {
                if !index.contains_key(&s) {
                    return Err(RelooperError::DanglingTarget(s));
                }
            }
        }

        // Reverse postorder over all edges (back edges included; DFS handles
        // the cycles via the visited set).
        let post = postorder(func, entry, &index);
        let mut rpo_num = HashMap::new();
        for (rank, id) in post.iter().rev().enumerate() {
            rpo_num.insert(*id, rank as u32);
        }

        // Predecessors (all edges) for the dominator fixpoint.
        let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        // Forward-only predecessor counts + loop headers.
        let mut forward_preds: HashMap<BlockId, usize> = HashMap::new();
        let mut loop_headers = HashSet::new();
        for b in &func.blocks {
            match &b.terminator {
                Terminator::RecurJump { target, .. } => {
                    preds.entry(*target).or_default().push(b.id);
                    loop_headers.insert(*target);
                }
                term => {
                    for s in successors(term) {
                        preds.entry(s).or_default().push(b.id);
                        *forward_preds.entry(s).or_default() += 1;
                    }
                }
            }
        }

        let idom = dominators(entry, &post, &rpo_num, &preds);

        // Group merge nodes under their immediate dominator, ascending RPO.
        let mut merge_children: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        for b in &func.blocks {
            let id = b.id;
            if id == entry || !rpo_num.contains_key(&id) {
                continue;
            }
            if forward_preds.get(&id).copied().unwrap_or(0) >= 2
                && let Some(&d) = idom.get(&id)
            {
                merge_children.entry(d).or_default().push(id);
            }
        }
        for kids in merge_children.values_mut() {
            kids.sort_by_key(|k| rpo_num[k]);
        }

        Ok(Relooper {
            func,
            entry,
            index,
            rpo_num,
            forward_preds,
            loop_headers,
            merge_children,
        })
    }

    fn block(&self, id: BlockId) -> &'f Block {
        &self.func.blocks[self.index[&id]]
    }

    fn is_merge(&self, id: BlockId) -> bool {
        self.forward_preds.get(&id).copied().unwrap_or(0) >= 2
    }

    // ── Structuring recursion ────────────────────────────────────────────────

    /// Translate the subtree rooted at `x`, wrapping it in a [`Structured::Loop`]
    /// if `x` is a loop header.
    fn do_tree(&self, x: BlockId) -> Result<Structured, RelooperError> {
        let empty = Vec::new();
        let merges = self.merge_children.get(&x).unwrap_or(&empty);
        let inner = self.node_within(x, merges)?;
        if self.loop_headers.contains(&x) {
            Ok(Structured::Loop {
                header: x,
                body: Box::new(inner),
            })
        } else {
            Ok(inner)
        }
    }

    /// Place `x`'s merge children as nested labeled blocks (largest RPO
    /// outermost), with `x`'s own code innermost.
    fn node_within(&self, x: BlockId, merges: &[BlockId]) -> Result<Structured, RelooperError> {
        if let Some((&outer, rest)) = merges.split_last() {
            let body = self.node_within(x, rest)?;
            let next = self.do_tree(outer)?;
            Ok(Structured::Labeled {
                label: outer,
                body: Box::new(body),
                next: Box::new(next),
            })
        } else {
            self.code_for(x)
        }
    }

    /// Emit `x`'s straight-line body followed by the translation of its
    /// terminator.
    fn code_for(&self, x: BlockId) -> Result<Structured, RelooperError> {
        let term = self.translate_terminator(x)?;
        Ok(Structured::Simple {
            block: x,
            next: Box::new(term),
        })
    }

    fn translate_terminator(&self, x: BlockId) -> Result<Structured, RelooperError> {
        match &self.block(x).terminator {
            Terminator::Return(v) => Ok(Structured::Return(*v)),
            Terminator::Unreachable => Ok(Structured::Unreachable),
            Terminator::Jump(t) => self.do_branch(x, *t),
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let then_arm = self.do_branch(x, *then_block)?;
                let else_arm = self.do_branch(x, *else_block)?;
                Ok(Structured::If {
                    cond: *cond,
                    then_arm: Box::new(then_arm),
                    else_arm: Box::new(else_arm),
                })
            }
            // The only back edge: continue to the loop header.
            Terminator::RecurJump { target, .. } => Ok(Structured::Br(*target)),
        }
    }

    /// Translate a forward edge `source -> target`: `br` to a merge node (placed
    /// by an enclosing labeled block), otherwise inline the target's subtree.
    fn do_branch(&self, source: BlockId, target: BlockId) -> Result<Structured, RelooperError> {
        // A forward edge must increase RPO; if it doesn't, the CFG is
        // irreducible (and not something Clojure source can produce).
        if self.rpo_num[&target] <= self.rpo_num[&source] {
            return Err(RelooperError::Irreducible {
                from: source,
                to: target,
            });
        }
        if self.is_merge(target) {
            Ok(Structured::Br(target))
        } else {
            self.do_tree(target)
        }
    }
}

// ── CFG helpers ──────────────────────────────────────────────────────────────

/// Successor block ids of a terminator (in branch order).
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

/// Iterative DFS postorder from `entry` over all edges.
fn postorder(func: &IrFunction, entry: BlockId, index: &HashMap<BlockId, usize>) -> Vec<BlockId> {
    let mut visited = HashSet::new();
    let mut post = Vec::new();
    let mut stack: Vec<(BlockId, usize)> = vec![(entry, 0)];
    visited.insert(entry);
    while let Some(&(b, child)) = stack.last() {
        let succs = successors(&func.blocks[index[&b]].terminator);
        if child < succs.len() {
            stack.last_mut().unwrap().1 += 1;
            let s = succs[child];
            if visited.insert(s) {
                stack.push((s, 0));
            }
        } else {
            post.push(b);
            stack.pop();
        }
    }
    post
}

/// Cooper–Harvey–Kennedy iterative dominators.  Returns `idom` for every
/// reachable block (`entry` maps to itself).
fn dominators(
    entry: BlockId,
    post: &[BlockId],
    rpo_num: &HashMap<BlockId, u32>,
    preds: &HashMap<BlockId, Vec<BlockId>>,
) -> HashMap<BlockId, BlockId> {
    let mut idom: HashMap<BlockId, BlockId> = HashMap::new();
    idom.insert(entry, entry);

    // Reverse postorder, skipping the entry.
    let rpo: Vec<BlockId> = post.iter().rev().copied().collect();

    let intersect = |idom: &HashMap<BlockId, BlockId>, mut a: BlockId, mut b: BlockId| -> BlockId {
        while a != b {
            while rpo_num[&a] > rpo_num[&b] {
                a = idom[&a];
            }
            while rpo_num[&b] > rpo_num[&a] {
                b = idom[&b];
            }
        }
        a
    };

    let mut changed = true;
    while changed {
        changed = false;
        for &b in &rpo {
            if b == entry {
                continue;
            }
            let empty = Vec::new();
            let mut new_idom: Option<BlockId> = None;
            for &p in preds.get(&b).unwrap_or(&empty) {
                if !idom.contains_key(&p) {
                    continue; // predecessor not yet processed
                }
                new_idom = Some(match new_idom {
                    None => p,
                    Some(cur) => intersect(&idom, p, cur),
                });
            }
            if let Some(ni) = new_idom
                && idom.get(&b) != Some(&ni)
            {
                idom.insert(b, ni);
                changed = true;
            }
        }
    }
    idom
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Const, Inst, KnownFn};
    use std::sync::Arc;

    /// Builder that pushes blocks in id order for terse test CFGs.
    struct Fb {
        f: IrFunction,
    }
    impl Fb {
        fn new() -> Self {
            Fb {
                f: IrFunction::new(Some(Arc::from("t")), None),
            }
        }
        fn var(&mut self) -> VarId {
            self.f.fresh_var()
        }
        fn blk(&mut self) -> BlockId {
            self.f.fresh_block()
        }
        fn push(&mut self, id: BlockId, insts: Vec<Inst>, term: Terminator) {
            self.f.blocks.push(Block {
                id,
                phis: vec![],
                insts,
                terminator: term,
            });
        }
    }

    /// Collect every `Simple{block}` id, in emission order, to check coverage.
    fn simple_blocks(s: &Structured, out: &mut Vec<BlockId>) {
        match s {
            Structured::Simple { block, next } => {
                out.push(*block);
                simple_blocks(next, out);
            }
            Structured::Labeled { body, next, .. } => {
                simple_blocks(body, out);
                simple_blocks(next, out);
            }
            Structured::Loop { body, .. } => simple_blocks(body, out),
            Structured::If {
                then_arm, else_arm, ..
            } => {
                simple_blocks(then_arm, out);
                simple_blocks(else_arm, out);
            }
            Structured::Br(_)
            | Structured::Return(_)
            | Structured::Unreachable
            | Structured::Nil => {}
        }
    }

    fn br_targets(s: &Structured, out: &mut Vec<BlockId>) {
        match s {
            Structured::Br(t) => out.push(*t),
            Structured::Simple { next, .. } => br_targets(next, out),
            Structured::Labeled { body, next, .. } => {
                br_targets(body, out);
                br_targets(next, out);
            }
            Structured::Loop { body, .. } => br_targets(body, out),
            Structured::If {
                then_arm, else_arm, ..
            } => {
                br_targets(then_arm, out);
                br_targets(else_arm, out);
            }
            _ => {}
        }
    }

    fn has_loop(s: &Structured) -> bool {
        match s {
            Structured::Loop { .. } => true,
            Structured::Simple { next, .. } => has_loop(next),
            Structured::Labeled { body, next, .. } => has_loop(body) || has_loop(next),
            Structured::If {
                then_arm, else_arm, ..
            } => has_loop(then_arm) || has_loop(else_arm),
            _ => false,
        }
    }

    /// Assert each block appears exactly once as a `Simple`.
    fn assert_each_block_once(f: &IrFunction, s: &Structured) {
        let mut got = Vec::new();
        simple_blocks(s, &mut got);
        let mut ids: Vec<u32> = f.blocks.iter().map(|b| b.id.0).collect();
        ids.sort_unstable();
        let mut got_ids: Vec<u32> = got.iter().map(|b| b.0).collect();
        got_ids.sort_unstable();
        assert_eq!(got_ids, ids, "every block emitted exactly once");
    }

    #[test]
    fn empty_function_errors() {
        let f = IrFunction::new(Some(Arc::from("empty")), None);
        assert!(matches!(reloop(&f), Err(RelooperError::Empty)));
    }

    #[test]
    fn single_return_block() {
        let mut b = Fb::new();
        let v = b.var();
        let b0 = b.blk();
        b.push(
            b0,
            vec![Inst::Const(v, Const::Long(7))],
            Terminator::Return(v),
        );
        let s = reloop(&b.f).unwrap();
        assert_each_block_once(&b.f, &s);
        match s {
            Structured::Simple { next, .. } => assert!(matches!(*next, Structured::Return(_))),
            other => panic!("expected Simple -> Return, got {other:?}"),
        }
    }

    #[test]
    fn linear_chain() {
        let mut b = Fb::new();
        let v = b.var();
        let b0 = b.blk();
        let b1 = b.blk();
        b.push(
            b0,
            vec![Inst::Const(v, Const::Long(1))],
            Terminator::Jump(b1),
        );
        b.push(b1, vec![], Terminator::Return(v));
        let s = reloop(&b.f).unwrap();
        assert_each_block_once(&b.f, &s);
        // No merge (b1 has one forward pred) → inlined, no Labeled.
        assert!(!matches!(s, Structured::Labeled { .. }));
    }

    #[test]
    fn diamond_places_merge_in_labeled_block() {
        // b0: branch -> b1 / b2 ; b1 -> b3 ; b2 -> b3 ; b3: return
        let mut b = Fb::new();
        let c = b.var();
        let v = b.var();
        let (b0, b1, b2, b3) = (b.blk(), b.blk(), b.blk(), b.blk());
        b.push(
            b0,
            vec![Inst::CallKnown(c, KnownFn::IsNil, vec![])],
            Terminator::Branch {
                cond: c,
                then_block: b1,
                else_block: b2,
            },
        );
        b.push(b1, vec![], Terminator::Jump(b3));
        b.push(b2, vec![], Terminator::Jump(b3));
        b.push(b3, vec![Inst::Const(v, Const::Nil)], Terminator::Return(v));

        let s = reloop(&b.f).unwrap();
        assert_each_block_once(&b.f, &s);

        // Top is a labeled block for the merge b3.
        match &s {
            Structured::Labeled { label, body, next } => {
                assert_eq!(*label, b3);
                // body holds b0 + the If; both arms br to b3.
                let mut targets = Vec::new();
                br_targets(body, &mut targets);
                assert_eq!(targets, vec![b3, b3]);
                // next is the merge block's own code, ending in Return.
                let mut merge_blocks = Vec::new();
                simple_blocks(next, &mut merge_blocks);
                assert_eq!(merge_blocks, vec![b3]);
            }
            other => panic!("expected Labeled(merge), got {other:?}"),
        }
    }

    #[test]
    fn loop_with_conditional_exit() {
        // b0: header; branch -> b1 (body) / b2 (exit)
        // b1: recur back to b0
        // b2: return
        let mut b = Fb::new();
        let c = b.var();
        let v = b.var();
        let (b0, b1, b2) = (b.blk(), b.blk(), b.blk());
        b.push(
            b0,
            vec![Inst::CallKnown(c, KnownFn::IsNil, vec![])],
            Terminator::Branch {
                cond: c,
                then_block: b1,
                else_block: b2,
            },
        );
        b.push(
            b1,
            vec![],
            Terminator::RecurJump {
                target: b0,
                args: vec![],
            },
        );
        b.push(b2, vec![Inst::Const(v, Const::Nil)], Terminator::Return(v));

        let s = reloop(&b.f).unwrap();
        assert_each_block_once(&b.f, &s);
        assert!(has_loop(&s), "header should be wrapped in a Loop");

        // The recur in b1 becomes a continue (br) to the header b0.
        let mut targets = Vec::new();
        br_targets(&s, &mut targets);
        assert!(targets.contains(&b0), "recur should br to the loop header");
    }

    #[test]
    fn nested_sequential_merges() {
        // Two diamonds in series: b0?->(b1,b2)->b3?->(b4,b5)->b6
        let mut b = Fb::new();
        let c0 = b.var();
        let c3 = b.var();
        let v = b.var();
        let (b0, b1, b2, b3, b4, b5, b6) = (
            b.blk(),
            b.blk(),
            b.blk(),
            b.blk(),
            b.blk(),
            b.blk(),
            b.blk(),
        );
        b.push(
            b0,
            vec![Inst::CallKnown(c0, KnownFn::IsNil, vec![])],
            Terminator::Branch {
                cond: c0,
                then_block: b1,
                else_block: b2,
            },
        );
        b.push(b1, vec![], Terminator::Jump(b3));
        b.push(b2, vec![], Terminator::Jump(b3));
        b.push(
            b3,
            vec![Inst::CallKnown(c3, KnownFn::IsNil, vec![])],
            Terminator::Branch {
                cond: c3,
                then_block: b4,
                else_block: b5,
            },
        );
        b.push(b4, vec![], Terminator::Jump(b6));
        b.push(b5, vec![], Terminator::Jump(b6));
        b.push(b6, vec![Inst::Const(v, Const::Nil)], Terminator::Return(v));

        let s = reloop(&b.f).unwrap();
        assert_each_block_once(&b.f, &s);

        // Both merges (b3, b6) are reached via br; both branches of each diamond
        // target their merge.
        let mut targets = Vec::new();
        br_targets(&s, &mut targets);
        assert_eq!(
            targets.iter().filter(|&&t| t == b3).count(),
            2,
            "both arms of the first diamond br to b3"
        );
        assert_eq!(
            targets.iter().filter(|&&t| t == b6).count(),
            2,
            "both arms of the second diamond br to b6"
        );
    }
}
