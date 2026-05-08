//! Region escape analysis.
//!
//! Classifies allocations as `:no-escape`, `:arg-escape`, `:returns`, or
//! `:escapes`.  Mirrors `cljrs.compiler.escape`.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use crate::{BlockId, Inst, IrFunction, KnownFn, Terminator, VarId};

// ── Escape state lattice ─────────────────────────────────────────────────────

/// Escape classification for an allocation or parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EscapeState {
    NoEscape,
    ArgEscape,
    Returns,
    Escapes,
}

impl EscapeState {
    /// Lattice join: NoEscape ⊑ ArgEscape ⊑ Returns ⊑ Escapes.
    fn join(a: Self, b: Self) -> Self {
        use EscapeState::*;
        match (a, b) {
            (Escapes, _) | (_, Escapes) => Escapes,
            (Returns, _) | (_, Returns) => Returns,
            (ArgEscape, _) | (_, ArgEscape) => ArgEscape,
            _ => NoEscape,
        }
    }
}

// ── Use-chain types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(crate) enum UseKind {
    Return,
    DefVar,
    SetBang,
    ClosureCapture,
    Throw,
    StoredInHeap,
    Recur,
    KnownCallArg { func: KnownFn, arg_index: usize },
    UnknownCallArg { callee: VarId, arg_index: usize },
    PhiInput,
    BranchCond,
    Deref,
    CallCallee,
}

#[derive(Debug, Clone)]
pub(crate) struct UseInfo {
    pub block: BlockId,
    pub kind: UseKind,
}

// ── Known-function escape semantics ─────────────────────────────────────────

/// Does a known function allow the argument at `arg_index` to escape into its return value?
pub(crate) fn known_fn_arg_escapes(func: &KnownFn, arg_index: usize) -> bool {
    use KnownFn::*;
    match func {
        // Non-escaping: predicates, arithmetic, I/O, lookups that return elements
        Get | Nth | Count | Contains | First | Add | Sub | Mul | Div | Rem | Eq | Lt | Gt | Lte
        | Gte | IsNil | IsSeq | IsVector | IsMap | Identical | IsNumber | IsString | IsKeyword
        | IsSymbol | IsBool | IsInt | Str | Deref | AtomDeref | Println | Pr | Prn | Print => false,

        // These return a modified copy of arg 0 → arg 0 escapes; others don't
        Dissoc | Disj => arg_index == 0,
        Rest | Next | Seq => arg_index == 0,
        Transient => arg_index == 0,
        AssocBang | ConjBang => arg_index == 0,
        PersistentBang => arg_index == 0,

        // Default: argument escapes
        _ => true,
    }
}

// ── Collect alloc instructions ───────────────────────────────────────────────

/// Returns `{alloc_var → defining_block_id}` for every allocation in the function.
pub(crate) fn collect_allocs(ir_func: &IrFunction) -> HashMap<VarId, BlockId> {
    let mut result = HashMap::new();
    for block in &ir_func.blocks {
        let block_id = block.id;
        let all_insts = block.phis.iter().chain(block.insts.iter());
        for inst in all_insts {
            if is_alloc_inst(inst) {
                if let Some(dst) = inst.dst() {
                    result.insert(dst, block_id);
                }
            }
        }
    }
    result
}

fn is_alloc_inst(inst: &Inst) -> bool {
    matches!(
        inst,
        Inst::AllocVector(..)
            | Inst::AllocMap(..)
            | Inst::AllocSet(..)
            | Inst::AllocList(..)
            | Inst::AllocCons(..)
            | Inst::AllocClosure(..)
    )
}

// ── Build def-use chains ─────────────────────────────────────────────────────

fn add_use(uses: &mut HashMap<VarId, Vec<UseInfo>>, var: VarId, block: BlockId, kind: UseKind) {
    uses.entry(var).or_default().push(UseInfo { block, kind });
}

