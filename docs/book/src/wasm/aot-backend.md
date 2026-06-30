# The AOT backend

This page explains how the WebAssembly code generator works. It lives entirely in
`crates/cljrs-compiler/src/wasm/` and is a second consumer of the same `cljrs-ir`
IR that the native (Cranelift) backend consumes.

```text
crates/cljrs-compiler/src/wasm/
  mod.rs    — public API: compile_function, compile_bundle, WasmBackend, WasmError
  abi.rs    — the ABI contract: Value→i32, the rt_abi import table, regions, WasmLayout
  reloop.rs — the relooper: IR CFG → structured control flow (wasm-private)
  emit.rs   — the emitter: IrFunction(s) → a validated wasm module
```

The pipeline is always **`reloop` → `emit`**.

## The value model

Under `wasm32`, **every pointer is an `i32`** — a linear-memory offset. The GC
heap and all regions live in the module's single linear memory.

| IR representation | wasm type | Meaning |
|---|---|---|
| boxed `*const Value` | `i32` | offset of the boxed value |
| region handle | `i32` | offset of the region arena |
| unboxed `Long` | `i64` | raw long payload |
| unboxed `Double` | `f64` | raw double payload |
| `Bool` / tags | `i32` | small integers |

The default model is **boxed-only**: every IR value is, by default, a wasm `i32`
holding a boxed pointer. This is always correct; unboxing is layered on top as an
optimization. Because every pointer is an `i32`, the entire runtime-bridge surface
(`rt_abi`, ~165 `extern "C"` functions) is expressible as wasm imports from the
module named `"rt"`, with no marshalling beyond width changes.

## Relooping

wasm has only `block`/`loop`/`if` + labelled `br` — no `goto`. The relooper
recovers structured control flow from the IR's CFG using dominator-tree
structuring (Ramsey's *"Beyond Relooper"*), specialized to two facts true of
every CFG this backend sees:

- **Back edges are exactly `recur`.** Clojure has no `goto`; the only cyclic
  control flow is `loop`/`recur`, so loop headers are precisely the `recur`
  targets and every other edge is forward.
- **The CFG is reducible.** Structured source can't produce irreducible control
  flow, so the relooper needs no node-splitting or dispatch variable.

The output is a `Structured` tree (`Simple`/`Labeled`/`Loop`/`If`/`Br`/`Return`)
that the emitter walks directly: `Labeled`→`block`, `Loop`→`loop`, `If`→`if`/`else`,
`Br`→`br N`. Each block is emitted exactly once. This pass is wasm-*private* —
Cranelift wants the raw CFG and would be pessimized by re-structuring.

## The emitter

The emitter lowers the structured tree and each `Inst` to bytecode. Highlights:

- **SSA φ resolution** — no `phi` instruction is emitted. On each edge, each φ's
  incoming value is copied into its local using the operand stack for
  parallel-move semantics (all reads before any writes), so a swapping
  `(recur b a)` cannot clobber.
- **Allocation** — element pointers are marshalled into a runtime scratch buffer
  (`rt_scratch_ptr`), then a slice-taking `rt_alloc_*` bridge is called. Regions
  reuse the same machinery with the handle threaded as a leading argument.
- **Calls** — direct calls resolve to a wasm function index; region-threaded
  calls pass the handle as a hidden trailing argument; dynamic calls dispatch
  through `rt_call`. Because imported functions occupy the low index space, the
  emitter runs **two passes** — discover imports, then encode with the import
  count settled.
- **Closures** use a shared imported function table (a closure's function pointer
  is a table index); **tail calls** become `return_call` when the tail-call
  proposal is enabled.
- **Constants/globals** intern their bytes into a deduplicated read-only data
  pool emitted as one data segment; **exceptions** use the boxed thread-local
  error path (`rt_throw`/`rt_try`).
- **Unboxed scalars** — representation inference assigns `i64`/`f64`/`i32` to
  intermediates wherever the boxed bridge's semantics survive on the raw value,
  so hot arithmetic compiles to native wasm ops; values box on demand only at
  boxed-context boundaries.

A GC `rt_safepoint` is emitted at function entry and at each loop header.

> The native (Cranelift) backend is the semantic reference: the wasm emitter
> mirrors `codegen.rs` arm for arm. The one structural difference is control
> flow — Cranelift consumes the raw CFG; the wasm backend reloops it first.

## The typed parameter ABI

By default, parameters stay **boxed** (the signature is all-`i32`), because the
always-boxed dispatchers — dynamic `rt_call`, the indirect function table,
cross-function direct calls — cannot supply unboxed arguments.

A function with static `^long`/`^double` parameter hints compiles to **two** wasm
functions:

- a **typed body** whose hinted params arrive unboxed (`i64`/`f64`), so the body
  reads them with no per-use unbox;
- a boxed-entry **trampoline** with the all-`i32` signature every dispatcher
  expects. The trampoline is the function's *primary* entry — exported, installed
  in the table, and the target of every direct call — so all the boxed dispatch
  paths reach a typed function unchanged. It coerces each boxed argument
  (`rt_coerce_long`/`rt_coerce_double`) and tail-calls the typed body.

The native backend's specialized prologue *deoptimizes* on a tag mismatch; the
wasm sandbox has no deopt seam, so a violated static hint **coerces or throws**
instead (Clojure's `longCast`/`doubleCast` semantics).

## Whole-program bundling

`cljrs compile --target wasm` lowers the entry namespace **and every
transitively-`require`d user namespace** the backend can lower into one module
(each as a `__cljrs_ns_init_N` initializer, mirroring the native path's
per-namespace discovery). A namespace the backend can't lower is **skipped**, left
for the runtime's IR-interpreter tier — the same graceful degradation native AOT
uses. The module's read-only data and function-table base addresses are
configurable (`WasmLayout`) so the linking step can place them at the addresses
the runtime reserves.

## Testing

Every new shape is **validated with `wasmparser`** in a unit test (`cargo test -p
cljrs-compiler wasm::`) — a module that validates is structurally correct wasm
even with no JS runtime to execute it. End-to-end tests drive real `.cljrs` source
through `compile_file_to_wasm`, including a cross-namespace `require`.

For the complete design, the increment-by-increment build log, and the open-task
list, see
[`docs/wasm-aot-plan.md`](https://github.com/csm/clojurust/blob/main/docs/wasm-aot-plan.md).
