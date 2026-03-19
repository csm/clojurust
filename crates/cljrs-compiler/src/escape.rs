//! Escape analysis for IR functions.
//!
//! Determines which allocated values remain local to a function (non-escaping)
//! versus those that may be observed outside (escaping). Non-escaping values
//! can be:
//! - Allocated in a region instead of the GC heap
//! - Replaced with transient (mutable) operations when building collections
//! - Elided entirely if unused
//!
//! The analysis is conservative: if in doubt, a value is marked as escaping.

use std::collections::{HashMap, HashSet};

use crate::ir::*;

/// Escape classification for an allocated value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeState {
    /// Value does not escape the function — safe for region allocation
    /// or transient optimization.
    NoEscape,
    /// Value escapes only as an argument to a known function at the given
    /// position. Useful for interprocedural refinement.
    ArgEscape {
        callee: VarId,
        arg_index: usize,
    },
    /// Value escapes the function (returned, stored in heap, captured by
    /// closure, or passed to an unknown function).
    Escapes,
}

/// Result of escape analysis: maps each allocating VarId to its escape state.
#[derive(Debug)]
pub struct EscapeAnalysis {
    /// Escape state for each VarId that produces an allocation.
    pub states: HashMap<VarId, EscapeState>,
    /// Def-use chains: for each VarId, the set of instructions that use it.
    pub uses: HashMap<VarId, Vec<UseInfo>>,
}

/// Information about where a VarId is used.
#[derive(Debug, Clone)]
pub struct UseInfo {
    /// Which block contains the use.
    pub block: BlockId,
    /// What kind of use it is.
    pub kind: UseKind,
}

/// How a VarId is used.
#[derive(Debug, Clone)]
pub enum UseKind {
    /// Used as argument `arg_index` in a call to a known function.
    KnownCallArg { func: KnownFn, arg_index: usize },
    /// Used as argument `arg_index` in a call to an unknown function.
    UnknownCallArg { callee: VarId, arg_index: usize },
    /// Used as the callee of a call.
    CallCallee,
    /// Used in a return instruction.
    Return,
    /// Used in a DefVar (stored to global state).
    DefVar,
    /// Used in a SetBang (stored to var binding).
    SetBang,
    /// Used in an AllocClosure capture list.
    ClosureCapture,
    /// Used in an AllocCons, AllocVector, AllocMap, AllocSet (stored in heap object).
    StoredInHeap,
    /// Used as condition in a Branch.
    BranchCond,
    /// Used in a Phi node.
    PhiInput,
    /// Used in a Deref.
    Deref,
    /// Used in a Throw.
    Throw,
    /// Used in Recur args.
    Recur,
}

/// Run escape analysis on an IR function.
pub fn analyze(func: &IrFunction) -> EscapeAnalysis {
    let mut analysis = EscapeAnalysis {
        states: HashMap::new(),
        uses: HashMap::new(),
    };

    // Step 1: Identify all allocating instructions and build def-use chains.
    let allocs = collect_allocs(func);
    let uses = build_use_chains(func);
    analysis.uses = uses;

    // Step 2: For each allocation, determine escape state.
    for alloc_var in &allocs {
        let state = classify_escape(*alloc_var, &analysis.uses, func);
        analysis.states.insert(*alloc_var, state);
    }

    analysis
}

/// Collect all VarIds that are produced by allocation instructions.
fn collect_allocs(func: &IrFunction) -> Vec<VarId> {
    let mut allocs = Vec::new();
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if matches!(inst.effect(), Effect::Alloc)
                && let Some(dst) = inst.dst()
            {
                allocs.push(dst);
            }
        }
    }
    allocs
}

