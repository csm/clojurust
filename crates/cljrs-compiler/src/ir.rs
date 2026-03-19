//! Intermediate representation for clojurust program analysis and optimization.
//!
//! The IR is a control-flow graph of basic blocks containing instructions in
//! A-normal form (all sub-expressions bound to named temporaries). It supports
//! SSA construction via phi nodes at join points.
//!
//! The IR serves two purposes:
//! 1. **Now**: escape analysis and optimization hints for the interpreter
//! 2. **Future**: input to Cranelift-based JIT/AOT code generation (Phase 10/11)

use std::fmt;
use std::sync::Arc;

use cljrs_reader::Form;
use cljrs_types::span::Span;

// ── Variable IDs ─────────────────────────────────────────────────────────────

/// A unique variable identifier within an IR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VarId(pub u32);

impl fmt::Display for VarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A basic block identifier within an IR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

impl fmt::Display for BlockId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

// ── Known functions ──────────────────────────────────────────────────────────

/// Built-in functions the IR knows about for precise effect tracking.
///
/// When the IR can identify a call target as a known function, it uses this
/// enum instead of a generic `Call` — enabling escape analysis to reason
/// precisely about argument flow and allocation behavior.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnownFn {
    // Collection constructors
    Vector,
    HashMap,
    HashSet,
    List,

    // Collection operations (return new persistent collection)
    Assoc,
    Dissoc,
    Conj,
    Disj,
    Get,
    Nth,
    Count,
    Contains,

    // Transient operations
    Transient,
    AssocBang,
    ConjBang,
    PersistentBang,

    // Sequence operations
    First,
    Rest,
    Next,
    Cons,
    Seq,
    LazySeq,

    // Arithmetic (pure, no alloc for i64/f64)
    Add,
    Sub,
    Mul,
    Div,
    Rem,

    // Comparison (pure)
    Eq,
    Lt,
    Gt,
    Lte,
    Gte,

    // Type checks (pure)
    IsNil,
    IsSeq,
    IsVector,
    IsMap,

    // String
    Str,

    // Identity / deref
    Deref,
    Identical,

    // I/O and side effects
    Println,
    Pr,

    // Atom operations
    AtomDeref,
    AtomReset,
    AtomSwap,

    // Apply
    Apply,
}

// ── Effect metadata ──────────────────────────────────────────────────────────

/// Effect classification for IR instructions.
///
/// Used by escape analysis and optimization passes to reason about what
/// side effects an instruction may have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// No observable side effects; result depends only on inputs.
    Pure,
    /// Allocates a new heap object (GC or region).
    Alloc,
    /// Reads from a heap object (may observe mutations).
    HeapRead,
    /// Writes to a heap object (atoms, volatiles, vars).
    HeapWrite,
    /// Performs I/O.
    IO,
    /// Calls an unknown function — must assume any effect.
    UnknownCall,
}

// ── Constant values ──────────────────────────────────────────────────────────

/// A constant value in the IR. Kept separate from `Value` to avoid requiring
/// GC allocation for IR analysis.
#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Nil,
    Bool(bool),
    Long(i64),
    Double(f64),
    Str(Arc<str>),
    Keyword(Arc<str>),
    Symbol(Arc<str>),
    Char(char),
}

// ── Instructions ─────────────────────────────────────────────────────────────

/// An IR instruction. Each instruction produces at most one result (the `dst`
/// field in variants that have one).
///
/// Instructions are in A-normal form: all operands are `VarId` references to
/// previously computed values, never nested expressions.
#[derive(Debug, Clone)]
pub enum Inst {
    /// Load a constant value.
    Const(VarId, Const),

    /// Load a local variable by name (from the interpreter's Env).
    LoadLocal(VarId, Arc<str>),

    /// Load a global var by namespace-qualified name.
    LoadGlobal(VarId, Arc<str>, Arc<str>), // dst, ns, name

    /// Allocate a vector from elements.
    AllocVector(VarId, Vec<VarId>),

    /// Allocate a map from key-value pairs.
    AllocMap(VarId, Vec<(VarId, VarId)>),

    /// Allocate a set from elements.
    AllocSet(VarId, Vec<VarId>),

    /// Allocate a list from elements.
    AllocList(VarId, Vec<VarId>),

    /// Allocate a cons cell.
    AllocCons(VarId, VarId, VarId), // dst, head, tail

    /// Allocate a closure, capturing the given variables.
    AllocClosure(VarId, ClosureTemplate, Vec<VarId>),

    /// Call a known built-in function.
    CallKnown(VarId, KnownFn, Vec<VarId>),

