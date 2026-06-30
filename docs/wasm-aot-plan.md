# Plan: AOT Clojure → WebAssembly (browser backend)

## Overview

This is the handoff document for the **AOT Clojure → WebAssembly backend** — a
second code-generation backend over the same `cljrs-ir` IR as the Cranelift
backend, targeting native-fast, sandbox-safe deployment in the browser.

> For the reader-facing reference guide to the whole feature — architecture,
> design decisions, and how each piece works — see [`wasm-aot.md`](./wasm-aot.md).
> *This* document is the increment-by-increment build log and open-task list.

The work lives entirely in `crates/cljrs-compiler/src/wasm/` and is developed on
branch `claude/wasm-aot-jit-compilation-gpz5nl`.

### Why a separate backend (the one hard constraint)

A wasm module **cannot generate and execute native machine code at runtime** —
there is no `mmap(PROT_EXEC)` inside the sandbox. So the Cranelift JIT
(`cljrs-jit`) cannot run in a browser: the in-sandbox story must be
*ahead-of-time*. Compile each Clojure function to wasm bytecode at build time
and ship it; the browser JITs *that* to native.

The runtime tiers therefore **invert** relative to native:

```text
  native:   tree-walk → IR-interp → JIT/OSR (peak, reached at runtime)
  browser:  tree-walk → IR-interp (dynamic: eval, REPL, freshly-required ns, macros)
            AOT-wasm                (peak, frozen at build time; browser JITs it)
```

The IR interpreter (`cljrs-eval::ir_interp`) therefore stays **on board** in the
wasm bundle as the dynamic-code tier; AOT-wasm is the frozen top tier. No
in-sandbox JIT/OSR hooks are installed.

### What is reused unchanged

Everything upstream of code generation is backend-agnostic and shared with the
Cranelift path:

- ANF/SSA lowering — `cljrs_ir::lower`
- Escape analysis + region inference — `cljrs_ir::lower::{escape, regionalize}`
- Scalar representation inference — `cljrs_compiler::typeinfer`
- The `rt_abi` runtime bridge contract — `cljrs_compiler::rt_abi`

Because escape analysis and region promotion are **properties of the IR**
(`Inst::Region*`, `IrFunction::takes_region_param`), bump allocation comes along
for free in wasm: a region is a linear-memory arena, a region handle is an `i32`
offset, and a region-parameterised variant takes that `i32` as a hidden trailing
parameter. The only genuinely new, wasm-specific work is **relooping** (recover
structured control flow) and the **wasm-encoder emitter**.

### Locked design decisions

- **Boxed-only value model first.** Every IR `VarId` is a wasm `i32` local
  holding a boxed `*const Value` (a linear-memory offset). This is the universal
  representation and is always correct; it mirrors the Cranelift backend's boxed
  fallback. Unboxed `Long`/`Double` specialization is deferred.
- **GC stays in linear memory.** Reuse the existing `wasm32-unknown-unknown` GC
  heap. WasmGC (host-managed reference types) is deferred indefinitely.
- **Relooper is wasm-private.** It lives in the wasm backend, not in shared
  lowering — Cranelift consumes the raw CFG and would be pessimized by
  re-structuring. (See "Any benefit to relooping in general?" — no; it is purely
  the price of a structured-control-flow target.)
- **`rt_abi` as wasm imports.** The emitted module imports the runtime bridge
  functions from the `"rt"` module; the runtime, compiled to the same linear
  memory, satisfies them at instantiation.

---

## Module layout

```
crates/cljrs-compiler/src/wasm/
  mod.rs      — public API (compile_function, WasmBackend, WasmError); tier model + rationale
  abi.rs      — ABI/region contract: Value→i32, rt_abi import table, region-handle threading
  reloop.rs   — relooper: IR CFG → structured control flow (Structured); wasm-private
  emit.rs     — wasm-encoder emitter: IrFunction → validated wasm module
```

Public API (`mod.rs`):

```rust
pub fn compile_function(func: &IrFunction, cfg: &WasmBackend) -> Result<Vec<u8>, WasmError>;
pub struct WasmBackend { tail_calls: bool, exceptions: bool }   // feature flags
pub enum WasmError { Reloop(RelooperError), Unsupported(String), Unimplemented(&'static str) }
```

`compile_function` runs `reloop::reloop(func)` then `emit::emit_function`.

---

## What is done

### ABI / region contract (`abi.rs`) — complete