fn add_uses_for_inst(uses: &mut HashMap<VarId, Vec<UseInfo>>, inst: &Inst, block_id: BlockId) {
    match inst {
        Inst::CallKnown(_, func, args) => {
            for (i, &arg) in args.iter().enumerate() {
                add_use(
                    uses,
                    arg,
                    block_id,
                    UseKind::KnownCallArg {
                        func: func.clone(),
                        arg_index: i,
                    },
                );
            }
        }
        Inst::Call(_, callee, args) => {
            add_use(uses, *callee, block_id, UseKind::CallCallee);
            for (i, &arg) in args.iter().enumerate() {
                add_use(
                    uses,
                    arg,
                    block_id,
                    UseKind::UnknownCallArg {
                        callee: *callee,
                        arg_index: i,
                    },
                );
            }
        }
        Inst::AllocClosure(_, _, captures) => {
            for &cap in captures {
                add_use(uses, cap, block_id, UseKind::ClosureCapture);
            }
        }
        Inst::AllocVector(_, elems) | Inst::AllocSet(_, elems) | Inst::AllocList(_, elems) => {
            for &elem in elems {
                add_use(uses, elem, block_id, UseKind::StoredInHeap);
            }
        }
        Inst::AllocMap(_, pairs) => {
            for &(k, v) in pairs {
                add_use(uses, k, block_id, UseKind::StoredInHeap);
                add_use(uses, v, block_id, UseKind::StoredInHeap);
            }
        }
        Inst::AllocCons(_, head, tail) => {
            add_use(uses, *head, block_id, UseKind::StoredInHeap);
            add_use(uses, *tail, block_id, UseKind::StoredInHeap);
        }
        Inst::DefVar(_, _, _, value) => {
            add_use(uses, *value, block_id, UseKind::DefVar);
        }
        Inst::SetBang(var, value) => {
            add_use(uses, *var, block_id, UseKind::SetBang);
            add_use(uses, *value, block_id, UseKind::SetBang);
        }
        Inst::Deref(_, src) => {
            add_use(uses, *src, block_id, UseKind::Deref);
        }
        Inst::Throw(value) => {
            add_use(uses, *value, block_id, UseKind::Throw);
        }
        Inst::Recur(args) => {
            for &arg in args {
                add_use(uses, arg, block_id, UseKind::Recur);
            }
        }
        Inst::Phi(_, entries) => {
            for &(_, var) in entries {
                add_use(uses, var, block_id, UseKind::PhiInput);
            }
        }
        // Const, LoadLocal, LoadGlobal, LoadVar, SourceLoc, RegionStart, RegionEnd — no uses
        Inst::RegionAlloc(_, region, _, operands) => {
            add_use(uses, *region, block_id, UseKind::StoredInHeap);
            for &op in operands {
                add_use(uses, op, block_id, UseKind::StoredInHeap);
            }
        }
        _ => {}
    }
}

fn add_uses_for_terminator(
    uses: &mut HashMap<VarId, Vec<UseInfo>>,
    term: &Terminator,
    block_id: BlockId,
) {
    match term {
        Terminator::Return(var) => {
            add_use(uses, *var, block_id, UseKind::Return);
        }
        Terminator::Branch { cond, .. } => {
            add_use(uses, *cond, block_id, UseKind::BranchCond);
        }
        Terminator::RecurJump { args, .. } => {
            for &arg in args {
                add_use(uses, arg, block_id, UseKind::Recur);
            }
        }
        // Jump, Unreachable — no uses
        _ => {}
    }
}

pub(crate) fn build_use_chains(ir_func: &IrFunction) -> HashMap<VarId, Vec<UseInfo>> {
    let mut uses: HashMap<VarId, Vec<UseInfo>> = HashMap::new();
    for block in &ir_func.blocks {
        let block_id = block.id;
        for inst in &block.phis {
            add_uses_for_inst(&mut uses, inst, block_id);
        }
        for inst in &block.insts {
            add_uses_for_inst(&mut uses, inst, block_id);
        }
        add_uses_for_terminator(&mut uses, &block.terminator, block_id);
    }
    uses
}

// ── Build var-def map ────────────────────────────────────────────────────────

pub(crate) fn build_var_defs(ir_func: &IrFunction) -> HashMap<VarId, &Inst> {
    let mut defs: HashMap<VarId, &Inst> = HashMap::new();
    for block in &ir_func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Some(dst) = inst.dst() {
                defs.insert(dst, inst);
            }
        }
    }
    defs
}

