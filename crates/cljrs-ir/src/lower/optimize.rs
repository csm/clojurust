//! Region allocation optimization pass.
//!
//! Rewrites non-escaping allocations to region-backed allocations scoped
//! over the minimal CFG subgraph that covers the allocation and all its uses.
//! Mirrors `cljrs.compiler.optimize`.

use std::collections::{HashMap, HashSet};

use super::escape::{EscapeContext, EscapeState, analyze, build_use_chains, make_context};
use super::inline::inline as inline_pass;
use super::regionalize::promote_cross_fn_allocs;
use crate::{Block, BlockId, Const, Inst, IrFunction, KnownFn, RegionAllocKind, Terminator, VarId};

// ── CFG helpers ──────────────────────────────────────────────────────────────

fn block_successors(block: &Block) -> Vec<BlockId> {
    match &block.terminator {
        Terminator::Jump(target) => vec![*target],
        Terminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            vec![*then_block, *else_block]
        }
        Terminator::RecurJump { target, .. } => vec![*target],
        // Return, Unreachable — no successors
        _ => vec![],
    }
}

fn block_by_id_map(ir_func: &IrFunction) -> HashMap<BlockId, &Block> {
    ir_func.blocks.iter().map(|b| (b.id, b)).collect()
}

fn predecessor_map(ir_func: &IrFunction) -> HashMap<BlockId, HashSet<BlockId>> {
    let mut preds: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    for block in &ir_func.blocks {
        for succ in block_successors(block) {
            preds.entry(succ).or_default().insert(block.id);
        }
    }
    preds
}

/// DFS from block 0; return block IDs in reverse-postorder.
fn reverse_postorder(ir_func: &IrFunction) -> Vec<BlockId> {
    let by_id = block_by_id_map(ir_func);
    let mut stack: Vec<(BlockId, bool)> = vec![(BlockId(0), false)];
    let mut visited: HashSet<BlockId> = HashSet::new();
    let mut postorder: Vec<BlockId> = Vec::new();

    while let Some((bid, done)) = stack.pop() {
        if done {
            postorder.push(bid);
            continue;
        }
        if !visited.insert(bid) {
            continue;
        }
        stack.push((bid, true));
        if let Some(block) = by_id.get(&bid) {
            for succ in block_successors(block) {
                stack.push((succ, false));
            }
        }
    }

    postorder.reverse();
    postorder
}

// ── Dominator analysis ───────────────────────────────────────────────────────

fn intersect_sets(sets: impl Iterator<Item = HashSet<BlockId>>) -> HashSet<BlockId> {
    let mut result: Option<HashSet<BlockId>> = None;
    for s in sets {
        result = Some(match result {
            None => s,
            Some(acc) => acc.intersection(&s).copied().collect(),
        });
    }
    result.unwrap_or_default()
}