/// Build def-use chains: for each VarId, where it is used.
fn build_use_chains(func: &IrFunction) -> HashMap<VarId, Vec<UseInfo>> {
    let mut uses: HashMap<VarId, Vec<UseInfo>> = HashMap::new();

    for block in &func.blocks {
        // Process phis.
        for inst in &block.phis {
            if let Inst::Phi(_, entries) = inst {
                for (_, var) in entries {
                    uses.entry(*var).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::PhiInput,
                    });
                }
            }
        }

        // Process regular instructions.
        for inst in &block.insts {
            match inst {
                Inst::CallKnown(_, func_kind, args) => {
                    for (i, arg) in args.iter().enumerate() {
                        uses.entry(*arg).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::KnownCallArg {
                                func: func_kind.clone(),
                                arg_index: i,
                            },
                        });
                    }
                }
                Inst::Call(_, callee, args) => {
                    uses.entry(*callee).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::CallCallee,
                    });
                    for (i, arg) in args.iter().enumerate() {
                        uses.entry(*arg).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::UnknownCallArg {
                                callee: *callee,
                                arg_index: i,
                            },
                        });
                    }
                }
                Inst::AllocClosure(_, _, captures) => {
                    for cap in captures {
                        uses.entry(*cap).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::ClosureCapture,
                        });
                    }
                }
                Inst::AllocVector(_, elems) | Inst::AllocSet(_, elems) | Inst::AllocList(_, elems) => {
                    for elem in elems {
                        uses.entry(*elem).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::StoredInHeap,
                        });
                    }
                }
                Inst::AllocMap(_, pairs) => {
                    for (k, v) in pairs {
                        uses.entry(*k).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::StoredInHeap,
                        });
                        uses.entry(*v).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::StoredInHeap,
                        });
                    }
                }
                Inst::AllocCons(_, h, t) => {
                    uses.entry(*h).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::StoredInHeap,
                    });
                    uses.entry(*t).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::StoredInHeap,
                    });
                }
                Inst::DefVar(_, _, _, val) => {
                    uses.entry(*val).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::DefVar,
                    });
                }
                Inst::SetBang(var, val) => {
                    uses.entry(*var).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::SetBang,
                    });
                    uses.entry(*val).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::SetBang,
                    });
                }
                Inst::Deref(_, src) => {
                    uses.entry(*src).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::Deref,
                    });
                }
                Inst::Throw(val) => {
                    uses.entry(*val).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::Throw,
                    });
                }
                Inst::Recur(args) => {
                    for arg in args {
                        uses.entry(*arg).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::Recur,
                        });
                    }
                }
                Inst::RegionAlloc(_, _, _, operands) => {
                    for op in operands {
                        uses.entry(*op).or_default().push(UseInfo {
                            block: block.id,
                            kind: UseKind::StoredInHeap, // same semantics
                        });
                    }
                }
                Inst::Const(..) | Inst::LoadLocal(..) | Inst::LoadGlobal(..)
                | Inst::SourceLoc(..) | Inst::Phi(..)
                | Inst::RegionStart(..) | Inst::RegionEnd(..) => {}
            }
        }

        // Process terminator.
        match &block.terminator {
            Terminator::Return(var) => {
                uses.entry(*var).or_default().push(UseInfo {
                    block: block.id,
                    kind: UseKind::Return,
                });
            }
            Terminator::Branch { cond, .. } => {
                uses.entry(*cond).or_default().push(UseInfo {
                    block: block.id,
                    kind: UseKind::BranchCond,
                });
            }
            Terminator::RecurJump { args, .. } => {
                for arg in args {
                    uses.entry(*arg).or_default().push(UseInfo {
                        block: block.id,
                        kind: UseKind::Recur,
                    });
                }
            }
            Terminator::Jump(_) | Terminator::Unreachable => {}
        }
    }

    uses
}

/// Classify whether an allocation escapes, considering transitive flow through
/// phi nodes and known function calls.
fn classify_escape(
    var: VarId,
    uses: &HashMap<VarId, Vec<UseInfo>>,
    func: &IrFunction,
) -> EscapeState {
    // Track which VarIds we've visited (handles phi cycles).
    let mut visited = HashSet::new();
    let mut worklist = vec![var];
    let mut result = EscapeState::NoEscape;

    while let Some(current) = worklist.pop() {
        if !visited.insert(current) {
            continue;
        }

        let Some(use_list) = uses.get(&current) else {
            continue;
        };

        for use_info in use_list {
            match &use_info.kind {
                // These always cause escape.
                UseKind::Return
                | UseKind::DefVar
                | UseKind::SetBang
                | UseKind::ClosureCapture
                | UseKind::Throw => {
                    return EscapeState::Escapes;
                }

                // Stored inside a heap object — the value escapes if the
                // containing object escapes. But conservatively, mark as escaping.
                UseKind::StoredInHeap => {
                    return EscapeState::Escapes;
                }

                // Unknown call — value escapes.
                UseKind::UnknownCallArg { callee, arg_index } => {
                    // If this is the only escape path, record it.
                    if result == EscapeState::NoEscape {
                        result = EscapeState::ArgEscape {
                            callee: *callee,
                            arg_index: *arg_index,
                        };
                    } else {
                        return EscapeState::Escapes;
                    }
                }

                // Known call — check if the function is known to not let
                // the argument escape.
                UseKind::KnownCallArg { func: known, arg_index } => {
                    if known_fn_arg_escapes(known, *arg_index) {
                        // The argument flows into the return value of the known call.
                        // Check if the result of this call escapes.
                        // Find the CallKnown instruction that uses our var and track
                        // its result.
                        if let Some(call_result) = find_call_result_for_use(
                            current,
                            known,
                            func,
                            use_info.block,
                        ) {
                            worklist.push(call_result);
                        } else {
                            return EscapeState::Escapes;
                        }
                    }
                    // Otherwise, the known function doesn't let this arg escape
                    // (e.g., `count`, `get` where the collection arg is only read).
                }

                // Phi: the value flows to the phi result. Track the phi output.
                UseKind::PhiInput => {
                    // Find the phi instruction in this block that uses `current`.
                    for block in &func.blocks {
                        if block.id == use_info.block {
                            for phi in &block.phis {
                                if let Inst::Phi(dst, entries) = phi
                                    && entries.iter().any(|(_, v)| *v == current)
                                {
                                    worklist.push(*dst);
                                }
                            }
                        }
                    }
                }

                // Recur: the value flows back to loop header phis.
                // This is handled through phi tracking above.
                UseKind::Recur => {
                    // Recur args go to loop phis — if the phi result escapes,
                    // so does this value. But we don't have the recur→phi mapping
                    // yet, so conservatively mark as escaping.
                    // TODO: track recur→phi mapping for better precision.
                    return EscapeState::Escapes;
                }

                // These uses don't cause the value to escape.
                UseKind::BranchCond | UseKind::Deref | UseKind::CallCallee => {}
            }
        }
    }

    result
}