// ── Inter-procedural support ─────────────────────────────────────────────────

/// Walk the IR tree depth-first, root first.
fn walk_functions(root: &IrFunction) -> Vec<&IrFunction> {
    let mut result = vec![root];
    for sub in &root.subfunctions {
        result.extend(walk_functions(sub));
    }
    result
}

/// Closure info for inter-procedural lookup.
#[derive(Debug, Clone)]
pub(crate) struct ClosureInfo {
    pub arity_fn_names: Vec<Arc<str>>,
    pub param_counts: Vec<usize>,
    pub is_variadic: Vec<bool>,
}

/// Build `{[ns, name] → ClosureInfo}` from the IR tree.
pub(crate) fn build_defn_map(root: &IrFunction) -> HashMap<(Arc<str>, Arc<str>), ClosureInfo> {
    let mut result = HashMap::new();
    for func in walk_functions(root) {
        for block in &func.blocks {
            // Collect alloc-closure info in this block
            let mut alloc_info: HashMap<VarId, ClosureInfo> = HashMap::new();
            for inst in &block.insts {
                if let Inst::AllocClosure(dst, tmpl, _) = inst {
                    alloc_info.insert(
                        *dst,
                        ClosureInfo {
                            arity_fn_names: tmpl.arity_fn_names.clone(),
                            param_counts: tmpl.param_counts.clone(),
                            is_variadic: tmpl.is_variadic.clone(),
                        },
                    );
                }
            }
            // Match DefVar instructions against alloc_info
            for inst in &block.insts {
                if let Inst::DefVar(_, ns, name, value) = inst {
                    if let Some(info) = alloc_info.get(value) {
                        result.insert((ns.clone(), name.clone()), info.clone());
                    }
                }
            }
        }
    }
    result
}

/// Build `{arity_fn_name → IrFunction}` registry.
pub(crate) fn build_fn_registry(root: &IrFunction) -> HashMap<Arc<str>, IrFunction> {
    let mut result = HashMap::new();
    for func in walk_functions(root) {
        if let Some(name) = &func.name {
            result.insert(name.clone(), func.clone());
        }
    }
    result
}

/// Pick the fixed arity from `info` whose param count matches `arg_count`.
fn pick_arity(info: &ClosureInfo, arg_count: usize) -> Option<Arc<str>> {
    for (i, &count) in info.param_counts.iter().enumerate() {
        if count == arg_count && !info.is_variadic[i] {
            return Some(info.arity_fn_names[i].clone());
        }
    }
    None
}

/// Resolve the callee of a `Call` instruction to a concrete arity-fn-name.
fn resolve_call_target(
    callee_var: VarId,
    arg_count: usize,
    var_defs: &HashMap<VarId, &Inst>,
    defn_map: &HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
) -> Option<Arc<str>> {
    let def_inst = var_defs.get(&callee_var)?;
    match def_inst {
        Inst::AllocClosure(_, tmpl, _) => {
            let info = ClosureInfo {
                arity_fn_names: tmpl.arity_fn_names.clone(),
                param_counts: tmpl.param_counts.clone(),
                is_variadic: tmpl.is_variadic.clone(),
            };
            pick_arity(&info, arg_count)
        }
        Inst::LoadGlobal(_, ns, name) => {
            let info = defn_map.get(&(ns.clone(), name.clone()))?;
            pick_arity(info, arg_count)
        }
        Inst::Deref(_, src) => {
            let src_def = var_defs.get(src)?;
            match src_def {
                Inst::LoadGlobal(_, ns, name) | Inst::LoadVar(_, ns, name) => {
                    let info = defn_map.get(&(ns.clone(), name.clone()))?;
                    pick_arity(info, arg_count)
                }
                _ => None,
            }
        }
        _ => None,
    }
}

// ── Find helpers ─────────────────────────────────────────────────────────────

