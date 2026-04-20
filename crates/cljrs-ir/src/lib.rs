//! Intermediate representation for clojurust program analysis and optimization.
//!
//! The IR is a control-flow graph of basic blocks containing instructions in
//! A-normal form (all sub-expressions bound to named temporaries). It supports
//! SSA construction via phi nodes at join points.
//!
//! The IR serves multiple purposes:
//! 1. **Escape analysis** and optimization hints
//! 2. **IR interpreter** (Tier 1 execution)
//! 3. **Cranelift-based JIT/AOT code generation** (Tier 2 execution)

#![allow(clippy::result_large_err)]

use cljrs_types::error::CljxError::SerializationError;
use cljrs_types::error::CljxResult;
use cljrs_types::span::Span;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

// ── Variable IDs ─────────────────────────────────────────────────────────────

/// A unique variable identifier within an IR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VarId(pub u32);

impl fmt::Display for VarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A basic block identifier within an IR function.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    // Higher-order functions
    Reduce2,
    Reduce3,
    Map,
    Filter,
    Mapv,
    Filterv,
    Some,
    Every,
    Into,
    Into3,

    // More HOFs
    GroupBy,
    Partition2,
    Partition3,
    Partition4,
    Frequencies,
    Keep,
    Remove,
    MapIndexed,
    Zipmap,
    Juxt,
    Comp,
    Partial,
    Complement,

    // Sequence operations
    Concat,
    Range1,
    Range2,
    Range3,
    Take,
    Drop,
    Reverse,
    Sort,
    SortBy,

    // Collection operations
    Keys,
    Vals,
    Merge,
    Update,
    GetIn,
    AssocIn,

    // Type predicates
    IsNumber,
    IsString,
    IsKeyword,
    IsSymbol,
    IsBool,
    IsInt,

    // Additional I/O
    Prn,
    Print,

    // Atom construction
    Atom,

    // Exception handling
    TryCatchFinally,

    // Dynamic binding
    SetBangVar,
    WithBindings,

    // Output capture
    WithOutStr,
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Inst {
    /// Load a constant value.
    Const(VarId, Const),

    /// Load a local variable by name (from the interpreter's Env).
    LoadLocal(VarId, Arc<str>),

    /// Load a global var by namespace-qualified name (returns the dereferenced value).
    LoadGlobal(VarId, Arc<str>, Arc<str>), // dst, ns, name

    /// Load a global Var object (not its value) — for `set!` and `binding`.
    LoadVar(VarId, Arc<str>, Arc<str>), // dst, ns, name

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

    /// Call a compiled function directly by name (bypasses dynamic dispatch).
    /// Generated by the direct-call optimization pass when a defn in the same
    /// compilation unit is called with a matching arity.
    CallDirect(VarId, Arc<str>, Vec<VarId>), // dst, compiled_fn_name, args

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosureTemplate {
    /// Function name (if named).
    pub name: Option<Arc<str>>,
    /// Compiled function names for each arity (indices match `param_counts`).
    pub arity_fn_names: Vec<Arc<str>>,
    /// Fixed parameter count for each arity (excludes rest param for variadic arities).
    pub param_counts: Vec<usize>,
    /// Whether each arity is variadic (has a `& rest` parameter).
    /// Variadic arities accept `param_counts[i]` or more arguments; extra args
    /// are packed into a list for the rest parameter.
    pub is_variadic: Vec<bool>,
    /// Names of the captured variables (in order).
    pub capture_names: Vec<Arc<str>>,
}

// ── Terminators ──────────────────────────────────────────────────────────────

/// A block terminator — controls flow between basic blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    RecurJump { target: BlockId, args: Vec<VarId> },

    /// Unreachable (e.g., after a `throw`).
    Unreachable,
}

// ── Basic blocks and functions ───────────────────────────────────────────────

