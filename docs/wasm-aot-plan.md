# Plan: AOT Clojure → WebAssembly (browser backend)

## Overview

This is the handoff document for the **AOT Clojure → WebAssembly backend** — a
second code-generation backend over the same `cljrs-ir` IR as the Cranelift
backend, targeting native-fast, sandbox-safe deployment in the browser.

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
  marshalling buffer, GC-heap + region allocation, inline-cache/deopt bridges)
  as `(name, params, results)` wasm function types. `lookup(name)` resolves one.
  Completing it to all of `rt_abi` is mechanical — one `RtImport` per `extern
  "C"` signature.

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

`AllocClosure` is **still** `Unsupported`: materializing a closure value calls
`rt_make_fn*` with the arity function's *pointer*, which under `wasm32` is a
**table index**, so it needs a function table + `ref.func` element segment, plus
the closure name in a data segment (item 2). Cross-function tail calls
(`return_call`) are likewise deferred.

### Tests

`cargo test -p cljrs-compiler wasm::` — 28 tests:

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
  subfunction so a `CallDirect` to it resolves) — each **validated with
  `wasmparser`**; plus `Unsupported` coverage for an unbundled direct-call target
  and un-lowered instructions, and the typed-signature accounting.

Dependencies added to `cljrs-compiler`: `wasm-encoder = "0.244"` (dep),
`wasmparser = "0.244"` (dev-dep) — both already in the lock transitively.

---

## What is next

Ordered by value. Allocation, region operations, and calls / multi-function
modules (now **done** — see those sections above) made real collection-building,
arena-allocating, and cross-function-calling programs compilable; calls reuse the
multi-function `emit_bundle` and the scratch-buffer marshalling the allocation
item introduced.

### 1. Closures, the function table, and cross-function tail calls

The leftovers of the calls increment, all blocked on machinery the next two
items introduce. `AllocClosure` materializes a closure via `rt_make_fn*`, which
takes the arity function's *pointer* — under `wasm32` a **table index** — so it
needs a function table populated by a `ref.func` element segment and the closure
name in a data segment (item 2). Cross-function tail calls use the wasm tail-call
proposal (`return_call`) when `WasmBackend::tail_calls`, else a trampoline. The
`rt_call_ic` inline cache (a writable per-call-site IC slot) lands with the
data-segment work too; until then `Call` dispatches through plain `rt_call`.

### 2. Constants needing a data segment

`Const::Str` / `Const::Keyword` / `Const::Symbol`. Emit the UTF-8 bytes into a
**data segment** and pass `(ptr, len)` to `rt_const_string`/`_keyword`/`_symbol`
(already in `RT_IMPORTS`). The data segment's memory placement must be
coordinated with the runtime's linear-memory layout — decide whether the
emitter owns a rodata region or the runtime reserves one.

### 3. Globals / vars

`LoadGlobal` / `LoadVar` / `DefVar` / `SetBang`. These resolve namespaced names;
follow `codegen.rs::emit_load_global` / `emit_def_var`. Needs the name-as-data
machinery from (2) and the var-resolution `rt_abi` bridges added to `RT_IMPORTS`.

### 4. Exceptions

`Throw` / `KnownFn::TryCatchFinally`. Use the wasm exception-handling proposal
(`try`/`catch`/`throw`) when `WasmBackend::exceptions`, else thread the
`rt_abi` thread-local error path the Cranelift backend uses (`rt_throw` +
`rt_try` checking the thread-local).

### 5. Unboxed specialization

Align the emitted signature with `function_signature` (typed ABI): keep
`Long`/`Double` values unboxed in `i64`/`f64` locals per `typeinfer`, guarding
specialized params and boxing only at boxed-context uses. Mirrors
`codegen.rs::compile_function_with_specs`. This is an optimization, not a
correctness requirement — do it after the functional subset is broad.

### 6. CLI + bundling

`cljrs compile <file> --target wasm -o <out>.wasm`. Drive `compile_bundle` over a
whole program, link with the runtime compiled to `wasm32-unknown-unknown` (the
`cljrs-wasm` crate already builds the interpreter to that target), and wire the
**IR interpreter into the bundle as the dynamic-code tier** (drop the JIT/OSR
hooks in-sandbox).

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