/// Find the destination of a CallKnown for `known_fn` that uses `used_var` in `block_id`.
fn find_call_result(
    used_var: VarId,
    known_fn: &KnownFn,
    ir_func: &IrFunction,
    block_id: BlockId,
) -> Option<VarId> {
    let block = ir_func.blocks.iter().find(|b| b.id == block_id)?;
    for inst in &block.insts {
        if let Inst::CallKnown(dst, func, args) = inst {
            if func == known_fn && args.contains(&used_var) {
                return Some(*dst);
            }
        }
    }
    None
}

/// Find a Call instruction in `block_id` with the given callee and arg.
fn find_unknown_call_with_arg(
    ir_func: &IrFunction,
    callee_var: VarId,
    arg_var: VarId,
    block_id: BlockId,
) -> Option<(VarId, usize)> {
    let block = ir_func.blocks.iter().find(|b| b.id == block_id)?;
    for inst in &block.insts {
        if let Inst::Call(dst, callee, args) = inst {
            if *callee == callee_var && args.contains(&arg_var) {
                return Some((*dst, args.len()));
            }
        }
    }
    None
}

// ── Escape classification mode ───────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum EscapeMode {
    Alloc,
    Param,
}

// ── Inter-procedural context ─────────────────────────────────────────────────

pub(crate) struct EscapeContext {
    pub registry: HashMap<Arc<str>, IrFunction>,
    pub defn_map: HashMap<(Arc<str>, Arc<str>), ClosureInfo>,
    pub cache: RefCell<HashMap<Arc<str>, Vec<EscapeState>>>,
    pub computing: RefCell<HashSet<Arc<str>>>,
}

pub(crate) fn make_context(root: &IrFunction) -> EscapeContext {
    EscapeContext {
        registry: build_fn_registry(root),
        defn_map: build_defn_map(root),
        cache: RefCell::new(HashMap::new()),
        computing: RefCell::new(HashSet::new()),
    }
}

// ── Core classification ──────────────────────────────────────────────────────

/// Compute the per-parameter escape summary for `ir_func`.
/// Returns one `EscapeState` per parameter.
pub(crate) fn compute_fn_summary(ir_func: &IrFunction, ctx: &EscapeContext) -> Vec<EscapeState> {
    let fn_name = ir_func.name.as_ref();

    // Check cache first
    if let Some(name) = fn_name {
        if let Some(cached) = ctx.cache.borrow().get(name) {
            return cached.clone();
        }
        if ctx.computing.borrow().contains(name) {
            // Cycle guard: conservative all-Escapes
            return ir_func
                .params
                .iter()
                .map(|_| EscapeState::Escapes)
                .collect();
        }
        ctx.computing.borrow_mut().insert(name.clone());
    }

    let uses = build_use_chains(ir_func);
    let var_defs = build_var_defs(ir_func);

    let summary: Vec<EscapeState> = ir_func
        .params
        .iter()
        .map(|(_, pv)| {
            classify_escape_with_ctx(
                *pv,
                &uses,
                ir_func,
                Some(ctx),
                Some(&var_defs),
                EscapeMode::Param,
            )
        })
        .collect();

    if let Some(name) = fn_name {
        ctx.cache.borrow_mut().insert(name.clone(), summary.clone());
        ctx.computing.borrow_mut().remove(name);
    }

    summary
}

