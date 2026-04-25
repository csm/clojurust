//! No-GC blacklist analysis for the AOT compilation pipeline.
//!
//! In `no-gc` mode every function pushes a scratch `Region` for intermediates
//! and pops it before evaluating the tail / return expression so that the
//! return value lands in the **caller's** allocation context.  Three patterns
//! cannot be compiled correctly in that model and are flagged here:
//!
//! 1. **Interior-pointer return** — the return variable is (transitively
//!    through phi nodes) the result of an allocation instruction within the
//!    function body.  Such values live in the scratch region and dangle as
//!    soon as the region is reset after the function returns.
//!
//! 2. **Region → static store** — an allocation result flows directly into a
//!    `DefVar` or `SetBang` instruction.  These operations write into
//!    program-lifetime containers (`Var` root / `set!`), so the stored value
//!    must come from the static arena, not a scratch region.
//!
//! 3. **Lazy-sequence escape** — a lazy-sequence-producing call result is
//!    bound as an intermediate (used more than once) and then returned.  The
//!    thunk captures references into the scratch region that dangle after reset.
//!
//! 4. **Escaping closure** — an `AllocClosure` result that captures
//!    region-local values is stored in a static container (`DefVar` / `SetBang`).
//!
//! These checks run on the Rust-level `IrFunction` after the Clojure-level
//! escape analysis and optimization passes.  They are only compiled when the
//! `no-gc` Cargo feature is enabled.

#![cfg(feature = "no-gc")]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cljrs_ir::{IrFunction, Inst, KnownFn, Terminator, VarId};

// ── Violation type ───────────────────────────────────────────────────────────

/// A no-gc memory-safety violation detected in an IR function.
#[derive(Debug, Clone)]
pub enum BlacklistViolation {
    /// The return variable was produced by an allocation instruction within
    /// the function body and would dangle after the scratch region is reset.
    InteriorPointerReturn {
        function: Option<Arc<str>>,
        var: VarId,
    },

    /// An allocation result flows directly into a `DefVar` or `SetBang`
    /// without being computed in the static context, so it would store a
    /// scratch-region pointer in a program-lifetime container.
    RegionToStaticStore {
        function: Option<Arc<str>>,
        var: VarId,
    },

    /// A lazy-sequence-producing call result was bound as an intermediate
    /// (used more than once) and is then returned unrealized.  The captured
    /// thunk references would dangle after the scratch region resets.
    LazySeqEscape {
        function: Option<Arc<str>>,
        var: VarId,
    },

    /// An `AllocClosure` that captures region-local values is stored in a
    /// static container (`DefVar` / `SetBang`).  Calling the closure later
    /// would access freed memory.
    EscapingClosure {
        function: Option<Arc<str>>,
        var: VarId,
    },
}

impl std::fmt::Display for BlacklistViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InteriorPointerReturn { function, var } => write!(
                f,
                "no-gc: function {fn_name} returns {var} which was allocated in its own \
                 scratch region — the return expression must produce a fresh value via a \
                 function call (e.g. use `(assoc ...)` rather than returning a let-bound map)",
                fn_name = fn_display(function),
            ),
            Self::RegionToStaticStore { function, var } => write!(
                f,
                "no-gc: function {fn_name} stores scratch-region allocation {var} in a \
                 static container (def / set!) — compute the value inside the sink so it \
                 is allocated in the static context",
                fn_name = fn_display(function),
            ),
            Self::LazySeqEscape { function, var } => write!(
                f,
                "no-gc: function {fn_name} lets unrealized lazy sequence {var} escape its \
                 scratch region via a non-tail binding — wrap with `doall` / `vec` / `into` \
                 before returning, or make the lazy producer the direct return expression",
                fn_name = fn_display(function),
            ),
            Self::EscapingClosure { function, var } => write!(
                f,
                "no-gc: function {fn_name} stores closure {var} (which captures \
                 region-local values) in a static container — ensure all captured values \
                 are computed in the static context before the closure is stored",
                fn_name = fn_display(function),
            ),
        }
    }
}

