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
//! # Why the cheap relooper suffices
//!
//! Clojure has no `goto`.  `loop`/`recur` produces reducible loop headers (the
//! IR models these as [`Terminator::RecurJump`] back-edges to a loop header),
//! and `if`/`cond`/`do` produce structured branches.  Inlining preserves
//! reducibility.  So every CFG this backend sees is reducible, which means the
//! relooper never needs node-splitting or a dispatch variable — the structure
//! recovers directly from the dominator tree / back-edge set.
//!
//! # Algorithm (target shape)
//!
//! Standard reducible-CFG structuring:
//!
//! 1. Compute reverse-postorder (RPO) and the dominator tree.
//! 2. A back-edge `b → h` (where `h` dominates `b`) marks `h` as a loop header;
//!    wrap the region in a [`Structured::Loop`] whose label a `RecurJump`
//!    lowers to [`Structured::Continue`].
//! 3. A two-way [`Terminator::Branch`] becomes [`Structured::If`]; control
//!    re-merges at the branch's immediate post-dominator, which becomes the
//!    `If`'s `next` continuation.
//! 4. A forward edge to a multi-predecessor join lowers to a `br` out of an
//!    enclosing labeled [`Structured::Block`] whose end is that join.
//!
//! # Status
//!
//! **Scaffold.**  The data model is final.  [`reloop`] implements the acyclic
//! single-successor, return, and simple re-joining diamond cases; loops,
//! multi-predecessor joins, and `try`/`catch` regions return
//! [`RelooperError::Unsupported`], with the dominator-based structuring left as
//! the documented next step.

use std::collections::HashMap;

use crate::ir::{Block, BlockId, IrFunction, Terminator, VarId};

/// A label naming an enclosing structured construct, targeted by `br`/continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelId(pub u32);

/// A structured control-flow tree — the relooper's output and the emitter's
/// input.  Each node maps to a small, fixed wasm shape.
///
/// `If` and `Loop` carry an explicit `next` continuation (the code that runs
/// after the construct), so the tree is a single spine with no implicit merge
/// blocks for the emitter to rediscover.
#[derive(Debug, Clone)]
pub enum Structured {
    /// Emit one IR block's straight-line body (`phis` + `insts`), then `next`.
    /// The block's own terminator is represented by the surrounding nodes.
    Simple {
        block: BlockId,
        next: Box<Structured>,
    },
    /// A reducible loop (`wasm loop`).  A [`Terminator::RecurJump`] to `header`
    /// lowers to [`Structured::Continue`] of `label`; falling off `body` exits
    /// the loop and runs `next`.
    Loop {
        label: LabelId,
        header: BlockId,
        body: Box<Structured>,
        next: Box<Structured>,
    },
    /// Structured two-way branch from a [`Terminator::Branch`].  `then_arm` and
    /// `else_arm` are the arm bodies (ending in `Nil` at the merge); `next` is
    /// the post-dominator continuation that runs after either arm.
    If {
        cond: VarId,
        then_arm: Box<Structured>,
        else_arm: Box<Structured>,
        next: Box<Structured>,
    },
    /// `br` to the end of the labeled enclosing block (exit a join region).
    Break(LabelId),
    /// `br` to the top of the labeled enclosing loop (a `recur`).
    Continue(LabelId),
    /// Function return of a value.
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
    /// A control-flow shape the scaffold does not structure yet.
    Unsupported(String),
}

impl std::fmt::Display for RelooperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RelooperError::Empty => write!(f, "function has no blocks"),
            RelooperError::Unsupported(what) => write!(f, "unsupported control flow: {what}"),
        }
    }
}