/// Does a known function allow argument at `arg_index` to escape into
/// its return value?
///
/// Returns `true` if the argument may be part of the result (e.g., `assoc`
/// returns a collection containing the key and value arguments).
/// Returns `false` if the argument is only read (e.g., `count`, `get`).
fn known_fn_arg_escapes(func: &KnownFn, arg_index: usize) -> bool {
    match func {
        // Collection constructors: all args are stored in the result.
        KnownFn::Vector | KnownFn::HashMap | KnownFn::HashSet | KnownFn::List => true,

        // Assoc: arg 0 (collection) becomes part of result, args 1+ are stored.
        KnownFn::Assoc => true,

        // Conj: arg 0 (collection) becomes part of result, arg 1 is stored.
        KnownFn::Conj => true,

        // Dissoc/Disj: arg 0 (collection) becomes part of result.
        KnownFn::Dissoc | KnownFn::Disj => arg_index == 0,

        // Cons: both head and tail are stored in the result.
        KnownFn::Cons => true,

        // These return a sub-part of the collection — the collection itself
        // doesn't escape, but the returned element might.
        KnownFn::Get | KnownFn::Nth | KnownFn::First => false,

        // Rest/Next/Seq return a derived sequence from arg 0.
        KnownFn::Rest | KnownFn::Next | KnownFn::Seq => arg_index == 0,

        // LazySeq: the thunk (arg 0) is captured.
        KnownFn::LazySeq => true,

        // Pure operations that return scalars — args don't escape.
        KnownFn::Count
        | KnownFn::Contains
        | KnownFn::Add
        | KnownFn::Sub
        | KnownFn::Mul
        | KnownFn::Div
        | KnownFn::Rem
        | KnownFn::Eq
        | KnownFn::Lt
        | KnownFn::Gt
        | KnownFn::Lte
        | KnownFn::Gte
        | KnownFn::IsNil
        | KnownFn::IsSeq
        | KnownFn::IsVector
        | KnownFn::IsMap
        | KnownFn::Identical => false,

        // Str: args are read to produce a new string.
        KnownFn::Str => false,

        // Deref reads from a ref.
        KnownFn::Deref | KnownFn::AtomDeref => false,

        // Atom operations: swap/reset store the value in the atom (escapes to heap).
        KnownFn::AtomReset | KnownFn::AtomSwap => true,

        // Transient: arg 0 (collection) is consumed and wrapped.
        KnownFn::Transient => arg_index == 0,

        // Transient mutation: result is the same transient.
        KnownFn::AssocBang | KnownFn::ConjBang => arg_index == 0,

        // Persistent!: arg 0 (transient) is consumed.
        KnownFn::PersistentBang => arg_index == 0,

        // I/O: args are read, not stored.
        KnownFn::Println | KnownFn::Pr => false,

        // Apply: unknown — all args may escape.
        KnownFn::Apply => true,
    }
}

