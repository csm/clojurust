# The WebAssembly AOT Backend

A complete guide to clojurust's ahead-of-time WebAssembly code generator: why it
exists, how it is built, what each piece does, and how to use it.

This is the *reference guide*. For the increment-by-increment build log and the
open task list, see [`wasm-aot-plan.md`](./wasm-aot-plan.md); for the per-symbol
API surface, see the [`cljrs-compiler` README](../crates/cljrs-compiler/README.md).

---

## 1. Why a WebAssembly backend at all

clojurust already has two ways to run Clojure: a tree-walking interpreter and a
native code generator. The native path tiers up at runtime —

```text
  native:   tree-walk → IR-interp → JIT/OSR   (peak, reached while running)
```

— and the top two tiers lean on **generating machine code at runtime**. The
Cranelift JIT (`cljrs-jit`) `mmap`s an executable page and writes native
instructions into it.

That is exactly what a WebAssembly sandbox forbids. There is no
`mmap(PROT_EXEC)` inside a wasm module: a module cannot generate and then execute
fresh machine code. So the in-sandbox story has to be **ahead-of-time** — compile
each Clojure function to wasm bytecode at *build* time and ship it; the browser's
own engine JITs *that* to native.

The tiers therefore **invert** relative to native:

```text
  browser:  tree-walk → IR-interp     (dynamic: eval, REPL, freshly-required ns, macros)
            AOT-wasm                   (peak, frozen at build time; the browser JITs it)
```

The IR interpreter (`cljrs-eval::ir_interp`) stays **on board** the wasm bundle
as the dynamic-code tier; AOT-wasm is the frozen top tier. No in-sandbox JIT or
on-stack-replacement hooks are installed.

This single hard constraint — *no runtime codegen* — is the reason the WebAssembly
backend is a separate code generator rather than a wasm target for the existing
JIT.

---

## 2. What is reused, and what is new

The crucial design lever is that **everything upstream of code generation is
backend-agnostic**. The WebAssembly backend is a *second* consumer of the same
`cljrs-ir` IR that Cranelift consumes:

| Shared, reused unchanged                     | Where                                   |
|----------------------------------------------|-----------------------------------------|
| ANF / SSA lowering                           | `cljrs_ir::lower`                       |
| Escape analysis + region inference           | `cljrs_ir::lower::{escape, regionalize}`|
| Scalar representation inference              | `cljrs_compiler::typeinfer`             |
| The `rt_abi` runtime-bridge contract         | `cljrs_compiler::rt_abi`                |

Because escape analysis and region promotion are **properties of the IR**
(`Inst::Region*`, `IrFunction::takes_region_param`), bump allocation comes along
for free: a region is a linear-memory arena, a region handle is an `i32` offset,
and a region-parameterised function takes that `i32` as a hidden trailing
parameter. Nothing wasm-specific is needed to get arenas.

Only two things are genuinely new and wasm-specific:

1. **Relooping** — recovering structured control flow from the IR's arbitrary
   CFG, because wasm has only `block`/`loop`/`if` + labelled `br`, not `goto`.
2. **The `wasm-encoder` emitter** — walking that structured tree and lowering
   each `Inst` to bytecode, with the `rt_abi` surface declared as wasm imports.

Both live in `crates/cljrs-compiler/src/wasm/`, and nowhere else — Cranelift
wants the raw CFG and would be *pessimised* by re-structuring, so the relooper is
deliberately wasm-private.

---

## 3. Map of the code

```text
crates/cljrs-compiler/src/wasm/
  mod.rs      — public API (compile_function, compile_bundle, WasmBackend, WasmError); the tier model
  abi.rs      — the ABI contract: Value→i32, the rt_abi import table, region threading, WasmLayout
  reloop.rs   — the relooper: IR CFG → structured control flow; wasm-private
  emit.rs     — the wasm-encoder emitter: IrFunction(s) → a validated wasm module
```

The public entry points (`mod.rs`):

```rust
pub fn compile_function(func: &IrFunction, cfg: &WasmBackend) -> Result<Vec<u8>, WasmError>;
pub fn compile_bundle(funcs: &[&IrFunction], cfg: &WasmBackend) -> Result<Vec<u8>, WasmError>;

pub struct WasmBackend {
    pub tail_calls: bool,        // use the tail-call proposal (return_call)
    pub exceptions: bool,        // (reserved) use the exception-handling proposal
    pub layout: abi::WasmLayout, // memory/table base addresses (see §4.4)
}

pub enum WasmError {
    Reloop(reloop::RelooperError), // control flow couldn't be structured
    Unsupported(String),           // an IR construct not yet lowered
    Unimplemented(&'static str),   // a scaffolded path
}
```