/// Structure the control flow of `func` into a [`Structured`] tree.
pub fn reloop(func: &IrFunction) -> Result<Structured, RelooperError> {
    if func.blocks.is_empty() {
        return Err(RelooperError::Empty);
    }
    let cfg = Cfg::new(func);
    if cfg.has_back_edges() {
        // Loop structuring (dominator tree + back-edge detection) is the
        // documented next step; the data model already has `Loop`/`Continue`.
        return Err(RelooperError::Unsupported(
            "loops (recur back-edges) — needs dominator-based loop structuring".into(),
        ));
    }
    let entry = func.blocks[0].id;
    structure_acyclic(&cfg, entry)
}

/// A lightweight CFG view over an [`IrFunction`]: block index + predecessor map
/// keyed by [`BlockId`].
struct Cfg<'f> {
    func: &'f IrFunction,
    index: HashMap<BlockId, usize>,
    preds: HashMap<BlockId, Vec<BlockId>>,
}

impl<'f> Cfg<'f> {
    fn new(func: &'f IrFunction) -> Self {
        let mut index = HashMap::new();
        let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
        for (i, b) in func.blocks.iter().enumerate() {
            index.insert(b.id, i);
            preds.entry(b.id).or_default();
        }
        for b in &func.blocks {
            for s in successors(&b.terminator) {
                preds.entry(s).or_default().push(b.id);
            }
        }
        Cfg { func, index, preds }
    }

    fn block(&self, id: BlockId) -> &'f Block {
        &self.func.blocks[self.index[&id]]
    }

    fn pred_count(&self, id: BlockId) -> usize {
        self.preds.get(&id).map(|p| p.len()).unwrap_or(0)
    }

    /// A back-edge is a `RecurJump`, or a successor that is not later than its
    /// source in block order.  Reducible Clojure CFGs place loop headers before
    /// their back-edges, so block-order comparison detects them.
    fn has_back_edges(&self) -> bool {
        self.func.blocks.iter().any(|b| {
            if matches!(b.terminator, Terminator::RecurJump { .. }) {
                return true;
            }
            let src = self.index[&b.id];
            successors(&b.terminator)
                .into_iter()
                .any(|s| self.index[&s] <= src)
        })
    }
}

/// Successor block ids of a terminator.
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

/// Structure an acyclic region rooted at `block`: emit the block body, then
/// recurse on its terminator.  Handles single-successor chains, returns, and
/// simple re-joining diamonds; bails on multi-predecessor joins it cannot yet
/// place behind a block label.
fn structure_acyclic(cfg: &Cfg, block: BlockId) -> Result<Structured, RelooperError> {
    let tail = match &cfg.block(block).terminator {
        Terminator::Return(v) => Structured::Return(*v),
        Terminator::Unreachable => Structured::Unreachable,
        Terminator::Jump(t) => {
            if cfg.pred_count(*t) > 1 {
                return Err(RelooperError::Unsupported(
                    "forward edge into a multi-predecessor join — needs block-label placement"
                        .into(),
                ));
            }
            structure_acyclic(cfg, *t)?
        }
        Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => structure_diamond(cfg, *cond, *then_block, *else_block)?,
        Terminator::RecurJump { .. } => {
            return Err(RelooperError::Unsupported(
                "recur outside a structured loop".into(),
            ));
        }
    };
    Ok(Structured::Simple {
        block,
        next: Box::new(tail),
    })
}