/// Find the result VarId of a CallKnown instruction in a block that uses
/// `used_var` as an argument to `known_fn`.
fn find_call_result_for_use(
    used_var: VarId,
    known_fn: &KnownFn,
    func: &IrFunction,
    block_id: BlockId,
) -> Option<VarId> {
    for block in &func.blocks {
        if block.id != block_id {
            continue;
        }
        for inst in &block.insts {
            if let Inst::CallKnown(dst, f, args) = inst
                && f == known_fn
                && args.contains(&used_var)
            {
                return Some(*dst);
            }
        }
    }
    None
}

/// Build a map from VarId to the instruction that defines it.
#[allow(dead_code)]
fn build_def_map(func: &IrFunction) -> HashMap<VarId, &Inst> {
    let mut defs = HashMap::new();
    for block in &func.blocks {
        for inst in block.phis.iter().chain(block.insts.iter()) {
            if let Some(dst) = inst.dst() {
                defs.insert(dst, inst);
            }
        }
    }
    defs
}

// ── Assoc-chain detection ────────────────────────────────────────────────────

/// An assoc/conj chain: a sequence of collection-building operations where
/// intermediate values don't escape.
#[derive(Debug, Clone)]
pub struct CollectionChain {
    /// The VarId of the initial collection (input to the first operation).
    pub root: VarId,
    /// Sequence of operations in the chain.
    pub ops: Vec<ChainOp>,
    /// The VarId of the final result.
    pub result: VarId,
}

/// A single operation in a collection chain.
#[derive(Debug, Clone)]
pub struct ChainOp {
    /// The known function (Assoc, Conj, etc.)
    pub func: KnownFn,
    /// The result VarId of this operation.
    pub result: VarId,
    /// The argument VarIds (excluding the collection, which is chained).
    pub args: Vec<VarId>,
}

/// Detect assoc/conj chains in an IR function where intermediate collections
/// don't escape.
///
/// Returns chains that can be optimized to use transient operations.
pub fn detect_collection_chains(
    func: &IrFunction,
    escape: &EscapeAnalysis,
) -> Vec<CollectionChain> {
    let mut chains = Vec::new();
    let uses = &escape.uses;

    // Forward-walk approach: for each block, find sequences of
    // assoc/conj calls that form chains. Non-chainable instructions
    // (like Const) between chain ops are fine — they don't break the chain.
    for block in &func.blocks {
        let mut current_chain: Option<CollectionChain> = None;

        for inst in &block.insts {
            if let Inst::CallKnown(dst, func_kind, args) = inst {
                if !is_chainable_op(func_kind) || args.is_empty() {
                    continue; // Skip non-chainable ops without flushing.
                }

                let collection_arg = args[0];
                let other_args: Vec<VarId> = args[1..].to_vec();

                if let Some(ref mut chain) = current_chain {
                    if chain.result == collection_arg {
                        // Extends the current chain.
                        // Check that the intermediate (chain.result) is only
                        // used as the collection arg of this call.
                        let intermediate_uses = uses.get(&chain.result);
                        let single_use = intermediate_uses
                            .map(|u| u.len() == 1)
                            .unwrap_or(true);

                        if single_use {
                            chain.ops.push(ChainOp {
                                func: func_kind.clone(),
                                result: *dst,
                                args: other_args,
                            });
                            chain.result = *dst;
                            continue;
                        }
                    }
                    // Can't extend — flush and maybe start new.
                    let old_chain = current_chain.take().unwrap();
                    if old_chain.ops.len() >= 2 {
                        chains.push(old_chain);
                    }
                }

                // Start a new chain.
                current_chain = Some(CollectionChain {
                    root: collection_arg,
                    ops: vec![ChainOp {
                        func: func_kind.clone(),
                        result: *dst,
                        args: other_args,
                    }],
                    result: *dst,
                });
            }
            // Non-chainable instructions don't flush — the chain continues
            // as long as the next chainable op uses the chain's result.
        }

        // Flush at block end.
        if let Some(chain) = current_chain.take()
            && chain.ops.len() >= 2
        {
            chains.push(chain);
        }
    }

    chains
}