`compile_function` is the single-function special case of `compile_bundle`. The
pipeline is always: **`reloop::reloop(func)` → `emit::emit_bundle`**.

---

## 4. The ABI contract (`abi.rs`)

`abi.rs` is the canonical, *data-only* description of how Clojure values, the
runtime bridge, and regions map onto WebAssembly. It deliberately has no
`wasm-encoder` dependency, so the contract is reviewable and unit-testable
independently of the encoder that consumes it.

### 4.1 The value model

Under `wasm32`, **every pointer is an `i32`** (a linear-memory offset):

| IR / `rt_abi` Rust type          | wasm type | Meaning                              |
|----------------------------------|-----------|--------------------------------------|
| `*const Value` (`Repr::Boxed`)   | `i32`     | offset of the boxed value            |
| `*mut Region` (region handle)    | `i32`     | offset of the `Region` arena         |
| `*const *const Value` (slices)   | `i32`     | offset of a contiguous pointer array |
| `i64` (`Repr::Long`)             | `i64`     | unboxed long payload                 |
| `f64` (`Repr::Double`)           | `f64`     | unboxed double payload               |
| `u8`/`u32` (`Repr::Bool`, tags)  | `i32`     | small integers                       |

`WasmValType::for_repr(Repr)` encodes this. Because every pointer collapses to an
`i32`, the **entire ~165-function `rt_abi` surface is expressible as wasm imports
with no marshalling beyond width changes**. The GC heap and all regions live in
the module's single linear memory.

The default value model is **boxed-only**: every IR `VarId` is, by default, a
wasm `i32` local holding a boxed `*const Value`. This is the universal
representation, always correct. Unboxing (§5.10, §5.11) is layered on top as an
optimisation — never a correctness requirement.

### 4.2 The `rt_abi` imports

The emitted module imports its runtime bridge from the module named `"rt"`.
`RT_IMPORTS: &[RtImport]` is the wired subset, each entry a `(name, params,
results)` wasm function type; `abi::lookup(name)` resolves one and the emitter
calls it via `Emitter::call_import(name)`. The table covers safepoints, constant
materialisation, the boxed and unboxed arithmetic/comparison bridges, the
`rt_scratch_ptr` marshalling buffer, GC-heap and region allocation, `rt_call`,
the `rt_make_fn*` closure constructors, the global/var bridges, the exception
path, and the specialisation bridges (`rt_value_tag`, `rt_unbox_long/double`,
`rt_coerce_long/double`, `rt_box_bool`).

Extending the backend to a new bridge is mechanical: add one `RtImport` mirroring
the `extern "C"` signature, then call it by name.

### 4.3 Regions (bump allocation, for free)

Region inference runs host-side at lowering time, identical for both backends.
The wasm realisation:

- A **region handle** is an `i32`, the offset of a `Region` arena (`(base, bump,
  limit)` in linear memory). `RegionStart` → `rt_region_start` (keep the handle);
  `RegionEnd` → `rt_region_end`.
- `RegionAlloc` → the matching `rt_region_alloc_*`, with the handle threaded as
  the **leading** `i32` argument, reusing the same `rt_scratch_ptr` element
  marshalling as the GC-heap allocators.
- `RegionParam` → bind the function's **hidden trailing `i32` parameter**, present
  iff `IrFunction::takes_region_param()`. The compiled signature of a
  region-parameterised variant is its visible params plus that one trailing `i32`
  — exactly mirroring `IrFunction::abi_param_count()` on the native side.
- `CallWithRegion` → an ordinary direct call passing the caller's handle as the
  trailing argument.

### 4.4 Where the module meets the runtime: `WasmLayout`

The emitted module installs two things at addresses the runtime must agree on:

- a **read-only data pool** (string/keyword/symbol bytes) at `rodata_base` in the
  runtime's imported linear memory;
- its **functions** at `func_table_base` in the runtime's imported indirect
  function table.

```rust
pub struct WasmLayout {
    pub rodata_base: u32,      // base offset of the rodata pool in linear memory
    pub func_table_base: u32,  // base slot of the AOT functions in the table
}
```