    /// Call an unknown function value.
    Call(VarId, VarId, Vec<VarId>), // dst, callee, args

    /// Dereference (@ operator).
    Deref(VarId, VarId),

    /// Store to a var's root binding (`def`).
    DefVar(VarId, Arc<str>, Arc<str>, VarId), // dst(=var), ns, name, value

    /// `set!` on a var.
    SetBang(VarId, VarId), // var, value

    /// Throw an exception.
    Throw(VarId),

    /// SSA phi node — value depends on which predecessor block we came from.
    Phi(VarId, Vec<(BlockId, VarId)>),

    /// Recur with new values (in a loop context).
    Recur(Vec<VarId>),

    /// No-op marker with a source span (for debugging / source mapping).
    SourceLoc(Span),

    // ── Region allocation nodes ─────────────────────────────────────────

    /// Begin a region scope.  The `VarId` identifies the region handle,
    /// used by subsequent `RegionAlloc` instructions.  Paired with
    /// `RegionEnd`.
    RegionStart(VarId),

    /// Allocate an object in a region instead of the GC heap.
    /// `(dst, region_handle, alloc_kind, operands)`.
    ///
    /// `alloc_kind` mirrors the collection `Alloc*` instructions but
    /// produces region-backed `GcPtr`s.
    RegionAlloc(VarId, VarId, RegionAllocKind, Vec<VarId>),

    /// End a region scope — all region-allocated objects are freed.
    /// The `VarId` is the region handle from `RegionStart`.
    RegionEnd(VarId),
}

/// The kind of object allocated in a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionAllocKind {
    /// `[elem ...]` — vector from elements.
    Vector,
    /// `{k v ...}` — map from key-value pairs.
    Map,
    /// `#{elem ...}` — set from elements.
    Set,
    /// `(elem ...)` — list from elements.
    List,
    /// `(cons head tail)` — cons cell.
    Cons,
}

impl fmt::Display for RegionAllocKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vector => write!(f, "vector"),
            Self::Map => write!(f, "map"),
            Self::Set => write!(f, "set"),
            Self::List => write!(f, "list"),
            Self::Cons => write!(f, "cons"),
        }
    }
}

/// Template for a closure — the static parts of an `fn*` form.
#[derive(Debug, Clone)]
pub struct ClosureTemplate {
    /// Function name (if named).
    pub name: Option<Arc<str>>,
    /// The original `fn*` body forms, kept for the interpreter.
    /// In the future, each arity would have its own `IrFunction`.
    pub body_forms: Vec<Form>,
}

// ── Terminators ──────────────────────────────────────────────────────────────

/// A block terminator — controls flow between basic blocks.
#[derive(Debug, Clone)]
pub enum Terminator {
    /// Unconditional jump.
    Jump(BlockId),

    /// Conditional branch.
    Branch {
        cond: VarId,
        then_block: BlockId,
        else_block: BlockId,
    },

    /// Return a value from the function.
    Return(VarId),

    /// Recur (tail-call back to loop header).
    RecurJump {
        target: BlockId,
        args: Vec<VarId>,
    },

    /// Unreachable (e.g., after a `throw`).
    Unreachable,
}

// ── Basic blocks and functions ───────────────────────────────────────────────

/// A basic block: a linear sequence of instructions followed by a terminator.
#[derive(Debug, Clone)]
pub struct Block {
    pub id: BlockId,
    /// Phi nodes at the top of this block (only at join points).
    pub phis: Vec<Inst>,
    /// Non-phi instructions, in order.
    pub insts: Vec<Inst>,
    /// How this block transfers control.
    pub terminator: Terminator,
}

/// An IR function — the unit of analysis.
#[derive(Debug)]
pub struct IrFunction {
    /// Function name (for diagnostics).
    pub name: Option<Arc<str>>,
    /// Parameters (mapped to VarIds).
    pub params: Vec<(Arc<str>, VarId)>,
    /// All basic blocks. `blocks[0]` is the entry block.
    pub blocks: Vec<Block>,
    /// Next VarId to allocate.
    pub next_var: u32,
    /// Next BlockId to allocate.
    pub next_block: u32,
    /// Source span of the original function definition.
    pub span: Option<Span>,
}

impl IrFunction {
    /// Create a new empty IR function.
    pub fn new(name: Option<Arc<str>>, span: Option<Span>) -> Self {
        Self {
            name,
            params: Vec::new(),
            blocks: Vec::new(),
            next_var: 0,
            next_block: 0,
            span,
        }
    }

