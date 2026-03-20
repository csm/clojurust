# cljrs-compiler

Program analysis, optimization, and AOT compilation for clojurust. Provides an
intermediate representation (IR) in A-normal form with SSA, escape analysis,
Cranelift-based native code generation, and a C-ABI runtime bridge.

The compiler has two front-ends for ANF lowering:
- **Rust front-end** (`anf.rs`, `escape.rs`) — the original implementation
- **Clojure front-end** (`cljrs.compiler.anf`, `cljrs.compiler.escape`) — tree/graph transformations written in Clojure, producing IR as plain data maps

The AOT driver tries the Clojure front-end first and falls back to Rust on failure.

**Phase:** 8.1 (optimization) + 11 (AOT compilation) — end-to-end AOT working for simple programs.

---

## File layout

```
src/
  lib.rs        — module declarations, embedded Clojure sources, register_compiler_sources()
  ir.rs         — IR types: IrFunction, Block, Inst, Terminator, VarId, BlockId, KnownFn, Effect, Const
  ir_convert.rs — Value → IrFunction conversion (Clojure data → Rust IR types)
  anf.rs        — ANF lowering (Rust): Form AST → IR instructions (AstLowering builder)
  escape.rs     — Escape analysis (Rust): EscapeState, def-use chains, collection chain detection
  rt_abi.rs     — C-ABI runtime bridge: ~40 extern "C" functions called by compiled code
  codegen.rs    — Cranelift code generator: IrFunction → native object code
  aot.rs        — AOT driver: source → parse → expand → lower → codegen → cargo build → binary
  clojure/compiler/
    ir.cljrs      — IR data constructors + mutable builder context (atom-based)
    known.cljrs   — Known function symbol → keyword resolution table
    anf.cljrs     — ANF lowering (Clojure): Form values → IR data maps
    escape.cljrs  — Escape analysis (Clojure): operates on plain IR data
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

### IR conversion (`ir_convert.rs`)

```rust
pub fn value_to_ir_function(val: &Value) -> ConvertResult<IrFunction>;
pub fn keyword_to_known_fn(kw: &str) -> Option<KnownFn>;
```

Converts Clojure data maps (produced by the Clojure front-end) back to Rust IR types.

### ANF lowering (`anf.rs`)

```rust
pub fn lower_fn_body(name: Option<&str>, ns: &str, params: &[Arc<str>], body: &[Form]) -> LowerResult<IrFunction>;
```

Handles: atoms, symbols, collections, `if`, `let`, `loop`/`recur`, `def`, `fn*`, `and`/`or`, `throw`, `set!`, `quote`, function calls (known + unknown).

### Compiler source registration (`lib.rs`)

```rust
pub fn register_compiler_sources(globals: &Arc<GlobalEnv>);
```

Registers embedded Clojure compiler namespaces as builtin sources so `require` can load them.

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
pub fn lower_via_clojure(name: Option<&str>, ns: &str, params: &[Arc<str>], forms: &[Form], env: &mut Env) -> AotResult<IrFunction>;
```

Pipeline: read source → parse → macro-expand → ANF lower (Clojure→Rust fallback) → Cranelift codegen → generate Cargo harness → `cargo build --release` → copy binary.

### Escape analysis (`escape.rs`)

```rust
pub fn analyze(func: &IrFunction) -> EscapeAnalysis;
pub fn detect_collection_chains(func: &IrFunction, escape: &EscapeAnalysis) -> Vec<CollectionChain>;
```

---

## Clojure front-end namespaces

### `cljrs.compiler.ir`
Mutable builder context (atom-based) for constructing IR data maps. Provides constructors for all instruction/terminator types and scope management.

### `cljrs.compiler.known`
Maps symbol names (e.g. `"+"`, `"assoc"`, `"println"`) to IR keyword tags (e.g. `:+`, `:assoc`, `:println`).

### `cljrs.compiler.anf`
ANF lowering: converts Clojure form values (from `form_to_value`) into IR data maps. Supports the same special forms as the Rust front-end.

### `cljrs.compiler.escape`
Escape analysis on IR data maps. Determines allocation escape states and detects collection operation chains.

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