`WasmLayout` rides on `WasmBackend`. `Default` uses the `0` *validation-time
placeholders* (`abi::RODATA_BASE` / `abi::FUNC_TABLE_BASE`) — a validated module
is structurally correct at any base. When the module is linked against the real
runtime, the CLI/linking step passes the runtime's *actually-reserved* bases
here, finalising the placeholders. (See §8.)

---

## 5. The emitter (`emit.rs`)

The emitter turns a relooped function into validated wasm bytecode. Everything it
produces is checked with `wasmparser` in a unit test; a module that validates is
structurally correct wasm even without a JS runtime to execute it.

### 5.0 The relooper, first

wasm has no `goto`. The relooper (`reloop.rs`) recovers structured control flow
from the IR CFG using Norman Ramsey's *"Beyond Relooper"* (ICFP 2022)
dominator-tree structuring, specialised to two facts true of every CFG this
backend sees:

- **Back edges are exactly `Terminator::RecurJump`.** Clojure has no `goto`; the
  only cyclic control flow is `loop`/`recur`. So loop headers are precisely the
  `RecurJump` targets, and every `Jump`/`Branch` edge is forward.
- **The CFG is reducible.** Structured source + reducible-preserving inlining
  cannot manufacture irreducibility, so the relooper never needs node-splitting
  or a dispatch variable.

The output is a `Structured` tree the emitter walks directly:

```rust
pub enum Structured {
    Simple   { block, next },        // straight-line body, then next
    Labeled  { label, body, next },  // wasm `block`; a forward Br(label) breaks to next
    Loop     { header, body },       // wasm `loop`; a backward Br(header) continues
    If       { cond, then_arm, else_arm },
    Br(BlockId),                     // forward break or backward continue
    Return(VarId), Unreachable, Nil,
}
```

A **merge node** (≥2 forward predecessors) is emitted once, at its immediate
dominator, inside a labelled `block`; branches to it become `Br`-break. Merge
children nest in ascending reverse-postorder so every `br` jumps *forward* out of
its enclosing blocks — the only direction wasm permits. Each block is emitted
exactly once. Irreducible control flow (which Clojure cannot produce) is rejected
via an RPO back-edge check.

### 5.1 Per-function locals and signature

For each function the emitter assigns wasm locals: the visible params occupy
locals `0..nparams`, the hidden region param (if any) is next, then the remaining
`VarId`s become declared locals typed by their representation, then one trailing
`i32` scratch local for argument marshalling. The signature is sized from
`IrFunction::abi_param_count()`.

### 5.2 Control flow

The `Structured` tree maps one-to-one onto wasm: `Labeled` → `block`, `Loop` →
`loop`, `If` → `if`/`else`, `Br N` → `br N` with the depth resolved from a stack
of enclosing control frames. A GC `rt_safepoint` is emitted at function entry and
at each loop header (so it runs on every back-edge).

### 5.3 SSA φ resolution

No `phi` instruction is ever emitted. On each edge, the φ's incoming value for
that predecessor is copied into the φ's local, using the operand stack for
**parallel-move semantics** — all `local.get`s before any `local.set`s — so a
swapping `recur` (`(recur b a)`) cannot clobber. Copies happen at the `Br` site
for merge/loop targets and at block entry for inlined single-predecessor edges.
When the φ destination is boxed but a source is unboxed, the source is boxed
first; when the destination is unboxed, representation inference guarantees every
source shares that repr, so a raw copy is type-correct.

### 5.4 Allocation (`Alloc*`)

The first lowering to touch linear memory. Element `*const Value` pointers are
marshalled into a runtime-provided scratch buffer (`rt_scratch_ptr(n_bytes) →
i32`, a thread-local, monotonically growing buffer), then the slice-taking
`rt_alloc_*` bridge is called. The module imports `"rt" "memory"` the first time
an allocation needs to store an element array. Maps flatten their pairs to
`[k0,v0,…]` and pass the *pair* count; `AllocCons` takes its two pointers
directly; empty literals pass a null pointer + zero count and need no memory.

### 5.5 Region operations

The escape-analysis payoff in practice: region ops reuse the allocation machinery
verbatim, with the handle threaded as a leading argument (see §4.3). `emit_one`
sizes the signature from `abi_param_count`, reserves the trailing region param's
local, and `RegionParam` copies it into its `VarId`.

### 5.6 Calls and multi-function modules

