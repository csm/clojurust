//! Stage-4 cross-function region promotion.
//!
//! For `Call(dst, callee, args)` sites where:
//!   * `dst` is `NoEscape` in the caller, **and**
//!   * the callee has `Returns`-tagged allocations
//!
//! this pass clones a *region-parameterised* variant of the callee whose
//! `Returns` allocs become [`Inst::RegionAlloc`].  At the caller, the call
//! site is wrapped in [`Inst::RegionStart`]/[`Inst::RegionEnd`] and the
//! `Call` is rewritten to a [`Inst::CallWithRegion`] targeting the
//! cloned variant by name.
//!
//! At runtime, the cloned variant's `RegionAlloc` instructions consult the
//! thread-local region stack — already populated by the caller's
//! `RegionStart` — and bump-allocate into the caller's region.  When the
//! caller's `RegionEnd` fires the allocations are freed.
//!
//! The clones are attached as additional subfunctions of the **calling**
//! function (the one containing the rewritten `Call`).  This keeps them
//! reachable both for the IR interpreter (which looks up `CallWithRegion`
//! targets in `ir_func.subfunctions`) and for codegen (which recursively
//! walks the tree, declares every subfunction in `user_funcs`, and
//! direct-calls them by name).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::{Inst, IrFunction, RegionAllocKind, VarId};

use super::escape::{
    ClosureInfo, EscapeContext, EscapeMode, EscapeState, build_use_chains, build_var_defs,
    classify_escape_with_ctx, collect_allocs,
};
use super::optimize::{
    blocks_on_path, collect_use_blocks, dominators, has_back_edge, lca_of_many, post_dominators,
    region_contains_throw,
};

// ── Per-call-site candidate ──────────────────────────────────────────────────

/// A `Call` instruction worth rewriting into `CallWithRegion`.
struct Candidate {
    /// `dst` of the `Call` — used to re-locate the instruction after each
    /// rewrite step (which may shift block-relative indices).
    dst: VarId,
    /// Resolved arity-fn name of the original callee.
    callee_fn_name: Arc<str>,
    /// VarIds (in the *callee's* scope) of the `Returns` allocs that should
    /// become `RegionAlloc` in the cloned variant.
    returns_allocs: HashSet<VarId>,
    /// Number of leading capture parameters the callee expects (computed as
    /// `callee.params.len() - args.len()`).  Always 0 or 1 for stage 4 —
    /// candidates with multi-capture callees are rejected upstream because
    /// we can't reconstruct the captures from a `Call` site.
    capture_count: usize,
}

// ── Resolution helpers ──────────────────────────────────────────────────────

/// Walk `(LoadGlobal | Deref(LoadGlobal|LoadVar))` and look up the callee
/// function via `defn_map`.  Returns the arity-fn name matching the call's
/// arg count (non-variadic only).
fn resolve_callee_name(
    callee_var: VarId,
    arg_count: usize,
    var_defs: &HashMap<VarId, &Inst>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
) -> Option<Arc<str>> {
    let def_inst = var_defs.get(&callee_var)?;
    let info: &ClosureInfo = match def_inst {
        Inst::LoadGlobal(_, ns, name) => defn_map.get(&(ns.clone(), name.clone()))?,
        Inst::Deref(_, src) => match var_defs.get(src)? {
            Inst::LoadGlobal(_, ns, name) | Inst::LoadVar(_, ns, name) => {
                defn_map.get(&(ns.clone(), name.clone()))?
            }
            _ => return None,
        },
        _ => return None,
    };
    for (i, &count) in info.param_counts.iter().enumerate() {
        if count == arg_count && !info.is_variadic[i] {
            return Some(info.arity_fn_names[i].clone());
        }
    }
    None
}

/// Compute the set of `Returns`-tagged allocations for a callee.
fn returns_allocs_of(callee: &IrFunction, ctx: &EscapeContext) -> HashSet<VarId> {
    let alloc_blocks = collect_allocs(callee);
    let uses = build_use_chains(callee);
    let var_defs = build_var_defs(callee);
    alloc_blocks
        .keys()
        .filter_map(|&alloc| {
            let state = classify_escape_with_ctx(
                alloc,
                &uses,
                callee,
                Some(ctx),
                Some(&var_defs),
                EscapeMode::Alloc,
            );
            if state == EscapeState::Returns {
                Some(alloc)
            } else {
                None
            }
        })
        .collect()
}

// ── Specialised-callee construction ──────────────────────────────────────────

