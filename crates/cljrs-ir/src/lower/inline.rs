//! Inlining pass.
//!
//! Replaces eligible `Call` sites with the callee's body before escape
//! analysis runs, so that allocations in the callee are visible as local to
//! the caller and can be region-promoted.
//!
//! **Eligibility criteria** (all must hold):
//! - The callee resolves to a named `IrFunction` in the same compilation unit.
//! - The callee has no `LoadLocal` instructions (no environment captures).
//! - The callee has no subfunctions (no closures in its body).
//! - The callee body fits within `INLINE_THRESHOLD` instructions.
//! - The callee is not the caller itself (no direct recursion).
//!
//! Inlining is run bottom-up (subfunctions first) for up to `MAX_ROUNDS`
//! passes per function, stopping early when no eligible call is found.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use super::escape::{ClosureInfo, build_defn_map, build_fn_registry};
use crate::{Block, BlockId, Inst, IrFunction, Terminator, VarId};

const INLINE_THRESHOLD: usize = 20;
const MAX_ROUNDS: usize = 8;

// ── Eligibility ──────────────────────────────────────────────────────────────

fn instruction_count(ir: &IrFunction) -> usize {
    ir.blocks.iter().map(|b| b.insts.len() + b.phis.len()).sum()
}

fn has_load_local(ir: &IrFunction) -> bool {
    ir.blocks.iter().any(|b| {
        b.insts
            .iter()
            .chain(b.phis.iter())
            .any(|i| matches!(i, Inst::LoadLocal(..)))
    })
}

fn is_eligible(callee: &IrFunction, forbidden: &HashSet<Arc<str>>) -> bool {
    let name_ok = callee
        .name
        .as_ref()
        .map(|n| !forbidden.contains(n))
        .unwrap_or(false); // unnamed callee → skip
    name_ok
        && callee.subfunctions.is_empty()
        && !has_load_local(callee)
        && !callee.blocks.is_empty()
        && instruction_count(callee) <= INLINE_THRESHOLD
}

// ── VarId / BlockId remapping ────────────────────────────────────────────────

fn rv(map: &HashMap<VarId, VarId>, v: VarId) -> VarId {
    map.get(&v).copied().unwrap_or(v)
}

fn rb(map: &HashMap<BlockId, BlockId>, b: BlockId) -> BlockId {
    map.get(&b).copied().unwrap_or(b)
}

