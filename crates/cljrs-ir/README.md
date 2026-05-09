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
  cljrs/compiler/
    ir.cljrs       — IR data constructors + mutable builder context (atom-based)
    known.cljrs    — symbol-name → KnownFn keyword resolution
    anf.cljrs      — ANF lowering: Form values → IR data maps
    escape.cljrs   — escape analysis on plain IR data maps
    optimize.cljrs — region-allocation optimization (escape → region rewriting)

test/cljrs/compiler/
  ir_test.cljrs       — clojure.test cases for `cljrs.compiler.ir`
  known_test.cljrs    — clojure.test cases for `cljrs.compiler.known`
  escape_test.cljrs   — clojure.test cases for `cljrs.compiler.escape`
  optimize_test.cljrs — clojure.test cases for `cljrs.compiler.optimize`
tests/
  clojure_tests.rs    — Rust integration test that boots a standard env,
                        requires each `*_test` namespace, runs
                        `clojure.test/run-tests`, and fails if any Clojure
                        assertion failed or errored.
```

---

## Running the Clojure-side tests

`cargo test -p cljrs-ir --test clojure_tests` runs the embedded
`clojure.test` suites against the compiler namespaces.  Add a new
`*_test.cljrs` file under `test/cljrs/compiler/` and append its namespace
to the `TEST_NSES` list in `tests/clojure_tests.rs` to extend coverage.

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

Some `KnownFn` variants exist purely for analysis precision — the
codegen and IR interpreter dispatch them through the dynamic builtin
lookup like a regular `Call`, but the analyzer can use them to tighten
escape verdicts.  For example, `Empty?`, `Peek`, `Pop`, `Vec`,
`Mapcat`, `Repeatedly` carry no specialised codegen path; they're
recognised so that the escape analyzer can see through `(empty? coll)`
or `(pop coll)` instead of treating them as opaque `UnknownCall`s.

### Recur and escape analysis

`UseKind::Recur` is *not* treated as an unconditional escape.  When the
analyzer encounters a `Recur` use, it walks to the matching loop-header
`Phi` (positionally aligned with the `RecurJump`'s args) and continues
analysis from the phi's downstream uses.  This is sound because `recur`
is structural control flow — values rebind at the loop header without
leaving the function — and it's what allows a loop-local empty vector
to reach `NoEscape` and get promoted to a region.

### Region allocation

`RegionAllocKind`: `Vector`, `Map`, `Set`, `List`, `Cons`

### Closures

`ClosureTemplate`: static description of an `fn*` form (arity info, capture names).

### Analysis (re-exported from `lower::`)

```rust
pub fn analyze(ir: &IrFunction, ctx: Option<&EscapeContext>) -> AnalysisResult;
pub fn make_analysis_context(ir: &IrFunction) -> EscapeContext;

pub enum EscapeState { NoEscape, ArgEscape, Returns, Escapes }
pub struct UseInfo { pub block: BlockId, pub kind: UseKind }
pub enum UseKind { Return, DefVar, SetBang, ClosureCapture, Throw,
                   StoredInHeap, Recur, KnownCallArg{..}, UnknownCallArg{..},
                   PhiInput, BranchCond, Deref, CallCallee }
pub struct AnalysisResult {
    pub states:       HashMap<VarId, EscapeState>,
    pub uses:         HashMap<VarId, Vec<UseInfo>>,
    pub alloc_blocks: HashMap<VarId, BlockId>,
}
```

These are the same types the optimizer uses internally; they are exposed
publicly so downstream tooling (e.g. `cljrs-ir-viz`) can present
escape-analysis results without re-implementing the use-chain walk.

### Source mapping

ANF lowering emits `Inst::SourceLoc(span)` markers at the head of each
form's lowering, deduplicated per `(file, line)` within a basic block.
`SourceLoc` has no `dst` and `Effect::Pure`, so it is invisible to the
optimizer and codegen — it exists for downstream tooling only.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span` type for source locations |