The canonical, data-only contract that the emitter consumes (no `wasm-encoder`
dependency, so it is reviewable and unit-tested independently):

- **Value representation.** `*const Value`, `*mut Region`, and pointer-array
  slices all collapse to `i32` (a linear-memory offset). Unboxed `Long`→`i64`,
  `Double`→`f64`. `WasmValType::for_repr(Repr)` encodes this. Because every
  pointer is an `i32`, the entire ~165-function `rt_abi` surface is expressible
  as wasm imports with no marshalling beyond width changes.
- **Region ABI (bump allocation).** A region handle is an `i32` (offset of a
  `Region` arena in linear memory). `RegionStart`→`rt_region_start`,
  `RegionAlloc`→`rt_region_alloc_*` (handle leading), `RegionEnd`→
  `rt_region_end`, `RegionParam`→bind the hidden trailing `i32` param,
  `CallWithRegion`→direct call passing the caller's handle as the trailing arg.
  This mirrors `IrFunction::abi_param_count` on the native side.
- **Import table.** `RT_IMPORTS: &[RtImport]` describes the subset wired so far
  (safepoint, constants, arithmetic/comparison bridges, the `rt_scratch_ptr`
  marshalling buffer, GC-heap + region allocation, `rt_call`, the `rt_make_fn*`
  closure constructors, inline-cache/deopt bridges) as `(name, params, results)`
  wasm function types. `lookup(name)` resolves one. Completing it to all of
  `rt_abi` is mechanical — one `RtImport` per `extern "C"` signature.
- **Shared function table.** A closure's `fn_ptr` is a `wasm32` table index, so
  `FUNC_TABLE_NAME`/`FUNC_TABLE_BASE` describe the imported
  `"rt" "__indirect_function_table"` and the base slot at which the emitter
  installs the bundle's functions (element segment in `emit.rs`).

### Relooper (`reloop.rs`) — complete for reducible CFGs

Implements Norman Ramsey's *"Beyond Relooper"* (ICFP 2022) dominator-tree
structuring, specialized to two facts true of every CFG this backend sees:

- **Back edges are exactly `Terminator::RecurJump`.** Clojure has no `goto`; the
  only cyclic control flow is `loop`/`recur`. So loop headers are precisely the
  `RecurJump` targets, and every `Jump`/`Branch` edge is forward.
- **The CFG is reducible.** Structured source + reducible-preserving inlining
  cannot manufacture irreducibility, so the relooper never needs node-splitting
  or a dispatch variable.

Output is a `Structured` tree:

```rust
pub enum Structured {
    Simple   { block, next },          // emit a block's straight-line body, then next
    Labeled  { label, body, next },    // wasm `block`; a forward Br(label) breaks to next
    Loop     { header, body },         // wasm `loop`; a backward Br(header) continues
    If       { cond, then_arm, else_arm },
    Br(BlockId),                       // forward break or backward continue
    Return(VarId), Unreachable, Nil,
}
pub enum RelooperError { Empty, DanglingTarget(BlockId), Irreducible { from, to } }
```

Algorithm (driven by a Cooper–Harvey–Kennedy dominator tree + reverse
postorder):

- Loop headers wrap their subtree in `Loop`; back edges become `Br`-continue.
- A **merge node** (≥2 forward predecessors) is emitted once, at its immediate
  dominator, inside a labeled `block`; branches to it become `Br`-break.
- Merge children of a node nest in **ascending reverse-postorder** (largest RPO
  outermost), which guarantees every `br` jumps *forward* out of enclosing
  blocks — the only direction wasm permits.
- All other forward edges target single-predecessor nodes and are inlined.

Each block is emitted exactly once. Irreducible control flow (which Clojure
cannot produce) is rejected via an RPO back-edge check.

### Emitter (`emit.rs`) — core complete, instruction set partial

Produces real, **`wasmparser`-validated** single-function modules.

- **Value model.** One boxed `i32` local per `VarId`; visible params are wasm
  locals `0..n`, the hidden trailing region param (if any) is next, and the
  remaining `VarId`s are declared locals. The signature is sized from
  `IrFunction::abi_param_count`.
- **Control flow.** The `Structured` tree maps directly: `Labeled`→`block`,
  `Loop`→`loop`, `If`→`if`/`else`, `Br`→`br N` with the depth resolved from a
  stack of enclosing control frames. A GC `rt_safepoint` is emitted at function
  entry and at each loop header.
