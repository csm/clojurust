# cljrs-ir

Intermediate representation types shared between the clojurust compiler
(`cljrs-compiler`) and interpreter (`cljrs-eval`).

The IR is a control-flow graph of basic blocks in A-normal form (ANF) with SSA
phi nodes at join points.  Every sub-expression is bound to a named temporary
(`VarId`), and control flow is explicit via `Terminator`s.

**Purpose:** Extracted into its own crate so that both `cljrs-eval` (IR
interpreter, Tier 1 execution) and `cljrs-compiler` (Cranelift codegen, Tier 2)
can depend on the same types without a circular dependency.

---

## File layout

```
src/
  lib.rs  — all IR types: IrFunction, Block, Inst, Terminator, VarId, BlockId,
             KnownFn, Effect, Const, ClosureTemplate, RegionAllocKind
```

---

## Public API

### Core types

```rust
pub struct VarId(pub u32);
pub struct BlockId(pub u32);

pub struct IrFunction {
    pub name: Option<Arc<str>>,
    pub params: Vec<(Arc<str>, VarId)>,
    pub blocks: Vec<Block>,
    pub next_var: u32,
    pub next_block: u32,
    pub span: Option<Span>,
    pub subfunctions: Vec<IrFunction>,
}

pub struct Block {
    pub id: BlockId,
    pub phis: Vec<Inst>,
    pub insts: Vec<Inst>,
    pub terminator: Terminator,
}
```

### Instructions (`Inst`)

`Const`, `LoadLocal`, `LoadGlobal`, `LoadVar`, `AllocVector`, `AllocMap`,
`AllocSet`, `AllocList`, `AllocCons`, `AllocClosure`, `CallKnown`, `Call`,
`CallDirect`, `Deref`, `DefVar`, `SetBang`, `Throw`, `Phi`, `Recur`,
`SourceLoc`, `RegionStart`, `RegionAlloc`, `RegionEnd`

### Terminators

`Jump`, `Branch`, `Return`, `RecurJump`, `Unreachable`

### Known functions (`KnownFn`)

160+ built-in function identifiers with effect classification (`Effect`):
`Pure`, `Alloc`, `HeapRead`, `HeapWrite`, `IO`, `UnknownCall`.

### Region allocation

`RegionAllocKind`: `Vector`, `Map`, `Set`, `List`, `Cons`

### Closures

`ClosureTemplate`: static description of an `fn*` form (arity info, capture names).

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span` type for source locations |