/// A basic block: a linear sequence of instructions followed by a terminator.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// Nested function bodies (from `fn*` forms), each compiled separately.
    pub subfunctions: Vec<IrFunction>,
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
            subfunctions: Vec::new(),
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

    /// Build a block index: `block_id.0` → index in `self.blocks`.
    ///
    /// If block IDs are dense and match array indices (the common case from
    /// the compiler), returns `None` — callers can use `block_id.0 as usize`
    /// directly.  Otherwise returns a lookup table.
    pub fn block_index(&self) -> Option<Vec<usize>> {
        // Check if block IDs are dense and sequential (0, 1, 2, ...).
        let is_identity = self
            .blocks
            .iter()
            .enumerate()
            .all(|(i, b)| b.id.0 as usize == i);
        if is_identity {
            return None; // Use block_id.0 directly as index.
        }
        // Sparse case: build a lookup table.
        let max_id = self.blocks.iter().map(|b| b.id.0).max().unwrap_or(0);
        let mut table = vec![0usize; max_id as usize + 1];
        for (i, b) in self.blocks.iter().enumerate() {
            table[b.id.0 as usize] = i;
        }
        Some(table)
    }

    pub fn serialize(&self) -> CljxResult<Vec<u8>> {
        postcard::to_allocvec(self).map_err(|e| SerializationError {
            message: e.to_string(),
        })
    }

    pub fn deserialize(bytes: &[u8]) -> CljxResult<Self> {
        postcard::from_bytes(bytes).map_err(|e| SerializationError {
            message: e.to_string(),
        })
    }
}

// ── IR Bundle ───────────────────────────────────────────────────────────────

/// A bundle of pre-lowered IR functions, keyed by a string identifier.
///
/// Used to serialize multiple functions (e.g. an entire namespace) into a
/// single blob that can be loaded at startup without running the Clojure
/// compiler.
#[derive(Debug, Serialize, Deserialize)]
pub struct IrBundle {
    /// Bundle entries keyed by identifier (typically `"ns/name:arity"` or
    /// the arity ID as a string).
    pub functions: HashMap<String, IrFunction>,
}

impl IrBundle {
    pub fn new() -> Self {
        Self {
            functions: HashMap::new(),
        }
    }

    /// Insert a function into the bundle.
    pub fn insert(&mut self, key: String, func: IrFunction) {
        self.functions.insert(key, func);
    }

    /// Look up a function by key.
    pub fn get(&self, key: &str) -> Option<&IrFunction> {
        self.functions.get(key)
    }

    /// Number of functions in the bundle.
    pub fn len(&self) -> usize {
        self.functions.len()
    }