    /// Allocate a fresh variable ID.
    pub fn fresh_var(&mut self) -> VarId {
        let id = VarId(self.next_var);
        self.next_var += 1;
        id
    }

    /// Allocate a fresh block ID.
    pub fn fresh_block(&mut self) -> BlockId {
        let id = BlockId(self.next_block);
        self.next_block += 1;
        id
    }
}

// ── Effect classification ────────────────────────────────────────────────────

impl Inst {
    /// Return the primary effect of this instruction.
    pub fn effect(&self) -> Effect {
        match self {
            Inst::Const(..) | Inst::LoadLocal(..) | Inst::Phi(..) | Inst::SourceLoc(..) => {
                Effect::Pure
            }
            Inst::LoadGlobal(..) => Effect::HeapRead,
            Inst::AllocVector(..)
            | Inst::AllocMap(..)
            | Inst::AllocSet(..)
            | Inst::AllocList(..)
            | Inst::AllocCons(..)
            | Inst::AllocClosure(..) => Effect::Alloc,
            Inst::CallKnown(_, known, _) => known.effect(),
            Inst::Call(..) => Effect::UnknownCall,
            Inst::Deref(..) => Effect::HeapRead,
            Inst::DefVar(..) => Effect::HeapWrite,
            Inst::SetBang(..) => Effect::HeapWrite,
            Inst::Throw(..) => Effect::UnknownCall, // conservative
            Inst::Recur(..) => Effect::Pure,
            Inst::RegionStart(..) | Inst::RegionEnd(..) => Effect::Alloc,
            Inst::RegionAlloc(..) => Effect::Alloc,
        }
    }

    /// Return the destination VarId, if this instruction produces one.
    pub fn dst(&self) -> Option<VarId> {
        match self {
            Inst::Const(v, _)
            | Inst::LoadLocal(v, _)
            | Inst::LoadGlobal(v, _, _)
            | Inst::AllocVector(v, _)
            | Inst::AllocMap(v, _)
            | Inst::AllocSet(v, _)
            | Inst::AllocList(v, _)
            | Inst::AllocCons(v, _, _)
            | Inst::AllocClosure(v, _, _)
            | Inst::CallKnown(v, _, _)
            | Inst::Call(v, _, _)
            | Inst::Deref(v, _)
            | Inst::DefVar(v, _, _, _)
            | Inst::Phi(v, _)
            | Inst::RegionStart(v)
            | Inst::RegionAlloc(v, _, _, _) => Some(*v),
            Inst::SetBang(..)
            | Inst::Throw(..)
            | Inst::Recur(..)
            | Inst::SourceLoc(..)
            | Inst::RegionEnd(..) => None,
        }
    }

    /// Return all VarIds used (read) by this instruction.
    pub fn uses(&self) -> Vec<VarId> {
        match self {
            Inst::Const(..) | Inst::LoadLocal(..) | Inst::LoadGlobal(..) | Inst::SourceLoc(..) => {
                vec![]
            }
            Inst::AllocVector(_, elems) | Inst::AllocSet(_, elems) | Inst::AllocList(_, elems) => {
                elems.clone()
            }
            Inst::AllocMap(_, pairs) => pairs.iter().flat_map(|(k, v)| [*k, *v]).collect(),
            Inst::AllocCons(_, h, t) => vec![*h, *t],
            Inst::AllocClosure(_, _, captures) => captures.clone(),
            Inst::CallKnown(_, _, args) => args.clone(),
            Inst::Call(_, callee, args) => {
                let mut v = vec![*callee];
                v.extend(args);
                v
            }
            Inst::Deref(_, src) => vec![*src],
            Inst::DefVar(_, _, _, val) => vec![*val],
            Inst::SetBang(var, val) => vec![*var, *val],
            Inst::Throw(val) => vec![*val],
            Inst::Phi(_, entries) => entries.iter().map(|(_, v)| *v).collect(),
            Inst::Recur(args) => args.clone(),
            Inst::RegionStart(..) => vec![],
            Inst::RegionAlloc(_, region, _, operands) => {
                let mut v = vec![*region];
                v.extend(operands);
                v
            }
            Inst::RegionEnd(region) => vec![*region],
        }
    }
}