fn clone_inst(
    inst: &Inst,
    var_map: &HashMap<VarId, VarId>,
    block_map: &HashMap<BlockId, BlockId>,
) -> Inst {
    match inst {
        Inst::Const(dst, c) => Inst::Const(rv(var_map, *dst), c.clone()),
        Inst::LoadLocal(dst, name) => Inst::LoadLocal(rv(var_map, *dst), name.clone()),
        Inst::LoadGlobal(dst, ns, name) => {
            Inst::LoadGlobal(rv(var_map, *dst), ns.clone(), name.clone())
        }
        Inst::LoadVar(dst, ns, name) => Inst::LoadVar(rv(var_map, *dst), ns.clone(), name.clone()),
        Inst::AllocVector(dst, elems) => Inst::AllocVector(
            rv(var_map, *dst),
            elems.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::AllocMap(dst, pairs) => Inst::AllocMap(
            rv(var_map, *dst),
            pairs
                .iter()
                .map(|&(k, v)| (rv(var_map, k), rv(var_map, v)))
                .collect(),
        ),
        Inst::AllocSet(dst, elems) => Inst::AllocSet(
            rv(var_map, *dst),
            elems.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::AllocList(dst, elems) => Inst::AllocList(
            rv(var_map, *dst),
            elems.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::AllocCons(dst, h, t) => {
            Inst::AllocCons(rv(var_map, *dst), rv(var_map, *h), rv(var_map, *t))
        }
        Inst::AllocClosure(dst, tmpl, captures) => Inst::AllocClosure(
            rv(var_map, *dst),
            tmpl.clone(),
            captures.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::CallKnown(dst, func, args) => Inst::CallKnown(
            rv(var_map, *dst),
            func.clone(),
            args.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::Call(dst, callee, args) => Inst::Call(
            rv(var_map, *dst),
            rv(var_map, *callee),
            args.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::CallDirect(dst, name, args) => Inst::CallDirect(
            rv(var_map, *dst),
            name.clone(),
            args.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::Deref(dst, src) => Inst::Deref(rv(var_map, *dst), rv(var_map, *src)),
        Inst::DefVar(dst, ns, name, val) => Inst::DefVar(
            rv(var_map, *dst),
            ns.clone(),
            name.clone(),
            rv(var_map, *val),
        ),
        Inst::SetBang(var, val) => Inst::SetBang(rv(var_map, *var), rv(var_map, *val)),
        Inst::Throw(val) => Inst::Throw(rv(var_map, *val)),
        Inst::Phi(dst, entries) => Inst::Phi(
            rv(var_map, *dst),
            entries
                .iter()
                .map(|&(bid, v)| (rb(block_map, bid), rv(var_map, v)))
                .collect(),
        ),
        Inst::Recur(args) => Inst::Recur(args.iter().map(|&v| rv(var_map, v)).collect()),
        Inst::SourceLoc(span) => Inst::SourceLoc(span.clone()),
        Inst::RegionStart(dst) => Inst::RegionStart(rv(var_map, *dst)),
        Inst::RegionAlloc(dst, region, kind, ops) => Inst::RegionAlloc(
            rv(var_map, *dst),
            rv(var_map, *region),
            *kind,
            ops.iter().map(|&v| rv(var_map, v)).collect(),
        ),
        Inst::RegionEnd(region) => Inst::RegionEnd(rv(var_map, *region)),
        Inst::RegionParam(dst) => Inst::RegionParam(rv(var_map, *dst)),
        Inst::CallWithRegion(dst, name, args, region) => Inst::CallWithRegion(
            rv(var_map, *dst),
            name.clone(),
            args.iter().map(|&v| rv(var_map, v)).collect(),
            rv(var_map, *region),
        ),
        Inst::Await { src, dst } => Inst::Await {
            src: rv(var_map, *src),
            dst: rv(var_map, *dst),
        },
        Inst::Spawn { fn_reg, args, dst } => Inst::Spawn {
            fn_reg: rv(var_map, *fn_reg),
            args: args.iter().map(|&v| rv(var_map, v)).collect(),
            dst: rv(var_map, *dst),
        },
        Inst::ChanTake { chan, dst } => Inst::ChanTake {
            chan: rv(var_map, *chan),
            dst: rv(var_map, *dst),
        },
        Inst::ChanPut { chan, val } => Inst::ChanPut {
            chan: rv(var_map, *chan),
            val: rv(var_map, *val),
        },
        // Async state-machine instructions are produced after inlining (by
        // `async_lower`), so these arms are not exercised in practice; remap for
        // completeness.
        Inst::StateStore { slot, val } => Inst::StateStore {
            slot: *slot,
            val: rv(var_map, *val),
        },
        Inst::StateLoad { dst, slot } => Inst::StateLoad {
            dst: rv(var_map, *dst),
            slot: *slot,
        },
        Inst::AsyncSuspend {
            kind,
            operands,
            next_state,
        } => Inst::AsyncSuspend {
            kind: *kind,
            operands: operands.iter().map(|&v| rv(var_map, v)).collect(),
            next_state: *next_state,
        },
        Inst::AsyncResume { dst, kind } => Inst::AsyncResume {
            dst: rv(var_map, *dst),
            kind: *kind,
        },
    }
}

fn clone_terminator(
    term: &Terminator,
    var_map: &HashMap<VarId, VarId>,
    block_map: &HashMap<BlockId, BlockId>,
    cont_block: BlockId,
) -> Terminator {
    match term {
        // The key transformation: Return → Jump to continuation.
        Terminator::Return(_) => Terminator::Jump(cont_block),
        Terminator::Jump(b) => Terminator::Jump(rb(block_map, *b)),
        Terminator::Branch {
            cond,
            then_block,
            else_block,
        } => Terminator::Branch {
            cond: rv(var_map, *cond),
            then_block: rb(block_map, *then_block),
            else_block: rb(block_map, *else_block),
        },
        Terminator::RecurJump { target, args } => Terminator::RecurJump {
            target: rb(block_map, *target),
            args: args.iter().map(|&v| rv(var_map, v)).collect(),
        },
        Terminator::Unreachable => Terminator::Unreachable,
    }
}

// ── Call-site resolution ─────────────────────────────────────────────────────

/// Resolve a `Call(_, callee_var, args)` to the named arity function in the
/// registry.  Returns `(arity_fn_name, callee_ir)`.
fn resolve<'r>(
    callee_var: VarId,
    arg_count: usize,
    var_defs: &HashMap<VarId, &Inst>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
    registry: &'r HashMap<Arc<str>, Arc<IrFunction>>,
) -> Option<(Arc<str>, &'r IrFunction)> {
    // callee_var must be defined by LoadGlobal in the same function.
    let fn_name = match var_defs.get(&callee_var)? {
        Inst::LoadGlobal(_, ns, name) => {
            let info = defn_map.get(&(ns.clone(), name.clone()))?;
            // Pick a matching non-variadic arity.
            info.arity_fn_names
                .iter()
                .zip(&info.param_counts)
                .zip(&info.is_variadic)
                .find(|&((_, &pc), &var)| pc == arg_count && !var)
                .map(|((name, _), _)| name.clone())?
        }
        _ => return None,
    };
    let callee = registry.get(&fn_name)?;
    Some((fn_name, callee.as_ref()))
}

// ── Core inline operation ────────────────────────────────────────────────────

/// Splice `callee` into `caller` at the `Call` in `caller.blocks[block_idx].insts[inst_idx]`.
///
/// Splits the caller block into B_pre + callee body + B_post.  A phi in B_post
/// gathers the callee's return values and binds them to `call_dst`.
///
/// `callee_self` is the VarId of the closure object at the call site, mapped to
/// the callee's leading self-param (the calling convention always prepends one).
fn do_inline(
    mut caller: IrFunction,
    block_idx: usize,
    inst_idx: usize,
    callee: &IrFunction,
    callee_self: VarId,
    args: Vec<VarId>,
    call_dst: VarId,
) -> IrFunction {
    // ── Allocate fresh VarIds and BlockIds ───────────────────────────────────

    let mut var_map: HashMap<VarId, VarId> = HashMap::new();
    let mut block_map: HashMap<BlockId, BlockId> = HashMap::new();

    // Arity functions always have a leading self/closure param followed by the
    // user-visible params (params.len() == args.len() + 1).  In the rare case
    // where there is no self param, fall back to 1-to-1 mapping.
    if callee.params.len() == args.len() + 1 {
        let (self_param, user_params) = callee.params.split_first().expect("non-empty");
        var_map.insert(self_param.1, callee_self);
        for (i, (_, param_var)) in user_params.iter().enumerate() {
            var_map.insert(*param_var, args[i]);
        }
    } else {
        for (i, (_, param_var)) in callee.params.iter().enumerate() {
            var_map.insert(*param_var, args[i]);
        }
    }

    // All other callee VarIds get fresh caller VarIds.
    for block in &callee.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Some(dst) = inst.dst() {
                var_map.entry(dst).or_insert_with(|| {
                    let fresh = VarId(caller.next_var);
                    caller.next_var += 1;
                    fresh
                });
            }
        }
    }

    // Fresh BlockIds for each callee block.
    for block in &callee.blocks {
        let fresh = BlockId(caller.next_block);
        caller.next_block += 1;
        block_map.insert(block.id, fresh);
    }

    // Continuation block id.
    let cont_id = BlockId(caller.next_block);
    caller.next_block += 1;

    // ── Collect return sites ─────────────────────────────────────────────────

    let return_sites: Vec<(BlockId, VarId)> = callee
        .blocks
        .iter()
        .filter_map(|b| {
            if let Terminator::Return(ret_var) = &b.terminator {
                Some((block_map[&b.id], rv(&var_map, *ret_var)))
            } else {
                None
            }
        })
        .collect();

    // If the callee never returns (all paths throw), the call_dst phi is empty
    // and the continuation is unreachable.  Inlining is still valid.

    // ── Clone callee blocks ──────────────────────────────────────────────────

    let cloned_blocks: Vec<Block> = callee
        .blocks
        .iter()
        .map(|block| Block {
            id: block_map[&block.id],
            phis: block
                .phis
                .iter()
                .map(|i| clone_inst(i, &var_map, &block_map))
                .collect(),
            insts: block
                .insts
                .iter()
                .map(|i| clone_inst(i, &var_map, &block_map))
                .collect(),
            terminator: clone_terminator(&block.terminator, &var_map, &block_map, cont_id),
        })
        .collect();

    let callee_entry = block_map[&callee.blocks[0].id];

    // ── Split the caller block ───────────────────────────────────────────────

    let orig_block = &mut caller.blocks[block_idx];
    let orig_phis = orig_block.phis.clone();
    let pre_insts: Vec<Inst> = orig_block.insts[..inst_idx].to_vec();
    let post_insts: Vec<Inst> = orig_block.insts[inst_idx + 1..].to_vec();
    let post_term = orig_block.terminator.clone();

    // B_pre: original phis, instructions before the call, jump into callee.
    orig_block.phis = orig_phis;
    orig_block.insts = pre_insts;
    orig_block.terminator = Terminator::Jump(callee_entry);

    // B_post: phi gathering callee returns → call_dst, then continuation code.
    let cont_block = Block {
        id: cont_id,
        phis: if return_sites.is_empty() {
            vec![]
        } else {
            vec![Inst::Phi(call_dst, return_sites)]
        },
        insts: post_insts,
        terminator: post_term.clone(),
    };

    // ── Append cloned + continuation blocks ─────────────────────────────────

    caller.blocks.extend(cloned_blocks);
    caller.blocks.push(cont_block);

    // ── Fix up stale phi predecessors ────────────────────────────────────────
    // B_pre's old terminator pointed to some successor block(s) (e.g. an
    // epilogue phi-return block created by ANF lowering).  Those blocks still
    // have phi entries recording B_pre as a predecessor, but after the split
    // B_pre jumps into the callee — it is B_post (cont_id) that now jumps to
    // those successors.  Rewrite every such phi entry to name cont_id instead.
    let bpre_id = caller.blocks[block_idx].id;
    let post_targets = terminator_targets(&post_term);
    for target_id in post_targets {
        if let Some(blk) = caller.blocks.iter_mut().find(|b| b.id == target_id) {
            for inst in &mut blk.phis {
                if let Inst::Phi(_, entries) = inst {
                    for (pred, _) in entries.iter_mut() {
                        if *pred == bpre_id {
                            *pred = cont_id;
                        }
                    }
                }
            }
        }
    }

    caller
}

/// Collect the block IDs that a terminator may jump to (excluding RecurJump
/// targets, which are back-edges that cannot carry stale phi predecessors in
/// this context).
fn terminator_targets(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Jump(b) => vec![*b],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        Terminator::Return(_) | Terminator::RecurJump { .. } | Terminator::Unreachable => vec![],
    }
}

// ── Pass driver ──────────────────────────────────────────────────────────────

/// Try to inline one eligible call per block in a single sweep.
/// Returns `(updated_func, changed)`.
fn inline_one_round(
    mut func: IrFunction,
    registry: &HashMap<Arc<str>, Arc<IrFunction>>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
    forbidden: &HashSet<Arc<str>>,
) -> (IrFunction, bool) {
    // Build var-def map once per round.  LoadGlobal defs don't change between
    // rounds (we only add new blocks at the tail), so this is safe.
    let var_defs_owned: HashMap<VarId, Inst> = func
        .blocks
        .iter()
        .flat_map(|b| b.phis.iter().chain(b.insts.iter()))
        .filter_map(|i| i.dst().map(|dst| (dst, i.clone())))
        .collect();
    let var_defs: HashMap<VarId, &Inst> = var_defs_owned.iter().map(|(k, v)| (*k, v)).collect();

    let mut changed = false;
    let mut block_idx = 0;

    while block_idx < func.blocks.len() {
        // Find the first eligible call in this block.
        let found = func.blocks[block_idx]
            .insts
            .iter()
            .enumerate()
            .find_map(|(inst_idx, inst)| {
                let Inst::Call(dst, callee_var, args) = inst else {
                    return None;
                };
                let (fn_name, callee) =
                    resolve(*callee_var, args.len(), &var_defs, defn_map, registry)?;
                if !is_eligible(callee, forbidden) {
                    return None;
                }
                Some((inst_idx, fn_name, *callee_var, args.clone(), *dst))
            });

        if let Some((inst_idx, fn_name, callee_self, args, call_dst)) = found {
            let callee = registry[&fn_name].clone();
            func = do_inline(
                func,
                block_idx,
                inst_idx,
                &callee,
                callee_self,
                args,
                call_dst,
            );
            changed = true;
            // block_idx now points to B_pre (ends in Jump) — advance past it.
        }
        block_idx += 1;
    }

    (func, changed)
}

/// Run the inlining pass on one `IrFunction` (subfunctions handled separately).
fn inline_fn(
    func: IrFunction,
    registry: &HashMap<Arc<str>, Arc<IrFunction>>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
    forbidden: &HashSet<Arc<str>>,
) -> IrFunction {
    let mut func = func;
    for _ in 0..MAX_ROUNDS {
        let (new_func, changed) = inline_one_round(func, registry, defn_map, forbidden);
        func = new_func;
        if !changed {
            break;
        }
    }
    func
}

/// Walk the function tree bottom-up, inlining at each level.
fn inline_tree(
    mut func: IrFunction,
    registry: &HashMap<Arc<str>, Arc<IrFunction>>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
) -> IrFunction {
    // Bottom-up: process subfunctions first.
    let subs = std::mem::take(&mut func.subfunctions);
    func.subfunctions = subs
        .into_iter()
        .map(|sub| inline_tree(sub, registry, defn_map))
        .collect();

    // Build a forbidden set: don't inline the function into itself.
    let mut forbidden: HashSet<Arc<str>> = HashSet::new();
    if let Some(name) = &func.name {
        forbidden.insert(name.clone());
    }

    inline_fn(func, registry, defn_map, &forbidden)
}

/// Run the inlining pass on the entire IR tree rooted at `ir_func`.
///
/// Must be called **before** escape analysis so that allocations in inlined
/// callees are visible as local to the caller.
pub fn inline(ir_func: IrFunction) -> IrFunction {
    // Build the registry and defn-map from the full tree.
    let registry = build_fn_registry(&ir_func);
    let defn_map = build_defn_map(&ir_func);

    // Also populate var_defs for the top-level: needed by resolve() inside
    // inline_one_round.  The tree walk handles each function independently.
    inline_tree(ir_func, &registry, &defn_map)
}

// ── Helpers re-exported from escape for internal use ────────────────────────

/// Count instructions across all blocks (used by tests).
#[cfg(test)]
pub fn count_insts(func: &IrFunction) -> usize {
    instruction_count(func)
}

/// Whether the function has any LoadLocal (used by tests).
#[cfg(test)]
pub fn check_has_load_local(func: &IrFunction) -> bool {
    has_load_local(func)
}