fn fn_display(name: &Option<Arc<str>>) -> String {
    match name {
        Some(n) => format!("`{n}`"),
        None => "<anonymous>".to_string(),
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// VarIds produced by allocation instructions (including region-backed ones).
fn collect_alloc_vars(func: &IrFunction) -> HashSet<VarId> {
    let mut allocs = HashSet::new();
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Some(dst) = alloc_dst(inst) {
                allocs.insert(dst);
            }
        }
    }
    allocs
}

fn alloc_dst(inst: &Inst) -> Option<VarId> {
    match inst {
        Inst::AllocVector(dst, _)
        | Inst::AllocMap(dst, _)
        | Inst::AllocSet(dst, _)
        | Inst::AllocList(dst, _) => Some(*dst),
        Inst::AllocCons(dst, _, _) => Some(*dst),
        Inst::AllocClosure(dst, _, _) => Some(*dst),
        Inst::RegionAlloc(dst, _, _, _) => Some(*dst),
        _ => None,
    }
}

/// Phi-node inputs: `phi_inputs[dst]` = list of source VarIds.
fn collect_phi_inputs(func: &IrFunction) -> HashMap<VarId, Vec<VarId>> {
    let mut phi_inputs: HashMap<VarId, Vec<VarId>> = HashMap::new();
    for block in &func.blocks {
        for inst in &block.phis {
            if let Inst::Phi(dst, entries) = inst {
                phi_inputs
                    .entry(*dst)
                    .or_default()
                    .extend(entries.iter().map(|(_, v)| *v));
            }
        }
    }
    phi_inputs
}

/// Returns `true` if `var` is (transitively through phi nodes) the result of
/// an allocation instruction.
fn is_alloc_derived(
    var: VarId,
    alloc_vars: &HashSet<VarId>,
    phi_inputs: &HashMap<VarId, Vec<VarId>>,
) -> bool {
    let mut visited = HashSet::new();
    let mut stack = vec![var];
    while let Some(v) = stack.pop() {
        if !visited.insert(v) {
            continue;
        }
        if alloc_vars.contains(&v) {
            return true;
        }
        if let Some(inputs) = phi_inputs.get(&v) {
            stack.extend_from_slice(inputs);
        }
    }
    false
}

/// Value operands of `DefVar` and `SetBang` instructions.
fn collect_static_sink_vals(func: &IrFunction) -> Vec<VarId> {
    let mut sinks = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            match inst {
                Inst::DefVar(_, _, _, val) => sinks.push(*val),
                Inst::SetBang(_, val) => sinks.push(*val),
                _ => {}
            }
        }
    }
    sinks
}

/// New-value operands of `AtomReset` / extra-arg operands of `AtomSwap`.
fn collect_atom_sink_vals(func: &IrFunction) -> Vec<VarId> {
    let mut sinks = Vec::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::CallKnown(_, kfn, args) = inst {
                match kfn {
                    KnownFn::AtomReset if args.len() >= 2 => sinks.push(args[1]),
                    KnownFn::AtomSwap if args.len() >= 3 => {
                        // args = [atom, f, extra1, extra2, ...]
                        // extra args are passed to f and their values end up
                        // in the atom; flag them.
                        sinks.extend_from_slice(&args[2..]);
                    }
                    _ => {}
                }
            }
        }
    }
    sinks
}

/// VarIds produced by lazy-sequence–producing known calls.
fn collect_lazy_vars(func: &IrFunction) -> HashSet<VarId> {
    let mut lazy = HashSet::new();
    for block in &func.blocks {
        for inst in &block.insts {
            if let Inst::CallKnown(dst, kfn, _) = inst {
                if is_lazy_producing(kfn) {
                    lazy.insert(*dst);
                }
            }
        }
    }
    lazy
}