/// Worklist-based escape classification.
pub(crate) fn classify_escape_with_ctx(
    var: VarId,
    uses: &HashMap<VarId, Vec<UseInfo>>,
    ir_func: &IrFunction,
    ctx: Option<&EscapeContext>,
    var_defs: Option<&HashMap<VarId, &Inst>>,
    mode: EscapeMode,
) -> EscapeState {
    let mut worklist: VecDeque<VarId> = VecDeque::new();
    let mut visited: HashSet<VarId> = HashSet::new();
    let mut result = EscapeState::NoEscape;

    worklist.push_back(var);

    'outer: while let Some(current) = worklist.pop_front() {
        if !visited.insert(current) {
            continue;
        }

        let use_list = match uses.get(&current) {
            Some(v) if !v.is_empty() => v,
            _ => continue,
        };

        for use_info in use_list {
            match &use_info.kind {
                UseKind::Return => {
                    if mode == EscapeMode::Param {
                        result = EscapeState::join(result, EscapeState::Returns);
                    } else {
                        result = EscapeState::Escapes;
                        break 'outer;
                    }
                }
                UseKind::DefVar
                | UseKind::SetBang
                | UseKind::ClosureCapture
                | UseKind::Throw
                | UseKind::StoredInHeap
                | UseKind::Recur => {
                    result = EscapeState::Escapes;
                    break 'outer;
                }
                UseKind::UnknownCallArg { callee, arg_index } => {
                    // Try inter-procedural lookup
                    let resolved = if let (Some(ctx), Some(vd)) = (ctx, var_defs) {
                        if let Some((_, arg_count)) =
                            find_unknown_call_with_arg(ir_func, *callee, current, use_info.block)
                        {
                            resolve_call_target(*callee, arg_count, vd, &ctx.defn_map)
                                .and_then(|name| ctx.registry.get(&name).cloned())
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    if let Some(target_fn) = resolved {
                        if let Some(ectx) = ctx {
                            let summary = compute_fn_summary(&target_fn, ectx);
                            match summary
                                .get(*arg_index)
                                .copied()
                                .unwrap_or(EscapeState::Escapes)
                            {
                                EscapeState::NoEscape => {} // stays no-escape
                                EscapeState::Returns => {
                                    // The return value of the call may alias this alloc
                                    if let Some((call_dst, _)) = find_unknown_call_with_arg(
                                        ir_func,
                                        *callee,
                                        current,
                                        use_info.block,
                                    ) {
                                        worklist.push_back(call_dst);
                                    }
                                }
                                _ => {
                                    result = EscapeState::Escapes;
                                    break 'outer;
                                }
                            }
                        }
                    } else if mode == EscapeMode::Param {
                        result = EscapeState::Escapes;
                        break 'outer;
                    } else {
                        // Conservative: arg-escape
                        result = EscapeState::join(result, EscapeState::ArgEscape);
                    }
                    let _ = arg_index; // suppress unused warning
                }
                UseKind::KnownCallArg { func, arg_index } => {
                    if known_fn_arg_escapes(func, *arg_index) {
                        // The call result may carry this alloc
                        if let Some(call_result) =
                            find_call_result(current, func, ir_func, use_info.block)
                        {
                            worklist.push_back(call_result);
                        } else {
                            result = EscapeState::Escapes;
                            break 'outer;
                        }
                    }
                    // else: doesn't escape through this call
                }
                UseKind::PhiInput => {
                    // Propagate through phi outputs
                    if let Some(block) = ir_func.blocks.iter().find(|b| b.id == use_info.block) {
                        for phi in &block.phis {
                            if let Inst::Phi(dst, entries) = phi {
                                if entries.iter().any(|(_, v)| *v == current) {
                                    worklist.push_back(*dst);
                                }
                            }
                        }
                    }
                }
                // BranchCond, Deref, CallCallee — don't cause escape
                _ => {}
            }

            if result == EscapeState::Escapes {
                break 'outer;
            }
        }
    }

    result
}

// ── Public analysis result ───────────────────────────────────────────────────

pub(crate) struct AnalysisResult {
    pub states: HashMap<VarId, EscapeState>,
    pub uses: HashMap<VarId, Vec<UseInfo>>,
    pub alloc_blocks: HashMap<VarId, BlockId>,
}

pub(crate) fn analyze(ir_func: &IrFunction, ctx: Option<&EscapeContext>) -> AnalysisResult {
    let alloc_blocks = collect_allocs(ir_func);
    let uses = build_use_chains(ir_func);
    let var_defs = build_var_defs(ir_func);

    let states: HashMap<VarId, EscapeState> = alloc_blocks
        .keys()
        .map(|&alloc_var| {
            let state = classify_escape_with_ctx(
                alloc_var,
                &uses,
                ir_func,
                ctx,
                Some(&var_defs),
                EscapeMode::Alloc,
            );
            (alloc_var, state)
        })
        .collect();

    AnalysisResult {
        states,
        uses,
        alloc_blocks,
    }
}