- **SSA φ resolution.** No `phi` instruction is emitted. On each edge the φ's
  incoming value for that predecessor is copied into its local, using the
  operand stack for **parallel-move semantics** (all `local.get`s before any
  `local.set`s) so a swapping `recur` cannot clobber. Copies happen at the `Br`
  site for merge/loop targets and at block entry for inlined single-predecessor
  edges.
- **Module assembly.** `ModuleAsm` interns function types and `rt_abi` imports,
  then emits the type/import/function/export/code sections; the function is
  exported under its name. Imports occupy function indices `0..k`; the defined
  function is index `k`.

**Instructions lowered so far:** scalar constants (`nil`/`bool`/`long`/`double`/
`char`), `LoadLocal` (→ nil, matching the Cranelift backend), folded boxed
arithmetic (`+ - * / rem`), binary comparison (`= < > <= >=`), collection
allocation (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/`AllocCons`), region
operations (`RegionStart`/`RegionAlloc`/`RegionEnd`/`RegionParam`), and all
control flow. Everything else (calls including `CallWithRegion`, globals/vars,
string/keyword/symbol constants, async) returns `WasmError::Unsupported`.

### Region operations (`Region*`) — complete

The escape-analysis payoff: a region is a linear-memory bump arena and a region
handle is an `i32`, so region ops reuse the allocation machinery verbatim with
the handle threaded as a leading argument (mirrors
`codegen.rs::emit_region_alloc_collection`):