impl KnownFn {
    /// Return the effect of calling this known function.
    pub fn effect(&self) -> Effect {
        match self {
            // Pure functions — no side effects, no allocation (result is scalar or reuses input)
            KnownFn::Get
            | KnownFn::Nth
            | KnownFn::Count
            | KnownFn::Contains
            | KnownFn::First
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
            | KnownFn::Identical => Effect::Pure,

            // Allocating — return a new persistent collection
            KnownFn::Vector
            | KnownFn::HashMap
            | KnownFn::HashSet
            | KnownFn::List
            | KnownFn::Assoc
            | KnownFn::Dissoc
            | KnownFn::Conj
            | KnownFn::Disj
            | KnownFn::Cons
            | KnownFn::Rest
            | KnownFn::Next
            | KnownFn::Seq
            | KnownFn::LazySeq
            | KnownFn::Str
            | KnownFn::Transient
            | KnownFn::PersistentBang => Effect::Alloc,

            // Transient mutation — heap write on the transient, but doesn't escape
            KnownFn::AssocBang | KnownFn::ConjBang => Effect::HeapWrite,

            // Deref reads from heap
            KnownFn::Deref | KnownFn::AtomDeref => Effect::HeapRead,

            // Atom mutation
            KnownFn::AtomReset | KnownFn::AtomSwap => Effect::HeapWrite,

            // I/O
            KnownFn::Println | KnownFn::Pr => Effect::IO,

            // Apply calls an unknown function
            KnownFn::Apply => Effect::UnknownCall,
        }
    }
}

// ── Display for debugging ────────────────────────────────────────────────────

impl fmt::Display for IrFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "fn {}({}):",
            self.name.as_deref().unwrap_or("<anon>"),
            self.params
                .iter()
                .map(|(name, id)| format!("{name}: {id}"))
                .collect::<Vec<_>>()
                .join(", ")
        )?;
        for block in &self.blocks {
            writeln!(f, "  {block}")?;
        }
        Ok(())
    }
}

impl fmt::Display for Block {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}:", self.id)?;
        for phi in &self.phis {
            writeln!(f, "    {phi}")?;
        }
        for inst in &self.insts {
            writeln!(f, "    {inst}")?;
        }
        write!(f, "    {}", self.terminator)
    }
}

impl fmt::Display for Inst {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Inst::Const(dst, c) => write!(f, "{dst} = const {c:?}"),
            Inst::LoadLocal(dst, name) => write!(f, "{dst} = load_local {name:?}"),
            Inst::LoadGlobal(dst, ns, name) => write!(f, "{dst} = load_global {ns}/{name}"),
            Inst::AllocVector(dst, elems) => write!(f, "{dst} = alloc_vec {elems:?}"),
            Inst::AllocMap(dst, pairs) => write!(f, "{dst} = alloc_map {pairs:?}"),
            Inst::AllocSet(dst, elems) => write!(f, "{dst} = alloc_set {elems:?}"),
            Inst::AllocList(dst, elems) => write!(f, "{dst} = alloc_list {elems:?}"),
            Inst::AllocCons(dst, h, t) => write!(f, "{dst} = cons {h} {t}"),
            Inst::AllocClosure(dst, tmpl, captures) => {
                write!(
                    f,
                    "{dst} = closure {:?} captures={captures:?}",
                    tmpl.name
                )
            }
            Inst::CallKnown(dst, func, args) => write!(f, "{dst} = call_known {func:?} {args:?}"),
            Inst::Call(dst, callee, args) => write!(f, "{dst} = call {callee} {args:?}"),
            Inst::Deref(dst, src) => write!(f, "{dst} = deref {src}"),
            Inst::DefVar(dst, ns, name, val) => write!(f, "{dst} = def {ns}/{name} {val}"),
            Inst::SetBang(var, val) => write!(f, "set! {var} {val}"),
            Inst::Throw(val) => write!(f, "throw {val}"),
            Inst::Phi(dst, entries) => write!(f, "{dst} = phi {entries:?}"),
            Inst::Recur(args) => write!(f, "recur {args:?}"),
            Inst::SourceLoc(span) => write!(f, "# {}:{}:{}", span.file, span.line, span.col),
            Inst::RegionStart(dst) => write!(f, "{dst} = region_start"),
            Inst::RegionAlloc(dst, region, kind, operands) => {
                write!(f, "{dst} = region_alloc {region} {kind} {operands:?}")
            }
            Inst::RegionEnd(region) => write!(f, "region_end {region}"),
        }
    }
}

impl fmt::Display for Terminator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Terminator::Jump(target) => write!(f, "jump {target}"),
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => write!(f, "branch {cond} then={then_block} else={else_block}"),
            Terminator::Return(val) => write!(f, "return {val}"),
            Terminator::RecurJump { target, args } => {
                write!(f, "recur_jump {target} {args:?}")
            }
            Terminator::Unreachable => write!(f, "unreachable"),
        }
    }
}