/// Is this known function a chainable collection operation?
fn is_chainable_op(func: &KnownFn) -> bool {
    matches!(
        func,
        KnownFn::Assoc | KnownFn::Conj | KnownFn::Dissoc | KnownFn::Disj
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anf::lower_fn_body;
    use cljrs_reader::Parser;
    use cljrs_reader::form::FormKind;
    use std::sync::Arc;

    fn parse(src: &str) -> cljrs_reader::Form {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        parser.parse_one().unwrap().unwrap()
    }

    fn lower_and_analyze(src: &str) -> (IrFunction, EscapeAnalysis) {
        let form = parse(src);
        let (params, body) = match &form.kind {
            FormKind::List(forms) => {
                let params: Vec<Arc<str>> = match &forms[1].kind {
                    FormKind::Vector(v) => v
                        .iter()
                        .map(|f| match &f.kind {
                            FormKind::Symbol(s) => Arc::from(s.as_str()),
                            _ => panic!("expected symbol"),
                        })
                        .collect(),
                    _ => panic!("expected vector"),
                };
                (params, forms[2..].to_vec())
            }
            _ => panic!("expected list"),
        };
        let ir = lower_fn_body(None, "user", &params, &body).unwrap();
        let escape = analyze(&ir);
        (ir, escape)
    }

    #[test]
    fn test_returned_value_escapes() {
        // (fn [x] (assoc x :a 1)) — result is returned, so it escapes.
        let (ir, escape) = lower_and_analyze("(fn [x] (assoc x :a 1))");
        // Find the assoc result VarId.
        let assoc_dst = ir.blocks.iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match i {
                Inst::CallKnown(dst, KnownFn::Assoc, _) => Some(*dst),
                _ => None,
            })
            .expect("expected assoc call");
        // It should escape because it's returned.
        assert_eq!(escape.states.get(&assoc_dst), Some(&EscapeState::Escapes));
    }

    #[test]
    fn test_intermediate_does_not_escape() {
        // (fn [m] (let [a (assoc m :x 1)] (count a)))
        // `a` is only used by `count` which doesn't let it escape.
        let (ir, escape) = lower_and_analyze("(fn [m] (let [a (assoc m :x 1)] (count a)))");
        let assoc_dst = ir.blocks.iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match i {
                Inst::CallKnown(dst, KnownFn::Assoc, _) => Some(*dst),
                _ => None,
            })
            .expect("expected assoc call");
        assert_eq!(
            escape.states.get(&assoc_dst),
            Some(&EscapeState::NoEscape),
            "intermediate assoc result should not escape"
        );
    }

    #[test]
    fn test_assoc_chain_detection() {
        // (fn [m] (let [a (assoc m :x 1) b (assoc a :y 2) c (assoc b :z 3)] c))
        // a and b are intermediates; c escapes (returned).
        // Should detect a chain of length 3.
        let (ir, escape) = lower_and_analyze(
            "(fn [m] (let [a (assoc m :x 1) b (assoc a :y 2) c (assoc b :z 3)] c))",
        );
        let chains = detect_collection_chains(&ir, &escape);
        assert!(
            !chains.is_empty(),
            "expected to detect an assoc chain"
        );
        assert!(
            chains[0].ops.len() >= 2,
            "expected chain of at least 2 operations, got {}",
            chains[0].ops.len()
        );
    }

    #[test]
    fn test_closure_capture_escapes() {
        // (fn [x] (let [v (vector 1 2 3)] (fn [] v)))
        // v is captured by a closure, so it escapes.
        let (ir, escape) = lower_and_analyze("(fn [x] (let [v [1 2 3]] (fn [] v)))");
        let alloc_dst = ir.blocks.iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match i {
                Inst::AllocVector(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("expected vector alloc");
        assert_eq!(
            escape.states.get(&alloc_dst),
            Some(&EscapeState::Escapes),
            "closure-captured value should escape"
        );
    }

    #[test]
    fn test_unused_alloc_no_escape() {
        // (fn [] (let [v [1 2 3]] 42))
        // v is allocated but never used — should not escape.
        let (ir, escape) = lower_and_analyze("(fn [] (let [v [1 2 3]] 42))");
        let alloc_dst = ir.blocks.iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match i {
                Inst::AllocVector(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("expected vector alloc");
        assert_eq!(
            escape.states.get(&alloc_dst),
            Some(&EscapeState::NoEscape),
            "unused allocation should not escape"
        );
    }

    #[test]
    fn test_def_causes_escape() {
        // (fn [] (def foo [1 2 3]))
        // The vector escapes via def (stored globally).
        let (ir, escape) = lower_and_analyze("(fn [] (def foo [1 2 3]))");
        let alloc_dst = ir.blocks.iter()
            .flat_map(|b| b.insts.iter())
            .find_map(|i| match i {
                Inst::AllocVector(dst, _) => Some(*dst),
                _ => None,
            })
            .expect("expected vector alloc");
        assert_eq!(
            escape.states.get(&alloc_dst),
            Some(&EscapeState::Escapes),
            "def'd value should escape"
        );
    }
}