- `RegionStart`→`rt_region_start` (keep the `i32` handle), `RegionEnd`→
  `rt_region_end` (the bridge's `i32` result is dropped).
- `RegionAlloc`→`rt_region_alloc_*` with the handle as the leading `i32`,
  reusing the `rt_scratch_ptr` element-array marshalling; maps pass the **pair**
  count, cons passes its two pointers directly, and a degenerate cons falls back
  to nil.
- `RegionParam`→bind the **hidden trailing `i32` parameter**.  `emit_function`
  now sizes the wasm signature from `IrFunction::abi_param_count` (visible params
  + one trailing `i32` iff `takes_region_param`), reserves that param's local,
  and `RegionParam` copies it into its `VarId` local.  The
  `takes_region_param()`→`Unsupported` guard is gone.

`CallWithRegion` is lowered as of the **Calls** increment below (it resolves the
callee's wasm function index in a multi-function module).

### Allocation (`Alloc*`) — complete

The first lowering to touch linear memory. Element `*const Value` pointers are
marshalled into a runtime-provided scratch buffer, then the slice-taking
`rt_alloc_*` bridge is called (mirrors `codegen.rs::emit_alloc_collection`):

- The module **imports `"rt" "memory"`** the first time an allocation needs to
  store an element array (memory lives in its own index space, so it does not
  shift the function indices). `ModuleAsm::needs_memory` records this.
- `rt_scratch_ptr(n_bytes) -> i32` (new `rt_abi` bridge + `RT_IMPORTS` entry)
  hands back a thread-local, monotonically growing scratch buffer; the emitter
  stores its pointer in one extra `i32` local past the `VarId` locals.
- Each element is stored with `i32.store` at `scratch + i*4` (pointers are wasm
  `i32`s), then `bridge(scratch, count)` is called. For maps the pairs are
  flattened to `[k0,v0,…]` and `count` is the **pair** count.
- Empty literals pass a null pointer + zero count and need no memory.
- `AllocCons` takes two pointer args directly, no array.

### Calls and multi-function modules (`emit::emit_bundle`) — complete (closures deferred)

The first lowering that needs *more than one* function in a module — a
[`compile_bundle`](../crates/cljrs-compiler/src/wasm/mod.rs) over a slice of
[`IrFunction`]s (each top-level function plus its flattened `subfunctions`) into
a single module, so a direct call resolves its callee to a wasm function index:

- **Two-pass index assignment.** In wasm, imported functions occupy the low
  function-index space (`0..k`) and defined functions follow (`k..k+n`), so the
  import count `k` must be settled before any `call` to a *defined* function can
  be encoded. `emit_bundle` therefore lowers every body twice: pass 1 into a
  throwaway buffer purely to discover each body's `rt_abi` imports; pass 2 with
  `func_base = imports.len()` known, so `CallDirect` targets resolve to their
  final absolute indices. Emission is deterministic, so the import set is
  identical across passes. `emit_function` is now a one-element `emit_bundle`.
- **`CallDirect`** → push the argument locals, `call` the resolved index
  (mirrors `codegen.rs::emit_direct_call`). An unbundled callee reports
  `Unsupported`.
- **`CallWithRegion`** → same, plus the caller's region handle pushed as the
  hidden trailing argument (mirrors `emit_direct_call_with_extra`); the callee
  variant's `abi_param_count` already accounts for it.
- **`Call`** (dynamic) → marshal the argument `*const Value` pointers through the
  `rt_scratch_ptr` buffer and dispatch through `rt_call(callee, args_ptr, nargs)`
  (a zero-arg call passes a null pointer + zero count). This is the
  inline-cache-free path; `rt_call_ic` additionally needs a writable per-call-site
  IC slot in linear memory — the same data-segment coordination the
  string/keyword/symbol constants need (item 2), so it is deferred with them.

`AllocClosure` and cross-function tail calls landed in the **Closures** increment
below.

### Closures, the function table, and cross-function tail calls (`emit_alloc_closure`, `try_emit_tail_call`) — complete

The leftovers of the calls increment, now landed:

- **The function table.** A closure's arity-function pointer is, under `wasm32`,
  a **table index**. The module imports the runtime's shared indirect function
  table (`"rt" "__indirect_function_table"`, mirroring the imported `"rt"
  "memory"`) and installs every defined function into it with an active
  `funcref` element segment at `abi::FUNC_TABLE_BASE`. The function pointer for
  the defined function at bundle position `p` is `FUNC_TABLE_BASE + p` (the table
  *slot*, distinct from its wasm function *index* `func_base + p`, which is the
  element segment's *content*). `ModuleAsm::needs_table` records the import, and
  the table occupies its own index space so it does not shift function indices.
  The runtime must reserve `[FUNC_TABLE_BASE, …)` of its table — the table
  analogue of the rodata coordination the string constants need; the concrete
  base is finalized in the CLI/bundling step (item 5).
- **`AllocClosure`** → `rt_make_fn` (single fixed arity), `rt_make_fn_variadic`
  (single variadic), or `rt_make_fn_multi` (multi-arity), mirroring
  `codegen.rs::AllocClosure`. The closure name bytes, the captured-value pointer
  array, and (multi-arity) the fn-pointer / param-count / variadic-flag arrays
  are marshalled **contiguously through one `rt_scratch_ptr` reservation** at
  distinct, alignment-respecting offsets — sidestepping the data-segment /
  memory-layout coordination (item 2) by writing the constant name bytes into
  scratch at call time rather than into a rodata segment. Zero-arity closures
  fall back to nil; an arity function not in the bundle reports `Unsupported`.
- **Cross-function tail calls** (`return_call`). A block whose trailing
  instruction is a direct call (`CallDirect`/`CallWithRegion`) whose result is
  the function's return value becomes a `return_call` when
  `WasmBackend::tail_calls` is set — the callee's `[i32; abi_param_count] →
  [i32]` signature matches this function's result, and the caller's frame is
  reclaimed before the callee runs. With `tail_calls` off, or for dynamic `Call`s
  (dispatched through the `rt_call` import), the ordinary `call` + `return` is
  emitted (correct but not constant-stack; a trampoline is the deferred
  alternative). Tail calls are a default-enabled wasm proposal, so the emitted
  modules still validate.

`rt_call_ic` (the inline cache) remains deferred with the writable per-call-site
IC region: until that is coordinated with the runtime's memory layout,
`Inst::Call` keeps dispatching through plain `rt_call`.

### String / keyword / symbol constants (`emit_string_like`) — complete

`Const::Str` / `Const::Keyword` / `Const::Symbol` intern their UTF-8 bytes into a
**deduplicated read-only data pool** (`ModuleAsm::rodata` + `rodata_map`) and
resolve to the `(ptr, len)` pair `(abi::RODATA_BASE + offset, len)` passed to
`rt_const_string` / `_keyword` / `_symbol` (already in `RT_IMPORTS`). The pool is
emitted as a **single active data segment** at `abi::RODATA_BASE` in the
runtime's imported linear memory (the data section follows the code section in
wasm's section order), so the emitter owns a rodata region whose base the runtime
reserves — the linear-memory analogue of `FUNC_TABLE_BASE`, finalized against the
runtime's actual memory layout in the CLI/bundling step. The dedup map makes
interning idempotent across the two emission passes and collapses repeated
constants to one set of bytes. Keywords go through `rt_const_keyword` directly,
**not** the per-call-site inline cache `codegen.rs` uses — that IC is deferred
with the rest of the `rt_call_ic` work. Mirrors `codegen.rs::emit_string_const`
(the native backend defines an anonymous data object per string). The closure
name bytes still marshal through `rt_scratch_ptr`; moving them into this pool is
a cleanup left for later.

### Globals / vars (`LoadGlobal`/`LoadVar`/`DefVar`/`SetBang`) — complete

The first consumer of the rodata pool's name-as-data, mirroring
`codegen.rs::emit_load_global` / `emit_load_var` / `emit_def_var`:

- Each bridge takes `(ns_ptr, ns_len, name_ptr, name_len)`; a shared
  `push_name_args` helper interns both `ns` and `name` into the rodata pool
  (`intern_rodata`) and pushes the two `(ptr, len)` pairs.
- `LoadGlobal` → `rt_load_global` (binding value); `LoadVar` → `rt_load_var`
  (the Var object, for `set!`/`binding`); `DefVar` → `rt_def_var` with the boxed
  value pushed after the name args; `SetBang` → `rt_set_bang(var, val)`, whose
  `*const Value` result the IR has no destination for and so is `drop`ped.
- Versioned `name@sha` references make **no** wasm-side distinction: the runtime
  `rt_load_global` itself splits the version and resolves it (uncached). The
  per-call-site versioned IC the Cranelift backend uses
  (`rt_load_global_versioned_ic`) is deferred with the rest of the `rt_call_ic`
  inline-cache work, which still needs a writable per-site data slot.
- The four bridges were added to `RT_IMPORTS`.

### Exceptions (`Throw`, `KnownFn::TryCatchFinally`) — complete (thread-local path)

The boxed, backend-agnostic error path the Cranelift backend uses, so no
wasm-specific control flow is needed:

- `Throw(val)` → `rt_throw(val)`, which stashes the exception in a thread-local
  and returns nil (dropped). The throwing block then falls into its
  `unreachable`/return terminator; the enclosing `rt_try` checks the
  thread-local after the body runs (mirrors `codegen.rs`'s `Inst::Throw`).
- `KnownFn::TryCatchFinally` → a fixed three-arg `rt_try(body, catch, finally)`
  over the boxed thunks, special-cased ahead of the arithmetic/comparison match
  in `emit_known` (it is neither a fold nor a binary compare).
- `rt_throw` / `rt_try` were added to `RT_IMPORTS`.

The wasm exception-handling proposal (`try`/`catch`/`throw`, gated on
`WasmBackend::exceptions`) is a deferred **optimization**: the thread-local path
is always correct and is what runs today regardless of the flag. Wiring the EH
proposal would mean encoding tags + structured `try`/`catch` in the emitter and
relooper — a larger change with no correctness payoff over the thread-local
path.

### Unboxed scalar values (`typeinfer` + `refine_reprs`) — complete (intermediates)

The optimization payoff: intermediate `Long`/`Double`/`Bool` values live unboxed
in `i64`/`f64`/`i32` locals, so hot scalar arithmetic compiles to native wasm ops
instead of the heap-allocating boxed bridges.

- **Repr map.** `emit_one` runs `typeinfer::infer(func, &[])` (the same inference
  the Cranelift backend uses) then a wasm-private **`refine_reprs`** cleanup that
  transitively demotes back to `Boxed` any unboxed producer the emitter can't
  lower, so the map only marks a value unboxed when the emitter can *produce* it
  unboxed (keeping a value's repr and its local's wasm type in lock-step). Each
  declared local is typed by its repr.
- **Box/unbox at boundaries.** `Emitter::get` boxes an unboxed local on demand
  (`rt_const_long`/`_double`/`rt_box_bool`) wherever a boxed value is needed (call
  args, collection elements, `return`, boxed φ, the var bridges); `get_i64` /
  `get_f64` read unboxed operands (`f64.convert_i64_s` for a mixed long→double
  promotion); φ moves use a raw same-repr copy; an `if` reads a `Bool` directly as
  its `i32` and treats an unboxed number as constant-true.
- **Native arithmetic.** Binary `+`/`-`/`*`/`/` and comparisons whose result the
  map kept unboxed lower to `i64`/`f64` ops. Checked long `+`/`-` emit the
  signed-overflow branch (`((a^s)&(b^s))<0` / `((a^b)&(a^s))<0`) → `rt_overflow_error`
  + `rt_throw` + an early boxed-`nil` `return`, mirroring
  `codegen.rs::emit_long_overflow_check`. `rt_overflow_error` was added to
  `RT_IMPORTS`.
- **Demoted / deferred.** Checked long `*` (wasm has no `i64.mul_hi` for the
  128-bit overflow check) is demoted to the boxed `rt_mul`. The typed-parameter
  ABI (unboxed *params* aligned with `function_signature`) landed in the **Typed
  parameter ABI** increment below.

### Typed parameter ABI (`is_typed`, `emit_trampoline`) — complete

The optimization that aligns a function's wasm *signature* with its static
`^long`/`^double` parameter hints, so a hinted param arrives unboxed
(`i64`/`f64`) instead of as a boxed `i32` the body re-unboxes on every use:

- **Two functions per typed function.** A function with a non-boxed
  `seed_repr` (`is_typed`) compiles to a **typed body** — its signature is
  `function_signature(func)`, the hinted params are unboxed, and inference is
  seeded with `seed_reprs` so the param `VarId`s carry their unboxed repr — plus
  a boxed-entry **trampoline** (`emit_trampoline`) with the all-`i32` signature
  every dispatcher expects.
- **The trampoline is the primary entry.** It occupies the same wasm index a
  non-typed function would (exported under the function name, installed in the
  shared table, and the target of every `CallDirect`), so all the always-boxed
  dispatch paths — dynamic `rt_call`, indirect closure calls, cross-function
  direct calls — reach a typed function **unchanged**. The trampoline coerces
  each boxed argument (`rt_coerce_long`/`rt_coerce_double`, new `rt_abi`
  bridges), threads the hidden trailing region handle through verbatim, and
  `return_call`s the body (or plain `call`+return with `tail_calls` off). The
  typed bodies are appended after the `n` primaries, so primary indices, table
  slots, and exports are untouched.
- **Coerce, don't deopt.** The native backend's prologue guards each spec'd
  param's tag and returns the deopt sentinel on mismatch; AOT-wasm has no
  in-sandbox deopt seam, so a violated static hint **coerces** the number (or
  throws via the thread-local pending-exception slot for a non-number), matching
  Clojure's `longCast`/`doubleCast` semantics for type-hinted params.
- **`refine_reprs`** now takes a `keep_params` set so a typed param (which has no
  def site and would otherwise be demoted back to boxed) stays unboxed, keeping
  its repr and its wasm local type in lock-step.

**What remains (a follow-up optimization):** a same-bundle `CallDirect` whose
caller can supply the argument already unboxed could target the typed body
directly, skipping the trampoline's coerce round-trip for the caller-side win.
Today every direct call goes through the boxed trampoline (correct, and the
internal body is still unboxed); the caller-side path is left for later.

### CLI front-end (`compile_file_to_wasm`, `cljrs compile --target wasm`) — complete (module emission)

The driver that turns a source file into a `.wasm` artifact:

- **`aot::compile_file_to_wasm(src, out, src_dirs)`** lowers the entry namespace
  **and its transitively-required user namespaces** to a bundle of IR functions
  (`lower_file_to_ir_bundle`: parse → macro-expand → per-namespace discovery →
  ANF/region optimization), runs `optimize_direct_calls` on each (so same-unit
  calls bind to wasm function indices rather than dispatching through `rt_call`),
  then drives `wasm::compile_bundle(&refs, …)` over the entry function, each
  namespace initializer, and their flattened subfunctions, writing the validated
  bytes. A new `AotError::Wasm(WasmError)` surfaces backend errors.
- **CLI**: `cljrs compile <file> --target wasm -o <out>.wasm`. `--target`
  (default `native`) selects the backend; `--target wasm` with `--test` is
  rejected (no wasm test harness yet), and an unknown target errors cleanly.
- **End-to-end tests** (`crates/cljrs-compiler/tests/wasm_compile.rs`): a file of
  simple `defn`s, a `loop`/`recur` accumulator, and a program that `require`s a
  second user namespace each compile through the full front-end and **validate
  with `wasmparser`** — the cross-namespace test asserts the dependency's
  `__cljrs_ns_init_0` initializer is bundled + exported alongside `__cljrs_main`,
  and the loop test surfaced (and fixed) a φ parallel-move bug where a boxed φ
  destination with an unboxed `i64` entry was copied without boxing.

### Cross-namespace bundling + configurable layout (`lower_file_to_ir_bundle`, `abi::WasmLayout`) — complete

Two of the three "runtime linking + bundling" pieces are compiler-side and now
done:

- **Whole-program bundling.** `compile_file_to_wasm` mirrors `compile_file`'s
  per-namespace initializer discovery (`discover_bundled_sources` →
  `lower_namespace`): every transitively-`require`d user namespace the backend
  can lower becomes a `__cljrs_ns_init_N` function in the same module (its
  subfunctions flattened in). A namespace that cannot be lowered/codegen'd is
  **skipped**, left for the runtime's IR-interpreter tier — the same graceful
  degradation the native path uses. Subfunction names are globally unique
  (`GLOBAL_NAME_CTR`), so there are no collisions across the bundle.
- **Configurable bases.** `abi::WasmLayout { rodata_base, func_table_base }`
  (carried on `WasmBackend`, `Default` = the `0` placeholders) replaces the
  hardcoded `RODATA_BASE` / `FUNC_TABLE_BASE` constants throughout the emitter
  (the data-segment offset, the element-segment offset + table minimum, the
  string-constant `(ptr,len)` pairs, and the closure table slots). The
  CLI/linking step overrides them with the runtime's actually-reserved bases once
  that layout is known — *finalizing* the two placeholders the plan tracked. A
  unit test relocates both segments to a non-zero layout and reads the offsets
  back out of the validated module.

**What remains (the runtime-side step):** the emitted module's `"rt"` imports
(the `rt_abi` bridges, linear memory, the shared `__indirect_function_table`)
must be satisfied by the runtime compiled to `wasm32-unknown-unknown` (the
`cljrs-wasm` crate already builds the interpreter to that target). That requires
the runtime to **export** the `rt_abi` surface, its memory, and its table, then a
host (JS) that instantiates the runtime and instantiates the AOT module against
those exports — reserving `[rodata_base, …)` / `[func_table_base, …)` and passing
the concrete bases through `WasmLayout`. **Wiring the IR interpreter in as the
dynamic-code tier** (so a skipped namespace and `eval`/REPL still run) lands in
the same step. This is genuinely runtime-side integration, not a compiler
increment, and is the last open item.

### Tests

`cargo test -p cljrs-compiler wasm::` — 52 tests:

- `abi`: Value→wasm-type mapping, region contract well-typed, no duplicate
  imports.
- `reloop`: empty/single-return/linear-chain/diamond/loop-with-exit/nested
  sequential merges, each asserting **every block is emitted exactly once**.
- `emit`: an arithmetic function, an if/else diamond with a merge φ, a loop with
  a φ counter + conditional `recur`, collection allocation (a two-element
  vector exercising the scratch buffer + imported memory, an empty vector, a
  one-pair map, and a cons), and region operations (a region-parameterised
  callee variant binding the hidden trailing param, a function-scoped
  `RegionStart`/`RegionAlloc`/`RegionEnd`, and a region map + cons), and calls
  (a two-function bundle with a `CallDirect`, a bundle with a `CallWithRegion`
  threading the region handle into a region-parameterised variant, a dynamic
  `Call` through `rt_call` with and without arguments, and a bundle flattening a
  subfunction so a `CallDirect` to it resolves), closures (a single-arity closure
  capturing a value over a subfunction through the shared table, a variadic
  closure with no captures, a multi-arity closure marshalling the fn-pointer /
  param-count / variadic arrays + a capture into one scratch buffer, and a
  zero-arity-fallback-to-nil), and cross-function tail calls (a `CallDirect` in
  tail position emitting `return_call` with `tail_calls` on and an ordinary call
  with it off — asserting the `return_call` operator's presence/absence via a
  `wasmparser` operator scan — and a tail `CallWithRegion` threading the region
  handle), and string/keyword/symbol
  constants (a vector of a string + keyword + symbol exercising the rodata data
  segment, asserting the data section's presence, and a duplicate-string case
  asserting the deduplicated pool holds one copy of the bytes), and globals /
  vars (a `LoadGlobal` resolving a namespaced binding through `rt_load_global`
  with ns/name bytes from the rodata pool, and a `DefVar`/`LoadVar`/`SetBang`
  trio where the dropped `rt_set_bang` result leaves a balanced stack), and
  exceptions (a `Throw` stashing via `rt_throw` and falling into an `unreachable`
  terminator, and a `TryCatchFinally` lowering to the three-arg `rt_try` over its
  boxed thunks), and unboxed scalars (an `i64.lt_s` long comparison, an `f64.add`
  double addition, a loop accumulator whose `0`-seeded counter infers unboxed
  `Long` so the `+`s lower to checked `i64.add` while `(< i n)` against the boxed
  param stays on `rt_lt`, and a checked long `*` demoting to the boxed `rt_mul` —
  each asserting the expected operator's presence/absence via `wasmparser`), and
  the **typed parameter ABI** (a `^long` and a `^double` param each emitting a
  trampoline + typed body — asserting two defined functions and the
  `rt_coerce_long`/`rt_coerce_double` import — the trampoline's `return_call`
  toggling with `tail_calls`, a non-typed function staying a single boxed body,
  and a boxed caller reaching a typed callee through its trampoline), and the
  **configurable layout** (a non-default `WasmLayout` relocating the data and
  element segments, read back out of the validated module) — each **validated
  with `wasmparser`**; plus end-to-end CLI coverage in `tests/wasm_compile.rs` (a
  `defn` file, a `loop`/`recur` accumulator, and a program that `require`s a
  second namespace — asserting the dependency's `__cljrs_ns_init_0` initializer is
  bundled + exported — each driven through `compile_file_to_wasm` and
  `wasmparser`-validated), and `Unsupported` coverage for an unbundled
  direct-call target, an out-of-bundle closure arity, and un-lowered
  instructions, and the closure-constructor import / typed-signature accounting.

Dependencies added to `cljrs-compiler`: `wasm-encoder = "0.244"` (dep),
`wasmparser = "0.244"` (dev-dep) — both already in the lock transitively.

---

## What is next

Ordered by value. Allocation, region operations, calls / multi-function modules,
closures + the function table + cross-function tail calls, string / keyword /
symbol constants, globals / vars, exceptions, unboxed scalar values, the
**typed parameter ABI**, the **CLI front-end** (`cljrs compile --target wasm`),
and **cross-namespace bundling + the configurable memory/table layout**
(`abi::WasmLayout`) are now **done** — see those sections above. The one
remaining item is runtime-side: making the emitted module *runnable* by linking
it against the `wasm32-unknown-unknown` runtime.

### 1. Runtime linking (the last, runtime-side step)

The compiler now emits a whole-program AOT module
(`compile_file_to_wasm`: entry namespace + every lowerable required namespace's
initializer) whose memory/table bases are `WasmLayout`-configurable. What remains
is genuinely **runtime-side**, not a compiler increment:

- **Make the runtime export the `"rt"` surface.** The emitted module imports the
  `rt_abi` bridges, linear memory, and the shared `__indirect_function_table`
  from module `"rt"`. The `cljrs-wasm` crate builds the interpreter to
  `wasm32-unknown-unknown`, but does not yet *export* the `rt_abi` functions (they
  live in `cljrs-compiler::rt_abi`, a host-side crate) or reserve table/memory
  regions for the AOT bundle. Exposing them — and a host (JS) that instantiates
  the runtime, then instantiates the AOT module against the runtime's exports,
  reserving `[rodata_base, …)` / `[func_table_base, …)` and passing the concrete
  bases through `WasmLayout` — is the open work.
- **Wire the IR interpreter in as the dynamic-code tier** (drop the JIT/OSR hooks
  in-sandbox) so `eval`/REPL/freshly-required namespaces — and any namespace the
  bundler skipped — still run.

### Deferred indefinitely

- **WasmGC** — swap the linear-memory GC for host-managed reference types. A
  real project on its own; keep the linear-memory GC.
- **In-browser JIT** — runtime wasm-generation + `WebAssembly.instantiate`. Only
  worthwhile as a tiering step *on top of* AOT, once AOT is solid.

---

## Pointers for the next implementer

- The relooper output is the emitter's contract — read `reloop.rs`'s module doc
  and the `Structured` variants first.
- `abi.rs` is the source of truth for the value/region ABI; extend `RT_IMPORTS`
  there when a new `rt_abi` bridge is needed, then call it via
  `Emitter::call_import(name)`.
- Mirror semantics from `codegen.rs` (the Cranelift backend) — it is the
  reference for what each `Inst` must do. Search for the matching
  `translate_inst` / `emit_*` arm.
- Validate every new shape with `wasmparser` in a unit test (see
  `emit::tests::validate`). A module that validates is structurally correct
  wasm even without a JS runtime to execute it.
- `TODO.md` Phase 11.5 tracks the checklist; keep it and the `cljrs-compiler`
  README in sync (per `CLAUDE.md`, READMEs are updated in the same commit).
```