fn is_lazy_producing(kfn: &KnownFn) -> bool {
    matches!(
        kfn,
        KnownFn::LazySeq
            | KnownFn::Map
            | KnownFn::Filter
            | KnownFn::Cons
            | KnownFn::Rest
            | KnownFn::Next
            | KnownFn::Concat
            | KnownFn::Range1
            | KnownFn::Range2
            | KnownFn::Range3
            | KnownFn::Take
            | KnownFn::Drop
            | KnownFn::Keep
            | KnownFn::Remove
    )
}

/// VarIds produced by `AllocClosure`.
fn collect_closure_vars(func: &IrFunction) -> HashSet<VarId> {
    let mut closures = HashSet::new();
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Inst::AllocClosure(dst, _, _) = inst {
                closures.insert(*dst);
            }
        }
    }
    closures
}

/// Count how many times `var` is referenced across all instructions and
/// terminators in the function.
fn count_uses(var: VarId, func: &IrFunction) -> usize {
    let mut count = 0;
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            count += uses_in_inst(var, inst);
        }
        count += uses_in_terminator(var, &block.terminator);
    }
    count
}

fn uses_in_inst(var: VarId, inst: &Inst) -> usize {
    match inst {
        Inst::CallKnown(_, _, args) => args.iter().filter(|&&a| a == var).count(),
        Inst::Call(_, callee, args) => {
            (if *callee == var { 1 } else { 0 }) + args.iter().filter(|&&a| a == var).count()
        }
        Inst::CallDirect(_, _, args) => args.iter().filter(|&&a| a == var).count(),
        Inst::AllocVector(_, elems) | Inst::AllocSet(_, elems) | Inst::AllocList(_, elems) => {
            elems.iter().filter(|&&e| e == var).count()
        }
        Inst::AllocMap(_, pairs) => pairs
            .iter()
            .filter(|(k, v)| *k == var || *v == var)
            .count(),
        Inst::AllocCons(_, head, tail) => {
            (if *head == var { 1 } else { 0 }) + (if *tail == var { 1 } else { 0 })
        }
        Inst::AllocClosure(_, _, captures) => captures.iter().filter(|&&c| c == var).count(),
        Inst::DefVar(_, _, _, val) => usize::from(*val == var),
        Inst::SetBang(_, val) => usize::from(*val == var),
        Inst::Phi(_, entries) => entries.iter().filter(|(_, v)| *v == var).count(),
        Inst::Deref(_, src) => usize::from(*src == var),
        Inst::Throw(v) => usize::from(*v == var),
        Inst::Recur(args) => args.iter().filter(|&&a| a == var).count(),
        Inst::RegionAlloc(_, region, _, operands) => {
            usize::from(*region == var) + operands.iter().filter(|&&o| o == var).count()
        }
        _ => 0,
    }
}