fn alloc_to_region_kind(inst: &Inst) -> Option<RegionAllocKind> {
    match inst {
        Inst::AllocVector(..) => Some(RegionAllocKind::Vector),
        Inst::AllocMap(..) => Some(RegionAllocKind::Map),
        Inst::AllocSet(..) => Some(RegionAllocKind::Set),
        Inst::AllocList(..) => Some(RegionAllocKind::List),
        Inst::AllocCons(..) => Some(RegionAllocKind::Cons),
        // Closures aren't region-allocatable.
        _ => None,
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

/// Recursively rename every named subfunction in `func` (and its nested
/// subfunctions) by appending `suffix`, and rewrite all `AllocClosure`
/// instructions whose `arity_fn_names` reference one of the renamed names.
///
/// Without this rename the cloned IR tree would contain duplicates of every
/// inner closure's arity-fn, and codegen's `declare_subfunctions` would call
/// `declare_function` twice with the same name → `Module(DuplicateDefinition)`.
fn rename_inner_subfunctions(func: &mut IrFunction, suffix: &str) {
    // Pass 1: collect old → new name mapping by walking every subfunction
    // (including recursively nested ones).
    let mut name_map: HashMap<Arc<str>, Arc<str>> = HashMap::new();
    fn collect(f: &IrFunction, suffix: &str, map: &mut HashMap<Arc<str>, Arc<str>>) {
        for sub in &f.subfunctions {
            if let Some(name) = &sub.name {
                let new_name: Arc<str> = Arc::from(format!("{name}{suffix}").as_str());
                map.insert(name.clone(), new_name);
            }
            collect(sub, suffix, map);
        }
    }
    collect(func, suffix, &mut name_map);
    if name_map.is_empty() {
        return;
    }

    // Pass 2: rewrite `arity_fn_names` references in every `AllocClosure`
    // throughout the tree, and update each subfunction's own `name` field.
    fn rewrite(f: &mut IrFunction, map: &HashMap<Arc<str>, Arc<str>>) {
        for block in &mut f.blocks {
            for inst in block.phis.iter_mut().chain(block.insts.iter_mut()) {
                if let Inst::AllocClosure(_, tmpl, _) = inst {
                    for n in &mut tmpl.arity_fn_names {
                        if let Some(new_name) = map.get(n) {
                            *n = new_name.clone();
                        }
                    }
                }
            }
        }
        for sub in &mut f.subfunctions {
            if let Some(name) = &sub.name
                && let Some(new_name) = map.get(name)
            {
                sub.name = Some(new_name.clone());
            }
            rewrite(sub, map);
        }
    }
    rewrite(func, &name_map);
}

/// Clone `original` and rewrite each `Returns` alloc in `targets` as
/// `RegionAlloc(dst, region_var, kind, ops)`, with a `RegionParam(region_var)`
/// inserted at the entry block's prologue.
///
/// Returns the cloned variant with `name = Some(<orig>__rg)`.  Returns
/// `None` if no allocation in `targets` is region-promotable (e.g. they're
/// all closures).
fn specialize(original: &IrFunction, targets: &HashSet<VarId>, suffix: &str) -> Option<IrFunction> {
    let mut clone = original.clone();
    let region_var = VarId(clone.next_var);
    clone.next_var += 1;

    let mut promoted_any = false;
    for block in &mut clone.blocks {
        for inst in &mut block.insts {
            let Some(dst) = inst.dst() else {
                continue;
            };
            if !targets.contains(&dst) {
                continue;
            }
            let Some(kind) = alloc_to_region_kind(inst) else {
                continue;
            };
            let ops = alloc_operands(inst);
            *inst = Inst::RegionAlloc(dst, region_var, kind, ops);
            promoted_any = true;
        }
    }
    if !promoted_any {
        return None;
    }

    if let Some(entry) = clone.blocks.first_mut() {
        entry.insts.insert(0, Inst::RegionParam(region_var));
    } else {
        return None;
    }

    // Inner closures cloned along with the body share names with the
    // original's inner closures — declaring them twice would explode in
    // codegen.  Give every inner subfunction (and matching `AllocClosure`
    // references) a fresh suffixed name.
    rename_inner_subfunctions(&mut clone, suffix);

    let new_name: Arc<str> = match &original.name {
        Some(n) => Arc::from(format!("{n}{suffix}").as_str()),
        None => Arc::from(format!("__cljrs_anon{suffix}").as_str()),
    };
    clone.name = Some(new_name);
    Some(clone)
}

// ── Caller-side rewrite ──────────────────────────────────────────────────────

/// Locate the `Call(dst, _, _)` instruction that defines `dst` in `func`
/// and return its `(block_idx, inst_idx)`.  `Call` instructions have unique
/// `dst`s, so this is a safe re-discovery after intervening insertions.
fn find_call_by_dst(func: &IrFunction, dst: VarId) -> Option<(usize, usize)> {
    for (b_idx, block) in func.blocks.iter().enumerate() {
        for (i_idx, inst) in block.insts.iter().enumerate() {
            if let Inst::Call(d, _, _) = inst
                && *d == dst
            {
                return Some((b_idx, i_idx));
            }
        }
    }
    None
}

/// Compute the dom/postdom-based region scope for `dst` and rewrite the
/// `Call` site in place.  Returns `true` if the rewrite succeeded.
///
/// On success: replaces the `Call` with `CallWithRegion(dst, target_name,
/// args)`, prepends `RegionStart(rv)` to the LCA-block's prologue, and
/// appends `RegionEnd(rv)` to the LCA-postdom's instruction list (before
/// the terminator).  Bails out without mutation if back-edges or `throw`
/// instructions cross the candidate region — matching the safety
/// constraints of the local region-promotion pass.
fn rewrite_call_with_region_scope(
    func: &mut IrFunction,
    dst: VarId,
    target_name: Arc<str>,
    capture_count: usize,
) -> bool {
    let Some((block_idx, inst_idx)) = find_call_by_dst(func, dst) else {
        return false;
    };
    let alloc_block = func.blocks[block_idx].id;

    let uses = build_use_chains(func);
    let mut use_blocks = collect_use_blocks(dst, &uses, func);
    use_blocks.insert(alloc_block);

    let doms = dominators(func);
    let postdoms = post_dominators(func);

    let start_block = match lca_of_many(&doms, use_blocks.iter().copied()) {
        Some(b) => b,
        None => return false,
    };
    let end_block = match lca_of_many(&postdoms, use_blocks.iter().copied()) {
        Some(b) => b,
        None => return false,
    };
    // The call's defining block must be dominated by `start_block`.
    if !doms
        .get(&alloc_block)
        .map(|d| d.contains(&start_block))
        .unwrap_or(false)
    {
        return false;
    }

    let region_blocks = blocks_on_path(func, start_block, end_block);
    // Include all use_blocks in the back-edge check: a use_block outside the
    // region path can be reached via a loop back edge through the end_block,
    // meaning the value lives across that back edge and the region would be
    // freed while the value is still reachable.
    let region_with_uses: std::collections::HashSet<_> = region_blocks
        .iter()
        .chain(use_blocks.iter())
        .copied()
        .collect();
    if has_back_edge(func, &region_with_uses, &doms) {
        return false;
    }
    if region_contains_throw(func, &region_blocks) {
        return false;
    }

    // All checks pass — perform the rewrite.
    let region_var = VarId(func.next_var);
    func.next_var += 1;

    // Replace `Call` with `CallWithRegion`.  If the callee expects a leading
    // self/closure capture parameter (`capture_count == 1`) prepend the
    // call's own `callee` VarId so the cloned variant receives the closure
    // object as its first argument — matching the `do_inline` calling
    // convention.
    let Inst::Call(call_dst, callee, args) = func.blocks[block_idx].insts[inst_idx].clone() else {
        return false;
    };
    debug_assert_eq!(call_dst, dst);
    let full_args: Vec<VarId> = if capture_count == 1 {
        let mut v = Vec::with_capacity(args.len() + 1);
        v.push(callee);
        v.extend(args);
        v
    } else {
        args
    };
    func.blocks[block_idx].insts[inst_idx] = Inst::CallWithRegion(dst, target_name, full_args);

    // Insert RegionStart at the head of `start_block`.
    if let Some(b) = func.blocks.iter_mut().find(|b| b.id == start_block) {
        b.insts.insert(0, Inst::RegionStart(region_var));
    }

    // Append RegionEnd before the terminator of `end_block`.
    if let Some(b) = func.blocks.iter_mut().find(|b| b.id == end_block) {
        b.insts.push(Inst::RegionEnd(region_var));
    }

    true
}

// ── Pass driver ──────────────────────────────────────────────────────────────

/// Walk the IR tree (root + every subfunction) and return a flat list of
/// candidate call sites along with the path to the function containing them.
///
/// Path encoding: an empty path means "root"; non-empty paths index into
/// `subfunctions` recursively.
struct CandidateLoc {
    /// Path to the enclosing function, where `path[i]` is the index into
    /// `subfunctions` at depth `i`.
    path: Vec<usize>,
    candidate: Candidate,
}

fn collect_candidates_in(
    func: &IrFunction,
    path: Vec<usize>,
    ctx: &EscapeContext,
    out: &mut Vec<CandidateLoc>,
) {
    let uses = build_use_chains(func);
    let var_defs = build_var_defs(func);

    for block in func.blocks.iter() {
        for inst in block.insts.iter() {
            let Inst::Call(dst, callee, args) = inst else {
                continue;
            };
            let dst_state = classify_escape_with_ctx(
                *dst,
                &uses,
                func,
                Some(ctx),
                Some(&var_defs),
                EscapeMode::Alloc,
            );
            if dst_state != EscapeState::NoEscape {
                continue;
            }
            let Some(callee_name) =
                resolve_callee_name(*callee, args.len(), &var_defs, &ctx.defn_map)
            else {
                continue;
            };
            let Some(callee_fn) = ctx.registry.get(&callee_name) else {
                continue;
            };
            // The callee's `params` includes leading capture parameters
            // (typically 0 or 1: the self-ref of a top-level `defn`).  For
            // stage 4 we can only reconstruct the call's full argument list
            // when there are 0 captures (pass through `args` 1-to-1) or
            // exactly 1 capture (prepend the call site's `callee_var`, which
            // *is* the closure object and serves as the self-ref).  Anything
            // beyond a single self-cap requires knowing which closed-over
            // values to pass — information not present at the call site.
            let total_params = callee_fn.params.len();
            if total_params < args.len() {
                continue;
            }
            let capture_count = total_params - args.len();
            if capture_count > 1 {
                continue;
            }
            let returns_allocs = returns_allocs_of(callee_fn, ctx);
            if returns_allocs.is_empty() {
                continue;
            }
            // Skip if none of the returns_allocs are actually region-promotable
            // (e.g. they're all closures).
            let any_promotable = returns_allocs.iter().any(|alloc_var| {
                callee_fn.blocks.iter().any(|b| {
                    b.insts
                        .iter()
                        .any(|i| i.dst() == Some(*alloc_var) && alloc_to_region_kind(i).is_some())
                })
            });
            if !any_promotable {
                continue;
            }
            out.push(CandidateLoc {
                path: path.clone(),
                candidate: Candidate {
                    dst: *dst,
                    callee_fn_name: callee_name,
                    returns_allocs,
                    capture_count,
                },
            });
        }
    }

    for (i, sub) in func.subfunctions.iter().enumerate() {
        let mut sub_path = path.clone();
        sub_path.push(i);
        collect_candidates_in(sub, sub_path, ctx, out);
    }
}

/// Borrow a function in the tree by `path`.
fn fn_at_path_mut<'a>(root: &'a mut IrFunction, path: &[usize]) -> &'a mut IrFunction {
    let mut cur = root;
    for &i in path {
        cur = &mut cur.subfunctions[i];
    }
    cur
}