In wasm, imported functions occupy the low function-index space (`0..k`) and
defined functions follow (`k..`). So the import count `k` must be settled before
any `call` to a *defined* function can be encoded. `emit_bundle` therefore runs
**two passes**: pass 1 lowers every body into a throwaway buffer purely to
discover its `rt_abi` imports; pass 2 re-lowers with `func_base = imports.len()`
known, so `CallDirect` targets resolve to final absolute indices. Emission is
deterministic, so the import set is identical across passes.

- **`CallDirect`** → push the argument locals, `call` the resolved index.
- **`CallWithRegion`** → the same, plus the caller's region handle as the hidden
  trailing argument.
- **`Call`** (dynamic) → marshal the argument pointers through the scratch buffer
  and dispatch through `rt_call(callee, args_ptr, nargs)`.

### 5.7 Closures and the function table

A closure's arity-function pointer is, under `wasm32`, a **table index**. The
module imports the runtime's shared `"rt" "__indirect_function_table"` and
installs every function's *primary* entry into it with an active `funcref`
element segment at `func_table_base`. The function pointer for the function at
bundle position `p` is `func_table_base + p` (the table *slot*, distinct from its
wasm function *index*, which is the element segment's *content*).

`AllocClosure` lowers to `rt_make_fn` (fixed arity), `rt_make_fn_variadic`, or
`rt_make_fn_multi` (multi-arity), with the closure name bytes, the captured-value
array, and (multi-arity) the fn-pointer / param-count / variadic arrays
marshalled contiguously through one `rt_scratch_ptr` reservation.

### 5.8 Cross-function tail calls

A block whose trailing instruction is a direct call whose result *is* the
function's return value becomes a `return_call` when `WasmBackend::tail_calls` is
set — the caller's frame is reclaimed before the callee runs, giving
constant-stack mutual recursion. With `tail_calls` off (or for dynamic `Call`s,
dispatched through `rt_call`), an ordinary `call` + `return` is emitted: correct,
but not constant-stack. Tail calls are a default-enabled wasm proposal, so the
emitted modules still validate.

### 5.9 Constants, globals, exceptions

- **String/keyword/symbol constants** intern their UTF-8 bytes into a
  **deduplicated read-only data pool** emitted as one active data segment at
  `rodata_base`. A constant resolves to the `(ptr, len)` pair `(rodata_base +
  offset, len)` passed to `rt_const_string`/`_keyword`/`_symbol`.
- **Globals / vars** (`LoadGlobal`/`LoadVar`/`DefVar`/`SetBang`) lower to the
  matching `rt_*` bridges, drawing the `(ns, name)` byte pairs from the same
  rodata pool.
- **Exceptions** (`Throw`, `KnownFn::TryCatchFinally`) use the boxed,
  backend-agnostic thread-local error path: `rt_throw` stashes the exception in a
  thread-local (its nil result dropped) and the block falls into its terminator;
  `rt_try(body, catch, finally)` runs the body thunk, routes a pending exception
  into the catch thunk, and always runs the finally thunk. The wasm
  exception-handling proposal (gated on `WasmBackend::exceptions`) is a deferred
  alternative — the thread-local path is always correct and is what runs today.

### 5.10 Unboxed scalar intermediates

`typeinfer::infer` assigns each `VarId` an unboxed `Repr` (`Long`→`i64`,
`Double`→`f64`, `Bool`→`i32` 0/1) wherever the boxed bridge's exact semantics
survive on the raw representation, so intermediate arithmetic compiles to native
`i64`/`f64` ops instead of the heap-allocating `rt_*` bridges. Values are **boxed
only where they flow into a boxed context** — a call argument, a collection
element, a `return`, a boxed φ, a var bridge — through `Emitter::get`; unboxed
operands are read with `get_i64`/`get_f64`.

Checked long `+`/`-` emit the same signed-overflow branch the native backend does
(`rt_overflow_error` + `rt_throw`, then an early boxed-`nil` return). A wasm-private
`refine_reprs` pass **demotes back to boxed**, transitively, any unboxed producer
the emitter cannot lower (e.g. checked long `*`, which needs a 128-bit overflow
check wasm cannot express without `i64.mul_hi`), so a value's repr and its local's
wasm type always stay in lock-step.

### 5.11 The typed parameter ABI

`refine_reprs` keeps *intermediates* unboxed but, by default, leaves **parameters
boxed** — the signature stays all-`i32`, because the always-boxed dispatchers
(`rt_call`, the indirect function table, cross-function `CallDirect`) cannot
supply unboxed arguments.

A function with static `^long`/`^double` parameter hints (`seed_reprs`, detected
by `is_typed`) bridges that gap by compiling to **two** wasm functions:

```text
   ┌─────────────────────────┐      coerce args, then (return_)call
   │  trampoline (primary)   │ ───────────────────────────────────────┐
   │  signature: all i32     │                                        │
   │  - rt_coerce_long(arg0) │                                        ▼
   │  - pass arg1 (boxed)    │                          ┌──────────────────────────┐
   │  - pass region handle   │                          │  typed body              │
   └─────────────────────────┘                          │  signature: i64, i32, …  │
        ▲ export, table slot, every CallDirect          │  hinted params unboxed   │
                                                         └──────────────────────────┘
```

- The **typed body**'s signature is `function_signature(func)`: the hinted params
  arrive unboxed, so the body reads them with no per-use unbox. Inference is
  seeded with `seed_reprs`, and `refine_reprs`'s `keep_params` set stops those
  (def-less) params from being demoted back to boxed.
- The **trampoline** has the all-`i32` boxed signature every dispatcher expects.
  It is the function's *primary* entry — exported under the function name,
  installed in the shared table, and the target of every `CallDirect` — so all
  the always-boxed dispatch paths reach a typed function unchanged. It coerces
  each boxed `^long`/`^double` argument (`rt_coerce_long`/`rt_coerce_double`),
  threads the region handle through, and `return_call`s the typed body.
- Typed bodies are appended after the `n` primaries, so primary indices, table
  slots, and exports are untouched.

There is no in-sandbox **deopt seam** (the native backend's specialised prologue
guards a param's tag and returns the deopt sentinel on mismatch; the dispatch
seam then re-runs at Tier 1). AOT-wasm cannot deopt, so a violated static hint
**coerces or throws** instead, matching Clojure's `longCast`/`doubleCast`
semantics: a `Long` passes through, a `Double` truncates/widens, a non-number
raises a cast exception via the thread-local pending-exception slot.

> A follow-up optimisation, not yet done: a same-bundle `CallDirect` whose caller
> already holds an argument unboxed could target the typed body *directly*,
> skipping the trampoline's coerce round-trip. Today every direct call goes
> through the boxed trampoline (correct; the body is still unboxed internally).

### 5.12 What the emitter does *not* yet lower

`Inst::Call` always dispatches through plain `rt_call`; the per-call-site inline
cache (`rt_call_ic`) needs a writable IC slot in linear memory and is deferred.
The async poll-function ABI (state-machine params) returns
`WasmError::Unsupported`.

---

## 6. The Cranelift backend is the reference

When in doubt about *what* an `Inst` must do, the Cranelift backend
(`codegen.rs`) is the source of truth — the wasm emitter mirrors its semantics
arm for arm. The mapping is consistent:

| Concept                | Cranelift (`codegen.rs`)              | wasm (`emit.rs`)                      |
|------------------------|---------------------------------------|---------------------------------------|
| value                  | `*const Value` / unboxed `i64`/`f64`  | `i32` / `i64`/`f64`                   |
| direct call            | `emit_direct_call`                    | `CallDirect` → `call`                 |
| region alloc           | `emit_region_alloc_collection`        | `RegionAlloc` → `rt_region_alloc_*`   |
| long overflow check    | `emit_long_overflow_check`            | the `((a^s)&(b^s))<0` branch          |
| typed-param prologue   | `compile_function_with_specs` + guard | the trampoline + coercion             |
| deopt on guard failure | `rt_deopt` sentinel                   | coerce-or-throw (no deopt seam)       |

The one structural difference is control flow: Cranelift consumes the raw CFG;
the wasm backend reloops it first.

---

## 7. The CLI front-end and whole-program bundling

`cljrs compile <file> --target wasm -o <out>.wasm` drives the backend.
`--target` defaults to `native`; `--target wasm` with `--test` is rejected (there
is no wasm test harness yet), and an unknown target errors cleanly.

Under the hood, `aot::compile_file_to_wasm`:

1. Calls `lower_file_to_ir_bundle`, which boots a full environment, parses and
   macro-expands the entry file, and **discovers every transitively-`require`d
   user namespace** (`discover_bundled_sources` → `lower_namespace`, mirroring the
   native `compile_file`). The result is a bundle of IR functions: the entry
   `__cljrs_main` followed by one `__cljrs_ns_init_N` initializer per namespace
   the backend can lower.
2. Runs `optimize_direct_calls` on each function so same-unit calls bind to wasm
   function indices rather than dispatching through `rt_call`.
3. Drives `wasm::compile_bundle` over the whole bundle (each function plus its
   flattened subfunctions) and writes the `wasmparser`-validated bytes.

A namespace the backend cannot lower is **skipped**, left for the runtime's
IR-interpreter tier — the same graceful degradation the native path uses when it
falls back to bundling a namespace as interpreted source. Subfunction names are
globally unique (`GLOBAL_NAME_CTR`), so there are no collisions across the bundle.

A new `AotError::Wasm(WasmError)` surfaces backend errors through the CLI.

---

## 8. Runtime linking — the last, runtime-side step

The compiler now emits a whole-program AOT module whose memory/table bases are
`WasmLayout`-configurable. What remains to make a module *runnable* is genuinely
**runtime-side**, not a compiler increment:

1. **Export the `"rt"` surface from the runtime.** The emitted module imports the
   `rt_abi` bridges, linear memory, and the shared `__indirect_function_table`
   from module `"rt"`. The `cljrs-wasm` crate already builds the interpreter to
   `wasm32-unknown-unknown`, but does not yet *export* the `rt_abi` functions
   (they live in `cljrs-compiler::rt_abi`, a host-side crate) nor reserve
   table/memory regions for the AOT bundle.
2. **Instantiate the AOT module against the runtime.** A host (JS) instantiates
   the runtime, then instantiates the AOT module with the runtime's exports as its
   imports, reserving `[rodata_base, …)` / `[func_table_base, …)` and passing the
   concrete bases through `WasmLayout`.
3. **Wire the IR interpreter in as the dynamic-code tier**, so `eval`/REPL,
   freshly-required namespaces, and any namespace the bundler skipped still run.

Until then, the emitted module validates and is structurally complete, but its
imports are unsatisfied — it is the AOT *artifact*, not yet a running program.

---

## 9. Testing

The backend is tested at two levels, both gated on `wasmparser` validation:

- **Unit tests** (`cargo test -p cljrs-compiler wasm::`, 52 tests) construct IR
  functions by hand and assert the emitted module validates *and* has the
  expected shape — e.g. that a `^long` param emits a trampoline + typed body and
  the `rt_coerce_long` import, that a tail call emits `return_call` only with
  `tail_calls` on (asserted via a `wasmparser` operator scan), that a non-default
  `WasmLayout` relocates the data and element segments. The `abi` tests check the
  value→type mapping and the region contract; the `reloop` tests assert every
  block is emitted exactly once across straight-line, diamond, loop, and nested
  merge shapes.
- **End-to-end tests** (`crates/cljrs-compiler/tests/wasm_compile.rs`) drive real
  `.cljrs` source through `compile_file_to_wasm`: a file of `defn`s, a
  `loop`/`recur` accumulator, and a program that `require`s a second namespace
  (asserting the dependency's `__cljrs_ns_init_0` is bundled and exported).

The guiding rule for any new lowering: **validate every new shape with
`wasmparser` in a unit test.** A module that validates is structurally correct
wasm even with no JS runtime to execute it.

---

## 10. Status at a glance

**Done** — relooper (reducible CFGs); the ABI/region contract; the emitter for
scalar + string/keyword/symbol constants, boxed and unboxed arithmetic/comparison,
collection + region allocation, calls (`CallDirect`/`CallWithRegion`/`Call`),
closures + the shared function table, cross-function tail calls, globals/vars,
exceptions (thread-local path), unboxed scalar intermediates, the typed parameter
ABI; the CLI front-end with whole-program cross-namespace bundling; the
configurable `WasmLayout`.

**Open (runtime-side)** — linking the module against the `wasm32-unknown-unknown`
runtime (export `rt_abi`, instantiate, finalise the bases) and wiring the IR
interpreter in as the dynamic-code tier. See §8.

**Not yet lowered** — the async poll-function ABI and the `rt_call_ic` inline
cache (both return `Unsupported`); the caller-side direct-call optimisation for
typed callees (§5.11).

**Deferred indefinitely** — **WasmGC** (host-managed reference types in place of
the linear-memory GC; a project on its own) and an **in-browser JIT** (runtime
wasm generation + `WebAssembly.instantiate`, worthwhile only as a tier *on top of*
solid AOT).