fn uses_in_terminator(var: VarId, term: &Terminator) -> usize {
    match term {
        Terminator::Return(v) => usize::from(*v == var),
        Terminator::Branch { cond, .. } => usize::from(*cond == var),
        Terminator::RecurJump { args, .. } => args.iter().filter(|&&a| a == var).count(),
        _ => 0,
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Run all four blacklist checks on a single IR function (not its subfunctions).
pub fn check_function(func: &IrFunction) -> Vec<BlacklistViolation> {
    let mut violations = Vec::new();

    let alloc_vars = collect_alloc_vars(func);
    let phi_inputs = collect_phi_inputs(func);
    let static_sink_vals = collect_static_sink_vals(func);
    let atom_sink_vals = collect_atom_sink_vals(func);
    let all_sink_vals: Vec<VarId> = static_sink_vals
        .iter()
        .chain(atom_sink_vals.iter())
        .copied()
        .collect();
    let lazy_vars = collect_lazy_vars(func);
    let closure_vars = collect_closure_vars(func);

    // ── Check 1: Interior-pointer return ─────────────────────────────────────
    //
    // Flag any return whose var is (transitively via phi) an allocation from
    // within this function body.  In the AOT-compiled no-gc path the value
    // would have been placed in the scratch region and dangles after reset.
    for block in &func.blocks {
        if let Terminator::Return(ret_var) = &block.terminator {
            if is_alloc_derived(*ret_var, &alloc_vars, &phi_inputs) {
                violations.push(BlacklistViolation::InteriorPointerReturn {
                    function: func.name.clone(),
                    var: *ret_var,
                });
            }
        }
    }

    // ── Check 2: Region → static store ───────────────────────────────────────
    //
    // Flag allocation vars that flow directly into DefVar / SetBang / AtomReset.
    // The interpreter handles this via StaticCtxGuard (Phase 5), but the AOT
    // codegen cannot inject a context switch around a pre-computed variable.
    for &sink_val in &all_sink_vals {
        if is_alloc_derived(sink_val, &alloc_vars, &phi_inputs) {
            violations.push(BlacklistViolation::RegionToStaticStore {
                function: func.name.clone(),
                var: sink_val,
            });
        }
    }

    // ── Check 3: Lazy-sequence escape ─────────────────────────────────────────
    //
    // A lazy-producing call result that is used more than once was bound as an
    // intermediate binding (not just the direct tail expression) and then
    // returned.  The thunk's captured pointers would dangle.
    for block in &func.blocks {
        if let Terminator::Return(ret_var) = &block.terminator {
            if lazy_vars.contains(ret_var) {
                // Use-count > 1 means the var was used somewhere else too
                // (e.g. printed, passed to another fn) before being returned,
                // indicating it was an intermediate binding.
                if count_uses(*ret_var, func) > 1 {
                    violations.push(BlacklistViolation::LazySeqEscape {
                        function: func.name.clone(),
                        var: *ret_var,
                    });
                }
            }
        }
    }

    // ── Check 4: Escaping closure ─────────────────────────────────────────────
    //
    // A closure stored in a static container may be called after the scratch
    // region that held its captured bindings has been reset.
    for &sink_val in &all_sink_vals {
        if closure_vars.contains(&sink_val) {
            violations.push(BlacklistViolation::EscapingClosure {
                function: func.name.clone(),
                var: sink_val,
            });
        }
    }

    violations
}

/// Run blacklist checks on `func` and all of its nested subfunctions.
pub fn check(func: &IrFunction) -> Vec<BlacklistViolation> {
    let mut v = check_function(func);
    for sub in &func.subfunctions {
        v.extend(check(sub));
    }
    v
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_ir::{Block, BlockId, ClosureTemplate, IrFunction, Terminator, VarId};

    fn make_func(name: &str, blocks: Vec<Block>) -> IrFunction {
        let max_var = blocks.iter().flat_map(|b| {
            b.phis.iter().chain(b.insts.iter()).filter_map(|i| match i {
                Inst::AllocVector(d, _) | Inst::AllocMap(d, _) | Inst::AllocSet(d, _)
                | Inst::AllocList(d, _) | Inst::AllocClosure(d, _, _) | Inst::Const(d, _)
                | Inst::LoadLocal(d, _) | Inst::LoadGlobal(d, _, _) | Inst::RegionAlloc(d, _, _, _)
                | Inst::DefVar(d, _, _, _) | Inst::CallKnown(d, _, _) | Inst::Call(d, _, _) => Some(d.0 + 1),
                Inst::AllocCons(d, _, _) => Some(d.0 + 1),
                _ => None,
            })
        }).max().unwrap_or(0);
        IrFunction {
            name: Some(Arc::from(name)),
            params: vec![],
            blocks,
            next_var: max_var,
            next_block: 1,
            span: None,
            subfunctions: vec![],
        }
    }

    fn simple_block(id: u32, insts: Vec<Inst>, term: Terminator) -> Block {
        Block { id: BlockId(id), phis: vec![], insts, terminator: term }
    }

    #[test]
    fn interior_pointer_return_flagged() {
        // %0 = alloc-map []; return %0  → error
        let v0 = VarId(0);
        let func = make_func(
            "bad",
            vec![simple_block(
                0,
                vec![Inst::AllocMap(v0, vec![])],
                Terminator::Return(v0),
            )],
        );
        let violations = check_function(&func);
        assert!(
            violations.iter().any(|v| matches!(v, BlacklistViolation::InteriorPointerReturn { .. })),
            "expected InteriorPointerReturn, got {violations:?}"
        );
    }

    #[test]
    fn call_return_not_flagged() {
        // %0 = load-global "m"; %1 = call-known assoc [%0]; return %1  → OK
        let v0 = VarId(0);
        let v1 = VarId(1);
        let func = make_func(
            "ok",
            vec![simple_block(
                0,
                vec![
                    Inst::LoadGlobal(v0, Arc::from("user"), Arc::from("m")),
                    Inst::CallKnown(v1, KnownFn::Assoc, vec![v0]),
                ],
                Terminator::Return(v1),
            )],
        );
        let violations = check_function(&func);
        assert!(
            !violations
                .iter()
                .any(|v| matches!(v, BlacklistViolation::InteriorPointerReturn { .. })),
            "expected no InteriorPointerReturn, got {violations:?}"
        );
    }

    #[test]
    fn region_to_static_store_flagged() {
        // %0 = alloc-map []; %1 = def-var ns/x %0  → error
        let v0 = VarId(0);
        let v1 = VarId(1);
        let func = make_func(
            "bad-def",
            vec![simple_block(
                0,
                vec![
                    Inst::AllocMap(v0, vec![]),
                    Inst::DefVar(v1, Arc::from("user"), Arc::from("x"), v0),
                ],
                Terminator::Return(v1),
            )],
        );
        let violations = check_function(&func);
        assert!(
            violations.iter().any(|v| matches!(v, BlacklistViolation::RegionToStaticStore { .. })),
            "expected RegionToStaticStore, got {violations:?}"
        );
    }

    #[test]
    fn lazy_seq_direct_return_not_flagged() {
        // %0 = call-known :map [...]; return %0  (used exactly once) → OK
        let v0 = VarId(0);
        let func = make_func(
            "ok-lazy",
            vec![simple_block(
                0,
                vec![Inst::CallKnown(v0, KnownFn::Map, vec![])],
                Terminator::Return(v0),
            )],
        );
        let violations = check_function(&func);
        assert!(
            !violations
                .iter()
                .any(|v| matches!(v, BlacklistViolation::LazySeqEscape { .. })),
            "direct lazy return should not be flagged, got {violations:?}"
        );
    }

    #[test]
    fn lazy_seq_intermediate_flagged() {
        // %0 = call-known :map []; %1 = call-known :println [%0]; return %0
        // → %0 used twice → LazySeqEscape
        let v0 = VarId(0);
        let v1 = VarId(1);
        let func = make_func(
            "bad-lazy",
            vec![simple_block(
                0,
                vec![
                    Inst::CallKnown(v0, KnownFn::Map, vec![]),
                    Inst::CallKnown(v1, KnownFn::Println, vec![v0]),
                ],
                Terminator::Return(v0),
            )],
        );
        let violations = check_function(&func);
        assert!(
            violations.iter().any(|v| matches!(v, BlacklistViolation::LazySeqEscape { .. })),
            "expected LazySeqEscape, got {violations:?}"
        );
    }

    #[test]
    fn escaping_closure_flagged() {
        // %0 = alloc-closure []; %1 = def-var ns/f %0  → EscapingClosure
        let v0 = VarId(0);
        let v1 = VarId(1);
        let tmpl = ClosureTemplate {
            name: None,
            arity_fn_names: vec![],
            param_counts: vec![],
            is_variadic: vec![],
            capture_names: vec![],
        };
        let func = make_func(
            "bad-closure",
            vec![simple_block(
                0,
                vec![
                    Inst::AllocClosure(v0, tmpl, vec![]),
                    Inst::DefVar(v1, Arc::from("user"), Arc::from("f"), v0),
                ],
                Terminator::Return(v1),
            )],
        );
        let violations = check_function(&func);
        assert!(
            violations.iter().any(|v| matches!(v, BlacklistViolation::EscapingClosure { .. })),
            "expected EscapingClosure, got {violations:?}"
        );
    }
}
