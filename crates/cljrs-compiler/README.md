# cljrs-compiler

Program analysis, optimization, and AOT compilation for clojurust. Provides an
intermediate representation (IR) in A-normal form with SSA, escape analysis,
Cranelift-based native code generation, and a C-ABI runtime bridge.

ANF lowering and escape analysis are written in Clojure (`cljrs.compiler.anf`,
`cljrs.compiler.escape`), producing IR as plain data maps. A thin Rust conversion
layer (`ir_convert.rs`) translates these back to the `IrFunction` structs that the
Cranelift codegen backend consumes.

**Phase:** 8.1 (optimization) + 11 (AOT compilation) + no-gc phases 6–7 — end-to-end AOT working for multi-file programs with variadic functions, protocols, escape analysis optimization, apply, core HOFs, sequence/collection ops, type predicates, atom constructor, and inline expansions.  Under the `no-gc` feature the AOT driver also runs the **blacklist analysis** (`escape.rs`) which rejects programs that cannot be safely compiled without a GC.

---

## File layout

```
src/
  lib.rs        — module declarations, embedded Clojure sources, register_compiler_sources()
  ir.rs         — re-exports all types from cljrs-ir crate
  ir_convert.rs — Value → IrFunction conversion (Clojure data → Rust IR types)
  rt_abi.rs     — C-ABI runtime bridge: ~40 extern "C" functions called by compiled code
  codegen.rs    — Cranelift code generator: IrFunction → native object code
  aot.rs        — AOT driver: source → parse → expand → lower → codegen → cargo build → binary
  escape.rs     — (no-gc only) blacklist analysis: 4 checks that reject no-gc–unsafe IR patterns
  clojure/compiler/
    ir.cljrs      — IR data constructors + mutable builder context (atom-based)
    known.cljrs   — Known function symbol → keyword resolution table
    anf.cljrs     — ANF lowering (Clojure): Form values → IR data maps
    escape.cljrs  — Escape analysis (Clojure): operates on plain IR data
    optimize.cljrs — Optimization passes (Clojure): escape analysis → region allocation rewriting
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
pub enum KnownFn { Vector, HashMap, Assoc, Conj, Get, Count, Add, Sub, Apply, Reduce2, Map, Filter, Mapv, Range1, Take, Drop, Concat, Sort, Keys, Vals, Merge, Update, Atom, ... }
pub enum Effect { Pure, Alloc, HeapRead, HeapWrite, IO, UnknownCall }
```

### IR conversion (`ir_convert.rs`)

```rust
pub fn value_to_ir_function(val: &Value) -> ConvertResult<IrFunction>;
pub fn keyword_to_known_fn(kw: &str) -> Option<KnownFn>;
```

Converts Clojure data maps (produced by the Clojure front-end) back to Rust IR types.

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
- **Region alloc:** `rt_region_start`, `rt_region_end`, `rt_region_alloc_vector`, `rt_region_alloc_map`, `rt_region_alloc_set`, `rt_region_alloc_list`, `rt_region_alloc_cons`
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

pub enum AotError { Io, Parse, Codegen, Eval, Link, NoGcBlacklist(Vec<BlacklistViolation>) /* no-gc only */ }
```

Pipeline: read source → parse → evaluate preamble → macro-expand → discover required namespaces → ANF lower (Clojure) → optimize (escape analysis + region alloc) → IR convert → **[no-gc] blacklist check** → Cranelift codegen → generate Cargo harness → `cargo build --release` → copy binary.

The generated harness `main()` (and the `compile_test_harness` test runner)
calls `cljrs_gc::dump_stats_from_env()` once at exit, so AOT binaries honor
the `CLJRS_GC_STATS` env var (empty/`"-"` → stdout, otherwise a file path).

### No-GC blacklist (`escape.rs`, no-gc only)

```rust
pub enum BlacklistViolation { InteriorPointerReturn { .. }, RegionToStaticStore { .. }, LazySeqEscape { .. }, EscapingClosure { .. } }
pub fn check(func: &IrFunction) -> Vec<BlacklistViolation>;
pub fn check_function(func: &IrFunction) -> Vec<BlacklistViolation>;
```

Detects four classes of no-gc memory-safety violations in IR functions:
1. **InteriorPointerReturn** — return var is (transitively via phi) an allocation from the function's scratch region.
2. **RegionToStaticStore** — allocation result flows into `DefVar` / `SetBang` without the static context.
3. **LazySeqEscape** — lazy-producing call result is bound as an intermediate and returned unrealized.
4. **EscapingClosure** — `AllocClosure` stored in a static container.

Multi-file support: when the source file uses `(ns ... (:require [...]))`, the required namespaces are loaded during compilation. Their source files are discovered from `src_dirs`, bundled into the harness as builtin sources, and made available at runtime so the binary is self-contained.

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

### `cljrs.compiler.optimize`
Optimization passes on IR data maps. Currently implements region allocation: rewrites non-escaping allocations (identified by escape analysis) into `region-start`/`region-alloc`/`region-end` instructions. Recursively optimizes subfunctions.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljrs-ir` (workspace) | IR types: `IrFunction`, `Block`, `Inst`, `KnownFn`, etc. |
| `cljrs-gc` (workspace) | `GcPtr<Value>` — GC interaction |
| `cljrs-value` (workspace) | `Value`, collections, `NativeFn` — value types referenced by IR and rt_abi |
| `cljrs-reader` (workspace) | `Form`, `FormKind` — input AST for lowering |
| `cljrs-eval` (workspace) | `Env`, `GlobalEnv`, macros, callback — macro expansion + rt_call dispatch |
| `cljrs-stdlib` (workspace) | `standard_env` — bootstrap environment for macro expansion + harness |
| `cranelift-*` (workspace) | Cranelift compiler infrastructure |
| `target-lexicon` (workspace) | Target triple detection |