    /// Whether the bundle is empty.
    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

impl Default for IrBundle {
    fn default() -> Self {
        Self::new()
    }
}

/// Serialize an [`IrBundle`] to bytes.
pub fn serialize_bundle(bundle: &IrBundle) -> CljxResult<Vec<u8>> {
    postcard::to_allocvec(bundle).map_err(|e| SerializationError {
        message: e.to_string(),
    })
}

/// Deserialize an [`IrBundle`] from bytes.
pub fn deserialize_bundle(bytes: &[u8]) -> CljxResult<IrBundle> {
    postcard::from_bytes(bytes).map_err(|e| SerializationError {
        message: e.to_string(),
    })
}

// ── Effect classification ────────────────────────────────────────────────────

impl Inst {
    /// Return the primary effect of this instruction.
    pub fn effect(&self) -> Effect {
        match self {
            Inst::Const(..) | Inst::LoadLocal(..) | Inst::Phi(..) | Inst::SourceLoc(..) => {
                Effect::Pure
            }
            Inst::LoadGlobal(..) | Inst::LoadVar(..) => Effect::HeapRead,
            Inst::AllocVector(..)
            | Inst::AllocMap(..)
            | Inst::AllocSet(..)
            | Inst::AllocList(..)
            | Inst::AllocCons(..)
            | Inst::AllocClosure(..) => Effect::Alloc,
            Inst::CallKnown(_, known, _) => known.effect(),
            Inst::Call(..) | Inst::CallDirect(..) => Effect::UnknownCall,
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
            | Inst::LoadVar(v, _, _)
            | Inst::AllocVector(v, _)
            | Inst::AllocMap(v, _)
            | Inst::AllocSet(v, _)
            | Inst::AllocList(v, _)
            | Inst::AllocCons(v, _, _)
            | Inst::AllocClosure(v, _, _)
            | Inst::CallKnown(v, _, _)
            | Inst::Call(v, _, _)
            | Inst::CallDirect(v, _, _)
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
            Inst::Const(..)
            | Inst::LoadLocal(..)
            | Inst::LoadGlobal(..)
            | Inst::LoadVar(..)
            | Inst::SourceLoc(..) => vec![],
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
            Inst::CallDirect(_, _, args) => args.clone(),
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

            // Sequence operations (allocating)
            KnownFn::Concat
            | KnownFn::Range1
            | KnownFn::Range2
            | KnownFn::Range3
            | KnownFn::Take
            | KnownFn::Drop
            | KnownFn::Reverse => Effect::Alloc,

            // Sort calls comparator (unknown call)
            KnownFn::Sort | KnownFn::SortBy => Effect::UnknownCall,

            // Collection operations
            KnownFn::Keys | KnownFn::Vals => Effect::Alloc,
            KnownFn::Merge | KnownFn::Update | KnownFn::GetIn | KnownFn::AssocIn => Effect::Alloc,

            // Type predicates
            KnownFn::IsNumber
            | KnownFn::IsString
            | KnownFn::IsKeyword
            | KnownFn::IsSymbol
            | KnownFn::IsBool
            | KnownFn::IsInt => Effect::Pure,

            // Additional I/O
            KnownFn::Prn | KnownFn::Print => Effect::IO,

            // Atom construction
            KnownFn::Atom => Effect::Alloc,

            // More HOFs call unknown functions
            KnownFn::GroupBy
            | KnownFn::Partition2
            | KnownFn::Partition3
            | KnownFn::Partition4
            | KnownFn::Keep
            | KnownFn::Remove
            | KnownFn::MapIndexed => Effect::UnknownCall,

            // Function combinators (return new fns, call unknown fns)
            KnownFn::Juxt | KnownFn::Comp | KnownFn::Partial | KnownFn::Complement => {
                Effect::UnknownCall
            }

            // Pure collection ops
            KnownFn::Frequencies | KnownFn::Zipmap => Effect::Alloc,

            // HOFs call unknown functions
            KnownFn::Reduce2
            | KnownFn::Reduce3
            | KnownFn::Map
            | KnownFn::Filter
            | KnownFn::Mapv
            | KnownFn::Filterv
            | KnownFn::Some
            | KnownFn::Every
            | KnownFn::Into
            | KnownFn::Into3 => Effect::UnknownCall,

            // Try/catch calls unknown closures
            KnownFn::TryCatchFinally => Effect::UnknownCall,

            // Dynamic binding
            KnownFn::SetBangVar => Effect::HeapWrite,
            KnownFn::WithBindings | KnownFn::WithOutStr => Effect::UnknownCall,
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
            Inst::LoadVar(dst, ns, name) => write!(f, "{dst} = load_var {ns}/{name}"),
            Inst::AllocVector(dst, elems) => write!(f, "{dst} = alloc_vec {elems:?}"),
            Inst::AllocMap(dst, pairs) => write!(f, "{dst} = alloc_map {pairs:?}"),
            Inst::AllocSet(dst, elems) => write!(f, "{dst} = alloc_set {elems:?}"),
            Inst::AllocList(dst, elems) => write!(f, "{dst} = alloc_list {elems:?}"),
            Inst::AllocCons(dst, h, t) => write!(f, "{dst} = cons {h} {t}"),
            Inst::AllocClosure(dst, tmpl, captures) => {
                write!(f, "{dst} = closure {:?} captures={captures:?}", tmpl.name)
            }
            Inst::CallKnown(dst, func, args) => write!(f, "{dst} = call_known {func:?} {args:?}"),
            Inst::Call(dst, callee, args) => write!(f, "{dst} = call {callee} {args:?}"),
            Inst::CallDirect(dst, name, args) => write!(f, "{dst} = call_direct {name} {args:?}"),
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

// ── Embedded Clojure compiler sources ───────────────────────────────────────

/// Clojure source for the IR builder namespace.
pub const COMPILER_IR_SOURCE: &str = include_str!("clojure/compiler/ir.cljrs");

/// Clojure source for the known function resolution namespace.
pub const COMPILER_KNOWN_SOURCE: &str = include_str!("clojure/compiler/known.cljrs");

/// Clojure source for the ANF lowering namespace.
pub const COMPILER_ANF_SOURCE: &str = include_str!("clojure/compiler/anf.cljrs");

/// Clojure source for the escape analysis namespace.
pub const COMPILER_ESCAPE_SOURCE: &str = include_str!("clojure/compiler/escape.cljrs");

/// Clojure source for the optimization pass namespace.
pub const COMPILER_OPTIMIZE_SOURCE: &str = include_str!("clojure/compiler/optimize.cljrs");

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple IR function for testing: one block that returns a constant.
    fn make_test_fn(name: &str, const_val: i64) -> IrFunction {
        let mut f = IrFunction::new(Some(Arc::from(name)), None);
        let dst = f.fresh_var();
        let block_id = f.fresh_block();
        f.blocks.push(Block {
            id: block_id,
            phis: vec![],
            insts: vec![Inst::Const(dst, Const::Long(const_val))],
            terminator: Terminator::Return(dst),
        });
        f
    }

    #[test]
    fn test_ir_function_serialize_roundtrip() {
        let f = make_test_fn("identity", 42);
        let bytes = f.serialize().unwrap();
        let f2 = IrFunction::deserialize(&bytes).unwrap();
        assert_eq!(f2.name.as_deref(), Some("identity"));
        assert_eq!(f2.blocks.len(), 1);
        assert_eq!(f2.next_var, 1);
        match &f2.blocks[0].insts[0] {
            Inst::Const(_, Const::Long(v)) => assert_eq!(*v, 42),
            other => panic!("expected Const(Long(42)), got {other:?}"),
        }
    }

    #[test]
    fn test_ir_function_with_closure_template() {
        let mut f = IrFunction::new(Some(Arc::from("outer")), None);
        let dst = f.fresh_var();
        let capture = f.fresh_var();
        let block_id = f.fresh_block();
        f.blocks.push(Block {
            id: block_id,
            phis: vec![],
            insts: vec![
                Inst::Const(capture, Const::Str(Arc::from("hello"))),
                Inst::AllocClosure(
                    dst,
                    ClosureTemplate {
                        name: Some(Arc::from("inner")),
                        arity_fn_names: vec![Arc::from("inner__0")],
                        param_counts: vec![1],
                        is_variadic: vec![false],
                        capture_names: vec![Arc::from("x")],
                    },
                    vec![capture],
                ),
            ],
            terminator: Terminator::Return(dst),
        });

        let bytes = f.serialize().unwrap();
        let f2 = IrFunction::deserialize(&bytes).unwrap();
        match &f2.blocks[0].insts[1] {
            Inst::AllocClosure(_, tmpl, captures) => {
                assert_eq!(tmpl.name.as_deref(), Some("inner"));
                assert_eq!(tmpl.param_counts, vec![1]);
                assert_eq!(tmpl.is_variadic, vec![false]);
                assert_eq!(captures.len(), 1);
            }
            other => panic!("expected AllocClosure, got {other:?}"),
        }
    }

    #[test]
    fn test_empty_bundle_roundtrip() {
        let bundle = IrBundle::new();
        assert!(bundle.is_empty());
        let bytes = serialize_bundle(&bundle).unwrap();
        let bundle2 = deserialize_bundle(&bytes).unwrap();
        assert!(bundle2.is_empty());
        assert_eq!(bundle2.len(), 0);
    }

    #[test]
    fn test_bundle_single_function() {
        let mut bundle = IrBundle::new();
        bundle.insert("clojure.core/inc:1".to_string(), make_test_fn("inc", 1));
        assert_eq!(bundle.len(), 1);

        let bytes = serialize_bundle(&bundle).unwrap();
        let bundle2 = deserialize_bundle(&bytes).unwrap();
        assert_eq!(bundle2.len(), 1);

        let f = bundle2.get("clojure.core/inc:1").unwrap();
        assert_eq!(f.name.as_deref(), Some("inc"));
    }

    #[test]
    fn test_bundle_multiple_functions() {
        let mut bundle = IrBundle::new();
        bundle.insert("clojure.core/inc:1".to_string(), make_test_fn("inc", 1));
        bundle.insert("clojure.core/dec:1".to_string(), make_test_fn("dec", -1));
        bundle.insert(
            "clojure.core/identity:1".to_string(),
            make_test_fn("identity", 0),
        );
        assert_eq!(bundle.len(), 3);

        let bytes = serialize_bundle(&bundle).unwrap();
        let bundle2 = deserialize_bundle(&bytes).unwrap();
        assert_eq!(bundle2.len(), 3);

        assert_eq!(
            bundle2.get("clojure.core/inc:1").unwrap().name.as_deref(),
            Some("inc")
        );
        assert_eq!(
            bundle2.get("clojure.core/dec:1").unwrap().name.as_deref(),
            Some("dec")
        );
        assert_eq!(
            bundle2
                .get("clojure.core/identity:1")
                .unwrap()
                .name
                .as_deref(),
            Some("identity")
        );
        assert!(bundle2.get("nonexistent").is_none());
    }

    #[test]
    fn test_bundle_with_complex_ir() {
        let mut f = IrFunction::new(Some(Arc::from("complex")), None);
        let p0 = f.fresh_var();
        let p1 = f.fresh_var();
        f.params = vec![(Arc::from("x"), p0), (Arc::from("y"), p1)];

        // Entry block: branch on x
        let entry = f.fresh_block();
        let then_bb = f.fresh_block();
        let else_bb = f.fresh_block();
        let join_bb = f.fresh_block();

        let cond_dst = f.fresh_var();
        f.blocks.push(Block {
            id: entry,
            phis: vec![],
            insts: vec![Inst::CallKnown(cond_dst, KnownFn::IsNil, vec![p0])],
            terminator: Terminator::Branch {
                cond: cond_dst,
                then_block: then_bb,
                else_block: else_bb,
            },
        });

        // Then block: return y
        f.blocks.push(Block {
            id: then_bb,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Jump(join_bb),
        });

        // Else block: return x
        f.blocks.push(Block {
            id: else_bb,
            phis: vec![],
            insts: vec![],
            terminator: Terminator::Jump(join_bb),
        });

        // Join block: phi + return
        let phi_dst = f.fresh_var();
        f.blocks.push(Block {
            id: join_bb,
            phis: vec![Inst::Phi(phi_dst, vec![(then_bb, p1), (else_bb, p0)])],
            insts: vec![],
            terminator: Terminator::Return(phi_dst),
        });

        let mut bundle = IrBundle::new();
        bundle.insert("test/complex:2".to_string(), f);

        let bytes = serialize_bundle(&bundle).unwrap();
        let bundle2 = deserialize_bundle(&bytes).unwrap();

        let f2 = bundle2.get("test/complex:2").unwrap();
        assert_eq!(f2.params.len(), 2);
        assert_eq!(f2.blocks.len(), 4);

        // Verify branch terminator survived roundtrip
        match &f2.blocks[0].terminator {
            Terminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                assert_eq!(*cond, cond_dst);
                assert_eq!(*then_block, then_bb);
                assert_eq!(*else_block, else_bb);
            }
            other => panic!("expected Branch, got {other:?}"),
        }

        // Verify phi survived roundtrip
        assert_eq!(f2.blocks[3].phis.len(), 1);
        match &f2.blocks[3].phis[0] {
            Inst::Phi(dst, entries) => {
                assert_eq!(*dst, phi_dst);
                assert_eq!(entries.len(), 2);
            }
            other => panic!("expected Phi, got {other:?}"),
        }
    }

    #[test]
    fn test_bundle_with_subfunctions() {
        let mut outer = make_test_fn("outer", 100);
        let inner = make_test_fn("inner", 200);
        outer.subfunctions.push(inner);

        let mut bundle = IrBundle::new();
        bundle.insert("test/outer:0".to_string(), outer);

        let bytes = serialize_bundle(&bundle).unwrap();
        let bundle2 = deserialize_bundle(&bytes).unwrap();

        let f = bundle2.get("test/outer:0").unwrap();
        assert_eq!(f.subfunctions.len(), 1);
        assert_eq!(f.subfunctions[0].name.as_deref(), Some("inner"));
    }

    #[test]
    fn test_deserialize_invalid_bytes() {
        let result = IrFunction::deserialize(&[0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());

        let result = deserialize_bundle(&[0xFF, 0xFE, 0xFD]);
        assert!(result.is_err());
    }
}