/// Structure a `Branch` whose arms re-merge at a common post-dominator (the
/// classic if/else diamond): each arm is a single block that jumps to the same
/// merge block.  The merge becomes the `If`'s `next` continuation.
fn structure_diamond(
    cfg: &Cfg,
    cond: VarId,
    then_block: BlockId,
    else_block: BlockId,
) -> Result<Structured, RelooperError> {
    let then_succ = successors(&cfg.block(then_block).terminator);
    let else_succ = successors(&cfg.block(else_block).terminator);

    if let ([then_merge], [else_merge]) = (then_succ.as_slice(), else_succ.as_slice())
        && then_merge == else_merge
        && cfg.pred_count(then_block) == 1
        && cfg.pred_count(else_block) == 1
    {
        let then_arm = Structured::Simple {
            block: then_block,
            next: Box::new(Structured::Nil),
        };
        let else_arm = Structured::Simple {
            block: else_block,
            next: Box::new(Structured::Nil),
        };
        let merge = structure_acyclic(cfg, *then_merge)?;
        return Ok(Structured::If {
            cond,
            then_arm: Box::new(then_arm),
            else_arm: Box::new(else_arm),
            next: Box::new(merge),
        });
    }

    Err(RelooperError::Unsupported(
        "branch arms do not form a simple re-joining diamond yet".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Const, Inst, KnownFn};
    use std::sync::Arc;

    fn ret_const(name: &str) -> IrFunction {
        let mut f = IrFunction::new(Some(Arc::from(name)), None);
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Const(v, Const::Long(7))],
            terminator: Terminator::Return(v),
        });
        f
    }

    #[test]
    fn empty_function_errors() {
        let f = IrFunction::new(Some(Arc::from("empty")), None);
        assert!(matches!(reloop(&f), Err(RelooperError::Empty)));
    }

    #[test]
    fn single_return_block_structures() {
        let f = ret_const("k");
        match reloop(&f).expect("structure single return") {
            Structured::Simple { next, .. } => assert!(matches!(*next, Structured::Return(_))),
            other => panic!("expected Simple -> Return, got {other:?}"),
        }
    }

    #[test]
    fn linear_jump_chain_structures() {
        let mut f = IrFunction::new(Some(Arc::from("chain")), None);
        let v = f.fresh_var();
        let b0 = f.fresh_block();
        let b1 = f.fresh_block();
        f.blocks.push(Block {
            id: b0,
            phis: vec![],
            insts: vec![Inst::Const(v, Const::Long(1))],
            terminator: Terminator::Jump(b1),
        });
        f.blocks.push(Block {
            id: b1,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Return(v),
        });
        assert!(reloop(&f).is_ok());
    }

    #[test]
    fn simple_diamond_structures_to_if() {
        // b0: branch -> b1 / b2 ; b1 -> b3 ; b2 -> b3 ; b3: return
        let mut f = IrFunction::new(Some(Arc::from("diamond")), None);
        let c = f.fresh_var();
        let v = f.fresh_var();
        let b0 = f.fresh_block();
        let b1 = f.fresh_block();
        let b2 = f.fresh_block();
        let b3 = f.fresh_block();
        f.blocks.push(Block {
            id: b0,
            phis: vec![],
            insts: vec![Inst::CallKnown(c, KnownFn::IsNil, vec![])],
            terminator: Terminator::Branch {
                cond: c,
                then_block: b1,
                else_block: b2,
            },
        });
        for b in [b1, b2] {
            f.blocks.push(Block {
                id: b,
                phis: vec![],
                insts: vec![],
                terminator: Terminator::Jump(b3),
            });
        }
        f.blocks.push(Block {
            id: b3,
            phis: vec![],
            insts: vec![Inst::Const(v, Const::Nil)],
            terminator: Terminator::Return(v),
        });

        match reloop(&f).expect("structure diamond") {
            Structured::Simple { next, .. } => assert!(
                matches!(*next, Structured::If { .. }),
                "entry block should be followed by an If"
            ),
            other => panic!("expected Simple -> If, got {other:?}"),
        }
    }

    #[test]
    fn loops_are_unsupported_for_now() {
        let mut f = IrFunction::new(Some(Arc::from("loopy")), None);
        let c = f.fresh_var();
        let b0 = f.fresh_block();
        let b1 = f.fresh_block();
        f.blocks.push(Block {
            id: b0,
            phis: vec![],
            insts: vec![Inst::CallKnown(c, KnownFn::IsNil, vec![])],
            terminator: Terminator::Branch {
                cond: c,
                then_block: b1,
                else_block: b1,
            },
        });
        f.blocks.push(Block {
            id: b1,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::RecurJump {
                target: b0,
                args: vec![],
            },
        });
        assert!(matches!(reloop(&f), Err(RelooperError::Unsupported(_))));
    }
}