/// Run stage-4 promotion over the tree rooted at `root`.  Specialised callee
/// variants are attached as subfunctions of the function containing the
/// rewritten call site (memoised per caller × call-signature).
pub fn promote_cross_fn_allocs(mut root: IrFunction, ctx: &EscapeContext) -> IrFunction {
    let mut candidates: Vec<CandidateLoc> = Vec::new();
    collect_candidates_in(&root, Vec::new(), ctx, &mut candidates);

    if candidates.is_empty() {
        return root;
    }

    // Memoise specialisations by (caller_path, callee_fn_name,
    // sorted_returns_allocs).  Two call sites in the same caller calling the
    // same callee with the same alloc-set share a single clone.
    type SpecialiseKey = (Vec<usize>, Arc<str>, Vec<u32>);
    let mut specialised: HashMap<SpecialiseKey, Arc<str>> = HashMap::new();
    let mut counter: usize = 0;

    for loc in candidates {
        let CandidateLoc { path, candidate } = loc;

        let mut alloc_key: Vec<u32> = candidate.returns_allocs.iter().map(|v| v.0).collect();
        alloc_key.sort_unstable();
        let key = (path.clone(), candidate.callee_fn_name.clone(), alloc_key);

        // Pre-flight: try the rewrite WITHOUT installing the clone first by
        // checking whether the dom/postdom analysis will succeed.  We do this
        // by attempting the rewrite — on failure the function is untouched.
        // Need the clone in place first so the target_name is real, but the
        // clone itself is harmless if the rewrite fails (it just adds dead
        // code).  The pass below skips installing duplicates via memoisation.

        let target_name = if let Some(n) = specialised.get(&key) {
            n.clone()
        } else {
            let original = match ctx.registry.get(&candidate.callee_fn_name) {
                Some(f) => f.clone(),
                None => continue,
            };
            counter += 1;
            let suffix = format!("__rg{counter}");
            let Some(clone) = specialize(&original, &candidate.returns_allocs, &suffix) else {
                continue;
            };
            let new_name = clone.name.clone().expect("specialised has name");
            // Attach the clone as a subfunction of the calling function so
            // both the interpreter (via `ir_func.subfunctions`) and codegen
            // (via tree-walking declaration) can find it by name.
            let caller = fn_at_path_mut(&mut root, &path);
            caller.subfunctions.push(clone);
            specialised.insert(key, new_name.clone());
            new_name
        };

        let caller = fn_at_path_mut(&mut root, &path);
        let _ok = rewrite_call_with_region_scope(
            caller,
            candidate.dst,
            target_name,
            candidate.capture_count,
        );
        // If rewrite failed, the clone we installed is dead but harmless.
        // It will only be referenced from `CallWithRegion` instructions.
    }

    root
}
