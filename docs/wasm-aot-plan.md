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
  locals `0..n`, remaining `VarId`s are declared locals.
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
allocation (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/`AllocCons`), and all
control flow. Everything else returns `WasmError::Unsupported`.

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

### Tests

`cargo test -p cljrs-compiler wasm::` — 20 tests:

- `abi`: Value→wasm-type mapping, region contract well-typed, no duplicate
  imports.
- `reloop`: empty/single-return/linear-chain/diamond/loop-with-exit/nested
  sequential merges, each asserting **every block is emitted exactly once**.
- `emit`: an arithmetic function, an if/else diamond with a merge φ, a loop with
  a φ counter + conditional `recur`, and collection allocation (a two-element
  vector exercising the scratch buffer + imported memory, an empty vector, a
  one-pair map, and a cons) — each **validated with `wasmparser`**; plus
  `Unsupported` coverage for region variants and un-lowered instructions, and the
  typed-signature accounting.

Dependencies added to `cljrs-compiler`: `wasm-encoder = "0.244"` (dep),
`wasmparser = "0.244"` (dev-dep) — both already in the lock transitively.

---

## What is next

Ordered by value. Allocation (the gateway item, now **done** — see the
"Allocation" section above) made real collection-building functions compilable
and introduced the linear-memory import + scratch-buffer machinery that the
region path reuses.

### 1. Region operations

Now that the scratch/memory machinery exists, region ops are mechanical and
unlock the escape-analysis payoff. Lower `RegionStart`/`RegionAlloc`/`RegionEnd`
to the `rt_region_*` imports, and `RegionParam`/`CallWithRegion` to the hidden
trailing-`i32` ABI (see `abi.rs`). Then drop the
`takes_region_param()`→`Unsupported` guard in `emit_function`. This needs
multi-function support (item 2) for `CallWithRegion`.

### 2. Calls and multi-function modules

`CallDirect` (same-module direct call), `Call` (dynamic, via the
`rt_call_ic` inline-cache bridge), `CallWithRegion`. Requires compiling a
**bundle** of functions into one module: extend `compile_function` to a
`compile_bundle(&IrBundle)` that assigns a wasm function index to each, emits all
bodies, and resolves `CallDirect` targets to those indices. Closures
(`AllocClosure`) and subfunctions follow the same shape as the Cranelift backend
(declare all arity functions into the module, materialize closure values via
`rt_make_fn*`). Cross-function tail calls use the wasm tail-call proposal
(`return_call`) when `WasmBackend::tail_calls`, else a trampoline.

### 3. Constants needing a data segment

`Const::Str` / `Const::Keyword` / `Const::Symbol`. Emit the UTF-8 bytes into a
**data segment** and pass `(ptr, len)` to `rt_const_string`/`_keyword`/`_symbol`
(already in `RT_IMPORTS`). The data segment's memory placement must be
coordinated with the runtime's linear-memory layout — decide whether the
emitter owns a rodata region or the runtime reserves one.

### 4. Globals / vars

`LoadGlobal` / `LoadVar` / `DefVar` / `SetBang`. These resolve namespaced names;
follow `codegen.rs::emit_load_global` / `emit_def_var`. Needs the name-as-data
machinery from (3) and the var-resolution `rt_abi` bridges added to `RT_IMPORTS`.

### 5. Exceptions

`Throw` / `KnownFn::TryCatchFinally`. Use the wasm exception-handling proposal
(`try`/`catch`/`throw`) when `WasmBackend::exceptions`, else thread the
`rt_abi` thread-local error path the Cranelift backend uses (`rt_throw` +
`rt_try` checking the thread-local).

### 6. Unboxed specialization

Align the emitted signature with `function_signature` (typed ABI): keep
`Long`/`Double` values unboxed in `i64`/`f64` locals per `typeinfer`, guarding
specialized params and boxing only at boxed-context uses. Mirrors
`codegen.rs::compile_function_with_specs`. This is an optimization, not a
correctness requirement — do it after the functional subset is broad.

### 7. CLI + bundling

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
