# cljrs-compiler

Program analysis and optimization for clojurust. Provides an intermediate
representation (IR) in A-normal form with SSA, escape analysis, and collection
chain detection. Currently generates optimization hints for the interpreter;
will serve as the input to Cranelift-based JIT/AOT code generation in Phase 10/11.

**Phase:** 8.1 (program optimization) — IR foundation + escape analysis implemented.

---

## File layout

```
src/
  lib.rs      — module declarations, crate doc
  ir.rs       — IR types: IrFunction, Block, Inst, Terminator, VarId, BlockId, KnownFn, Effect, Const
  anf.rs      — ANF lowering: Form AST → IR instructions (AstLowering builder)
  escape.rs   — Escape analysis: EscapeState, def-use chains, collection chain detection
```

---

## Public API

### IR types (`ir.rs`)

```rust
pub struct IrFunction { name, params, blocks, ... }
pub struct Block { id, phis, insts, terminator }
pub enum Inst { Const, LoadLocal, LoadGlobal, AllocVector, AllocMap, AllocSet, AllocList, AllocCons, AllocClosure, CallKnown, Call, Deref, DefVar, SetBang, Throw, Phi, Recur, SourceLoc, RegionStart, RegionAlloc, RegionEnd }
pub enum RegionAllocKind { Vector, Map, Set, List, Cons }
pub enum Terminator { Jump, Branch, Return, RecurJump, Unreachable }
pub enum KnownFn { Vector, HashMap, Assoc, Conj, Get, Count, Add, Sub, ... }
pub enum Effect { Pure, Alloc, HeapRead, HeapWrite, IO, UnknownCall }
```

### ANF lowering (`anf.rs`)

```rust
pub fn lower_fn_body(name: Option<&str>, ns: &str, params: &[Arc<str>], body: &[Form]) -> LowerResult<IrFunction>;
```

Handles: atoms, symbols, collections, `if`, `let`, `loop`/`recur`, `def`, `fn*`, `and`/`or`, `throw`, `set!`, `quote`, function calls (known + unknown).

### Escape analysis (`escape.rs`)

```rust
pub fn analyze(func: &IrFunction) -> EscapeAnalysis;
pub fn detect_collection_chains(func: &IrFunction, escape: &EscapeAnalysis) -> Vec<CollectionChain>;

pub enum EscapeState { NoEscape, ArgEscape { callee, arg_index }, Escapes }
pub struct CollectionChain { root, ops, result }
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr<Value>` — GC interaction |
| `cljrs-value` (workspace) | `Value`, `NativeFn` — value types referenced by IR |
| `cljrs-reader` (workspace) | `Form`, `FormKind` — input AST for lowering |
| `cljrs-eval` (workspace) | `Env`, `GlobalEnv` — evaluator types |
