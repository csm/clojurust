# cljrs-compiler

Program analysis, optimization, and AOT compilation for clojurust. Provides an
intermediate representation (IR) in A-normal form with SSA, escape analysis,
Cranelift-based native code generation, and a C-ABI runtime bridge.

**Phase:** 8.1 (optimization) + 11 (AOT compilation) — end-to-end AOT working for simple programs.

---

## File layout

```
src/
  lib.rs      — module declarations, crate doc
  ir.rs       — IR types: IrFunction, Block, Inst, Terminator, VarId, BlockId, KnownFn, Effect, Const
  anf.rs      — ANF lowering: Form AST → IR instructions (AstLowering builder)
  escape.rs   — Escape analysis: EscapeState, def-use chains, collection chain detection
  rt_abi.rs   — C-ABI runtime bridge: ~40 extern "C" functions called by compiled code
  codegen.rs  — Cranelift code generator: IrFunction → native object code
  aot.rs      — AOT driver: source file → parse → expand → lower → codegen → cargo build → binary
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

### Runtime bridge (`rt_abi.rs`)

All functions are `#[unsafe(no_mangle)] pub extern "C"` — called by symbol name from compiled code.

- **Constants:** `rt_const_nil`, `rt_const_true`, `rt_const_false`, `rt_const_long(i64)`, `rt_const_double(f64)`, `rt_const_char(u32)`, `rt_const_string(ptr, len)`, `rt_const_keyword(ptr, len)`, `rt_const_symbol(ptr, len)`
- **Truthiness:** `rt_truthiness(v) -> u8`
- **Arithmetic:** `rt_add`, `rt_sub`, `rt_mul`, `rt_div`, `rt_rem`
- **Comparison:** `rt_eq`, `rt_lt`, `rt_gt`, `rt_lte`, `rt_gte`
- **Collections:** `rt_alloc_vector`, `rt_alloc_map`, `rt_alloc_set`, `rt_alloc_list`, `rt_alloc_cons`, `rt_get`, `rt_count`, `rt_first`, `rt_rest`, `rt_assoc`, `rt_conj`
- **Dispatch:** `rt_call(callee, args, nargs)`, `rt_deref(v)`, `rt_load_global(ns, ns_len, name, name_len)`
- **Output:** `rt_println(v)`, `rt_pr(v)`, `rt_str(v)`
- **Type checks:** `rt_is_nil`, `rt_is_vector`, `rt_is_map`, `rt_is_seq`, `rt_identical`
- **Linker anchor:** `anchor_rt_symbols()` — call from harness to prevent dead-code elimination

### Cranelift codegen (`codegen.rs`)

```rust
pub struct Compiler { ... }
impl Compiler {
    pub fn new() -> CodegenResult<Self>;
    pub fn declare_function(&mut self, name: &str, param_count: usize) -> CodegenResult<FuncId>;
    pub fn compile_function(&mut self, ir_func: &IrFunction, func_id: FuncId) -> CodegenResult<()>;
    pub fn finish(self) -> Vec<u8>;
}
```

### AOT driver (`aot.rs`)

```rust
pub fn compile_file(src_path: &Path, out_path: &Path, src_dirs: &[PathBuf]) -> AotResult<()>;
```

Pipeline: read source → parse → macro-expand (via interpreter env) → ANF lower → Cranelift codegen → generate Cargo harness → `cargo build --release` → copy binary.

### Escape analysis (`escape.rs`)

```rust
pub fn analyze(func: &IrFunction) -> EscapeAnalysis;
pub fn detect_collection_chains(func: &IrFunction, escape: &EscapeAnalysis) -> Vec<CollectionChain>;
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr<Value>` — GC interaction |
| `cljrs-value` (workspace) | `Value`, collections, `NativeFn` — value types referenced by IR and rt_abi |
| `cljrs-reader` (workspace) | `Form`, `FormKind` — input AST for lowering |
| `cljrs-eval` (workspace) | `Env`, `GlobalEnv`, macros, callback — macro expansion + rt_call dispatch |
| `cljrs-stdlib` (workspace) | `standard_env` — bootstrap environment for macro expansion + harness |
| `cranelift-*` (workspace) | Cranelift compiler infrastructure |
| `target-lexicon` (workspace) | Target triple detection |