/// Generic iterative dominator computation.
///
/// `roots` — block IDs initialised to `{root}` (only dominate themselves).
/// `block_ids` — all block IDs in reverse-postorder.
/// `pred_fn` — block_id → set of predecessor IDs.
fn dom_iterate(
    roots: &HashSet<BlockId>,
    block_ids: &[BlockId],
    pred_fn: &dyn Fn(BlockId) -> HashSet<BlockId>,
) -> HashMap<BlockId, HashSet<BlockId>> {
    let universe: HashSet<BlockId> = block_ids.iter().copied().collect();
    let mut doms: HashMap<BlockId, HashSet<BlockId>> = block_ids
        .iter()
        .map(|&b| {
            let set = if roots.contains(&b) {
                let mut s = HashSet::new();
                s.insert(b);
                s
            } else {
                universe.clone()
            };
            (b, set)
        })
        .collect();

    loop {
        let mut changed = false;
        for &b in block_ids {
            if roots.contains(&b) {
                continue;
            }
            let preds: Vec<_> = pred_fn(b)
                .into_iter()
                .filter(|p| doms.contains_key(p))
                .collect();
            if preds.is_empty() {
                continue;
            }
            let pred_doms = preds.iter().map(|p| doms[p].clone());
            let mut new_set = intersect_sets(pred_doms);
            new_set.insert(b);
            if new_set != doms[&b] {
                doms.insert(b, new_set);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    doms
}

pub(crate) fn dominators(ir_func: &IrFunction) -> HashMap<BlockId, HashSet<BlockId>> {
    let rpo = reverse_postorder(ir_func);
    let preds = predecessor_map(ir_func);
    let mut roots = HashSet::new();
    roots.insert(BlockId(0));
    dom_iterate(&roots, &rpo, &|b| {
        preds.get(&b).cloned().unwrap_or_default()
    })
}

fn collect_exits(ir_func: &IrFunction) -> HashSet<BlockId> {
    ir_func
        .blocks
        .iter()
        .filter(|b| {
            matches!(
                b.terminator,
                Terminator::Return(_) | Terminator::Unreachable
            )
        })
        .map(|b| b.id)
        .collect()
}

pub(crate) fn post_dominators(ir_func: &IrFunction) -> HashMap<BlockId, HashSet<BlockId>> {
    let rpo = reverse_postorder(ir_func);
    let by_id = block_by_id_map(ir_func);
    let exits = collect_exits(ir_func);
    // Post-dominator: reverse the CFG (successors become predecessors)
    let succ_fn = |b: BlockId| -> HashSet<BlockId> {
        if let Some(block) = by_id.get(&b) {
            block_successors(block).into_iter().collect()
        } else {
            HashSet::new()
        }
    };
    dom_iterate(&exits, &rpo, &succ_fn)
}

// ── LCA in dominator relation ────────────────────────────────────────────────

/// Lowest common ancestor of `a` and `b` in the dominator relation `dom_of`.
/// Returns the deepest block that dominates both a and b (i.e. appears in
/// both dom_of[a] and dom_of[b], and is dominated by all other common
/// dominators).
pub(crate) fn lca_of(
    dom_of: &HashMap<BlockId, HashSet<BlockId>>,
    a: BlockId,
    b: BlockId,
) -> Option<BlockId> {
    let da = dom_of.get(&a)?;
    let db = dom_of.get(&b)?;
    let common: HashSet<_> = da.intersection(db).copied().collect();
    if common.is_empty() {
        return None;
    }
    // Pick the deepest: the one dominated by all others (i.e. has the most dominators)
    common
        .into_iter()
        .max_by_key(|&d| dom_of.get(&d).map(|s| s.len()).unwrap_or(0))
}

pub(crate) fn lca_of_many(
    dom_of: &HashMap<BlockId, HashSet<BlockId>>,
    blocks: impl IntoIterator<Item = BlockId>,
) -> Option<BlockId> {
    let mut iter = blocks.into_iter();
    let first = iter.next()?;
    iter.try_fold(first, |acc, b| lca_of(dom_of, acc, b))
}

// ── Region path analysis ─────────────────────────────────────────────────────

/// Return the set of block IDs reachable from `start` whose paths terminate at `end`.
/// Stops expanding past `end`. Includes both `start` and `end`.
pub(crate) fn blocks_on_path(
    ir_func: &IrFunction,
    start: BlockId,
    end: BlockId,
) -> HashSet<BlockId> {
    let by_id = block_by_id_map(ir_func);
    let mut stack = vec![start];
    let mut seen: HashSet<BlockId> = HashSet::new();

    while let Some(b) = stack.pop() {
        if !seen.insert(b) {
            continue;
        }
        if b == end {
            continue; // Don't expand past end
        }
        if let Some(block) = by_id.get(&b) {
            for succ in block_successors(block) {
                stack.push(succ);
            }
        }
    }
    seen
}

pub(crate) fn has_back_edge(
    ir_func: &IrFunction,
    region_blocks: &HashSet<BlockId>,
    doms: &HashMap<BlockId, HashSet<BlockId>>,
) -> bool {
    let by_id = block_by_id_map(ir_func);
    for &b in region_blocks {
        if let Some(block) = by_id.get(&b) {
            for succ in block_successors(block) {
                if region_blocks.contains(&succ) {
                    // succ dominates b → back edge
                    if doms.get(&b).map(|d| d.contains(&succ)).unwrap_or(false) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

pub(crate) fn region_contains_throw(
    ir_func: &IrFunction,
    region_blocks: &HashSet<BlockId>,
) -> bool {
    let by_id = block_by_id_map(ir_func);
    for &b in region_blocks {
        if let Some(block) = by_id.get(&b) {
            if matches!(block.terminator, Terminator::Unreachable) {
                return true;
            }
            for inst in &block.insts {
                if matches!(inst, Inst::Throw(_)) {
                    return true;
                }
            }
        }
    }
    false
}

// ── Collect use-blocks ───────────────────────────────────────────────────────

/// Walk the propagation chain and collect all blocks where `alloc_var` (or
/// any value derived from it through phi/known-call forwarding) is used.
pub(crate) fn collect_use_blocks(
    alloc_var: VarId,
    uses: &HashMap<VarId, Vec<super::escape::UseInfo>>,
    ir_func: &IrFunction,
) -> HashSet<BlockId> {
    use super::escape::{UseKind, known_fn_arg_escapes};

    let mut worklist: Vec<VarId> = vec![alloc_var];
    let mut visited: HashSet<VarId> = HashSet::new();
    let mut use_blocks: HashSet<BlockId> = HashSet::new();

    while let Some(current) = worklist.pop() {
        if !visited.insert(current) {
            continue;
        }
        for use_info in uses.get(&current).into_iter().flatten() {
            use_blocks.insert(use_info.block);
            match &use_info.kind {
                UseKind::KnownCallArg { func, arg_index }
                    if known_fn_arg_escapes(func, *arg_index) =>
                {
                    // Find the call result and propagate
                    if let Some(block) = ir_func.blocks.iter().find(|b| b.id == use_info.block) {
                        for inst in &block.insts {
                            if let Inst::CallKnown(dst, f, args) = inst
                                && f == func
                                && args.contains(&current)
                            {
                                worklist.push(*dst);
                            }
                        }
                    }
                }
                UseKind::KnownCallArg { .. } => {}
                UseKind::PhiInput => {
                    if let Some(block) = ir_func.blocks.iter().find(|b| b.id == use_info.block) {
                        for phi in &block.phis {
                            if let Inst::Phi(dst, entries) = phi
                                && entries.iter().any(|(_, v)| *v == current)
                            {
                                worklist.push(*dst);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    use_blocks
}

// ── Alloc-op → RegionAllocKind mapping ──────────────────────────────────────

fn alloc_to_region_kind(inst: &Inst) -> Option<RegionAllocKind> {
    match inst {
        Inst::AllocVector(..) => Some(RegionAllocKind::Vector),
        Inst::AllocMap(..) => Some(RegionAllocKind::Map),
        Inst::AllocSet(..) => Some(RegionAllocKind::Set),
        Inst::AllocList(..) => Some(RegionAllocKind::List),
        Inst::AllocCons(..) => Some(RegionAllocKind::Cons),
        _ => None, // AllocClosure not region-allocatable
    }
}

fn alloc_operands(inst: &Inst) -> Vec<VarId> {
    match inst {
        Inst::AllocVector(_, elems) | Inst::AllocSet(_, elems) | Inst::AllocList(_, elems) => {
            elems.clone()
        }
        Inst::AllocMap(_, pairs) => pairs.iter().flat_map(|&(k, v)| [k, v]).collect(),
        Inst::AllocCons(_, head, tail) => vec![*head, *tail],
        _ => vec![],
    }
}

// ── Region rewriting ─────────────────────────────────────────────────────────

fn emit_region_for_alloc(
    mut ir_func: IrFunction,
    alloc_var: VarId,
    alloc_block: BlockId,
    use_blocks: HashSet<BlockId>,
    doms: &HashMap<BlockId, HashSet<BlockId>>,
    postdoms: &HashMap<BlockId, HashSet<BlockId>>,
    next_var: &mut u32,
) -> IrFunction {
    let mut relevant: HashSet<BlockId> = use_blocks;
    relevant.insert(alloc_block);

    let start = match lca_of_many(doms, relevant.iter().copied()) {
        Some(s) => s,
        None => return ir_func,
    };
    let end = match lca_of_many(postdoms, relevant.iter().copied()) {
        Some(e) => e,
        None => return ir_func,
    };

    // Alloc block must be dominated by start
    if !doms
        .get(&alloc_block)
        .map(|d| d.contains(&start))
        .unwrap_or(false)
    {
        return ir_func;
    }

    let region = blocks_on_path(&ir_func, start, end);

    // Check for back edges in the region OR in any use_blocks that fall outside
    // the region. A use_block outside the path (e.g., a loop body block reached
    // via a back edge through the end_block) can create a cycle: the value lives
    // across that back edge, so closing the region at end_block is unsafe.
    let region_with_uses: HashSet<BlockId> =
        region.iter().chain(relevant.iter()).copied().collect();
    if has_back_edge(&ir_func, &region_with_uses, doms) {
        return ir_func;
    }
    if region_contains_throw(&ir_func, &region) {
        return ir_func;
    }

    let region_var = VarId(*next_var);
    *next_var += 1;

    // Rewrite the alloc instruction in alloc_block → RegionAlloc
    for block in &mut ir_func.blocks {
        if block.id == alloc_block {
            for inst in &mut block.insts {
                if inst.dst() == Some(alloc_var)
                    && let Some(kind) = alloc_to_region_kind(inst)
                {
                    let operands = alloc_operands(inst);
                    *inst = Inst::RegionAlloc(alloc_var, region_var, kind, operands);
                }
            }
        }
    }

    // Insert RegionStart at head of `start` block
    for block in &mut ir_func.blocks {
        if block.id == start {
            block.insts.insert(0, Inst::RegionStart(region_var));
            break;
        }
    }

    // Append RegionEnd to `end` block (before terminator)
    for block in &mut ir_func.blocks {
        if block.id == end {
            block.insts.push(Inst::RegionEnd(region_var));
            break;
        }
    }

    ir_func
}

// ── The pass ─────────────────────────────────────────────────────────────────

fn optimize_regions(ir_func: IrFunction, ctx: Option<&EscapeContext>) -> IrFunction {
    let analysis = analyze(&ir_func, ctx);
    let no_escape_allocs: Vec<(VarId, BlockId)> = analysis
        .alloc_blocks
        .iter()
        .filter(|(v, _)| analysis.states.get(v) == Some(&EscapeState::NoEscape))
        .map(|(&v, &b)| (v, b))
        .collect();

    if no_escape_allocs.is_empty() {
        return ir_func;
    }

    let doms = dominators(&ir_func);
    let postdoms = post_dominators(&ir_func);
    let mut next_var = ir_func.next_var;
    let mut result = ir_func;

    for (alloc_var, alloc_block) in no_escape_allocs {
        let use_blocks = collect_use_blocks(alloc_var, &analysis.uses, &result);
        result = emit_region_for_alloc(
            result,
            alloc_var,
            alloc_block,
            use_blocks,
            &doms,
            &postdoms,
            &mut next_var,
        );
    }

    result.next_var = next_var;
    result
}

fn optimize_tree(ir_func: IrFunction, ctx: &EscapeContext) -> IrFunction {
    let subfunctions = ir_func.subfunctions.clone();
    let optimized_subs: Vec<IrFunction> = subfunctions
        .into_iter()
        .map(|sub| optimize_tree(sub, ctx))
        .collect();

    let mut optimized = optimize_regions(ir_func, Some(ctx));
    optimized.subfunctions = optimized_subs;
    optimized
}

// ── Function-scope region wrapping ──────────────────────────────────────────

/// Returns true when `kfn` always produces a non-collection scalar value
/// (Long, Bool, or Nil) that will never be allocated in the active region.
fn is_scalar_knownfn(kfn: &KnownFn) -> bool {
    matches!(
        kfn,
        KnownFn::Add
            | KnownFn::Sub
            | KnownFn::Mul
            | KnownFn::Div
            | KnownFn::Rem
            | KnownFn::Eq
            | KnownFn::Lt
            | KnownFn::Gt
            | KnownFn::Lte
            | KnownFn::Gte
            | KnownFn::Count
            | KnownFn::CountFilter
            | KnownFn::IsNil
            | KnownFn::IsSeq
            | KnownFn::IsVector
            | KnownFn::IsMap
            | KnownFn::IsEmpty
            | KnownFn::IsNumber
            | KnownFn::IsString
            | KnownFn::IsKeyword
            | KnownFn::IsSymbol
            | KnownFn::IsBool
            | KnownFn::IsInt
            | KnownFn::Contains
            | KnownFn::Identical
            | KnownFn::Println
            | KnownFn::Pr
            | KnownFn::Prn
            | KnownFn::Print
    )
}

/// Classify a single instruction's destination as provably scalar, given
/// already-classified vars.  Returns `true` if the dst should be added to
/// the scalar set.
fn inst_is_provably_scalar(inst: &Inst, scalar: &HashSet<VarId>) -> bool {
    match inst {
        Inst::Const(_, c) => matches!(
            c,
            Const::Nil | Const::Bool(_) | Const::Long(_) | Const::Double(_)
        ),
        Inst::CallKnown(_, kfn, _) => is_scalar_knownfn(kfn),
        Inst::Phi(_, arms) => !arms.is_empty() && arms.iter().all(|(_, v)| scalar.contains(v)),
        _ => false,
    }
}

fn inst_dst(inst: &Inst) -> Option<VarId> {
    match inst {
        Inst::Const(d, _)
        | Inst::LoadLocal(d, _)
        | Inst::LoadGlobal(d, _, _)
        | Inst::LoadVar(d, _, _)
        | Inst::AllocVector(d, _)
        | Inst::AllocMap(d, _)
        | Inst::AllocSet(d, _)
        | Inst::AllocList(d, _)
        | Inst::AllocCons(d, _, _)
        | Inst::AllocClosure(d, _, _)
        | Inst::CallKnown(d, _, _)
        | Inst::Call(d, _, _)
        | Inst::CallDirect(d, _, _)
        | Inst::Deref(d, _)
        | Inst::DefVar(d, _, _, _)
        | Inst::SetBang(d, _)
        | Inst::Throw(d)
        | Inst::Phi(d, _)
        | Inst::RegionStart(d)
        | Inst::RegionAlloc(d, _, _, _)
        | Inst::RegionParam(d)
        | Inst::CallWithRegion(d, _, _) => Some(*d),
        Inst::Await { dst, .. } => Some(*dst),
        Inst::Spawn { dst, .. } => Some(*dst),
        Inst::ChanTake { dst, .. } => Some(*dst),
        Inst::RegionEnd(_) | Inst::SourceLoc(_) | Inst::Recur(_) | Inst::ChanPut { .. } => None,
    }
}

/// Returns true when all `Return` terminators in `ir_func` return provably
/// scalar (non-collection) values, making a function-scoped region safe.
fn is_scalar_returning(ir_func: &IrFunction) -> bool {
    let mut scalar: HashSet<VarId> = HashSet::new();
    let mut changed = true;
    while changed {
        changed = false;
        for block in &ir_func.blocks {
            for inst in block.phis.iter().chain(block.insts.iter()) {
                if let Some(dst) = inst_dst(inst)
                    && !scalar.contains(&dst)
                    && inst_is_provably_scalar(inst, &scalar)
                {
                    scalar.insert(dst);
                    changed = true;
                }
            }
        }
    }

    ir_func.blocks.iter().all(|block| match &block.terminator {
        Terminator::Return(var) => scalar.contains(var),
        _ => true,
    })
}

/// Wrap the function body in a function-scoped region: insert `RegionStart`
/// at entry and `RegionEnd` before every `Return` terminator.
fn wrap_function_region(mut ir_func: IrFunction) -> IrFunction {
    let region_var = VarId(ir_func.next_var);
    ir_func.next_var += 1;

    // Insert RegionStart as the first instruction of the entry block.
    if !ir_func.blocks.is_empty() {
        ir_func.blocks[0]
            .insts
            .insert(0, Inst::RegionStart(region_var));
    }

    // Before each Return terminator, emit RegionEnd into the block's insts.
    for block in &mut ir_func.blocks {
        if matches!(block.terminator, Terminator::Return(_)) {
            block.insts.push(Inst::RegionEnd(region_var));
        }
    }

    ir_func
}

/// Apply function-scope region wrapping to every function in the tree that is
/// provably scalar-returning.  Subfunctions are processed first so the parent's
/// analysis sees final instruction shapes.
fn wrap_scalar_returning(ir_func: IrFunction) -> IrFunction {
    let mut ir_func = ir_func;
    let subfunctions = std::mem::take(&mut ir_func.subfunctions);
    ir_func.subfunctions = subfunctions
        .into_iter()
        .map(wrap_scalar_returning)
        .collect();

    if is_scalar_returning(&ir_func) {
        wrap_function_region(ir_func)
    } else {
        ir_func
    }
}

// ── Eager HOF fusion ─────────────────────────────────────────────────────────

/// Fuse lazy HOFs into their eager, compiled consumers:
///
/// * `(count  (filter pred coll))` → `CountFilter(pred, coll)`
/// * `(into to (filter pred coll))` → `IntoFilter(to, pred, coll)`
/// * `(into to (mapcat f  coll))`   → `IntoMapcat(to, f, coll)`
///
/// fired only when the lazy producer's result is consumed *exactly once*, by
/// the consumer it fuses with — so no laziness or seq identity is relied upon.
/// The consumers (`count`/`into`) fully realize their source, so the eager
/// fusion is observationally identical while skipping the interpreted lazy-seq
/// realization (Form re-walking, symbol re-parsing) and the per-element cons
/// allocations that dominate `samples/life.cljrs`'s `step`.
///
/// The dead producer instruction is removed once its consumer is rewritten.
fn fuse_eager_hofs(mut ir_func: IrFunction) -> IrFunction {
    ir_func.subfunctions = ir_func
        .subfunctions
        .into_iter()
        .map(fuse_eager_hofs)
        .collect();

    let uses = build_use_chains(&ir_func);

    // seq_var → (producer_fn, arg0, arg1) for every `filter`/`mapcat` whose
    // result is used exactly once.
    let mut producers: HashMap<VarId, (KnownFn, VarId, VarId)> = HashMap::new();
    for block in &ir_func.blocks {
        for inst in &block.insts {
            if let Inst::CallKnown(dst, kfn @ (KnownFn::Filter | KnownFn::Mapcat), args) = inst
                && args.len() == 2
                && uses.get(dst).map(|u| u.len() == 1).unwrap_or(false)
            {
                producers.insert(*dst, (kfn.clone(), args[0], args[1]));
            }
        }
    }
    if producers.is_empty() {
        return ir_func;
    }

    // Decide fusions by inspecting consumers.  consumer_dst → (fused_fn, args);
    // remove_seqs → producer instructions made dead.
    let mut rewrites: HashMap<VarId, (KnownFn, Vec<VarId>)> = HashMap::new();
    let mut remove_seqs: HashSet<VarId> = HashSet::new();
    for block in &ir_func.blocks {
        for inst in &block.insts {
            let Inst::CallKnown(cdst, cfn, cargs) = inst else {
                continue;
            };
            match cfn {
                // (count seq) where seq = (filter pred coll)
                KnownFn::Count if cargs.len() == 1 => {
                    if let Some((KnownFn::Filter, pred, coll)) = producers.get(&cargs[0]) {
                        rewrites.insert(*cdst, (KnownFn::CountFilter, vec![*pred, *coll]));
                        remove_seqs.insert(cargs[0]);
                    }
                }
                // (into to seq) where seq = (filter pred coll) | (mapcat f coll)
                KnownFn::Into if cargs.len() == 2 => {
                    let to = cargs[0];
                    if let Some((prod, a0, a1)) = producers.get(&cargs[1]) {
                        let fused = match prod {
                            KnownFn::Filter => KnownFn::IntoFilter,
                            KnownFn::Mapcat => KnownFn::IntoMapcat,
                            _ => continue,
                        };
                        rewrites.insert(*cdst, (fused, vec![to, *a0, *a1]));
                        remove_seqs.insert(cargs[1]);
                    }
                }
                _ => {}
            }
        }
    }
    if rewrites.is_empty() {
        return ir_func;
    }

    for block in &mut ir_func.blocks {
        // Drop the now-dead producer instructions.
        block.insts.retain(|inst| match inst {
            Inst::CallKnown(dst, KnownFn::Filter | KnownFn::Mapcat, _) => {
                !remove_seqs.contains(dst)
            }
            _ => true,
        });
        // Rewrite each fused consumer in place.
        for inst in &mut block.insts {
            if let Inst::CallKnown(dst, cfn, cargs) = inst
                && let Some((fused_fn, fused_args)) = rewrites.get(dst)
            {
                *cfn = fused_fn.clone();
                *cargs = fused_args.clone();
            }
        }
    }

    ir_func
}

// ── Top-level pass ───────────────────────────────────────────────────────────

/// Run all optimization passes on an IR function tree.
///
/// Order:
///   0. Eager HOF fusion (`fuse_eager_hofs`) — fuse count(filter),
///      `count(filter …)` into the allocation-free `CountFilter`.
///   1. Inlining — splice eligible callees into call sites so their
///      allocations are visible as local to the caller.
///   2. Local region promotion (`optimize_tree`) — turn `NoEscape`
///      allocations into `RegionAlloc` scoped over the LCA dominator subgraph.
///   3. Cross-function region promotion (`promote_cross_fn_allocs`) — for
///      `Call` sites whose result is `NoEscape`, clone a region-parameterised
///      variant of the callee that uses the caller's region.
///   4. Function-scope region wrapping (`wrap_scalar_returning`) — wrap the
///      bodies of provably scalar-returning functions so their intermediate
///      collection allocations are freed at function return.
pub fn optimize(ir_func: IrFunction) -> IrFunction {
    let ir_func = fuse_eager_hofs(ir_func);
    let ir_func = inline_pass(ir_func);
    let ctx = make_context(&ir_func);
    let ir_func = optimize_tree(ir_func, &ctx);

    // Stage 4 needs a fresh analysis context because the local pass may have
    // rewritten allocations (and added blocks), invalidating the cached
    // per-function summaries the original `ctx` carries.
    let ctx2 = make_context(&ir_func);
    let ir_func = promote_cross_fn_allocs(ir_func, &ctx2);

    // Stage 5: function-scope regions for scalar-returning functions.
    wrap_scalar_returning(ir_func)
}
