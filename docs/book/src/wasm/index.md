# WebAssembly

clojurust can compile Clojure to **WebAssembly** for native-fast, sandbox-safe
deployment in the browser. `cljrs compile --target wasm` runs the same IR
pipeline as the native backend, but emits a `.wasm` module instead of a native
binary.

> **Status: code generation complete; runtime linking in progress.** The
> compiler emits validated wasm modules for most of the language; making a module
> *runnable* in the browser (linking it against the wasm runtime) is the
> remaining step. See [Status](#status) below and the full design in
> [`docs/wasm-aot-plan.md`](https://github.com/csm/clojurust/blob/main/docs/wasm-aot-plan.md).

## Why a separate backend

clojurust's native execution tiers up at runtime, and its top tiers **generate
machine code while the program runs** (the [JIT](../memory/jit.md)). A WebAssembly
sandbox forbids exactly that: there is no `mmap(PROT_EXEC)` inside a module, so it
cannot generate and then execute fresh machine code.

The browser story is therefore **ahead-of-time**: compile each Clojure function
to wasm bytecode at *build* time and ship it; the browser's own engine JITs that
to native. The execution tiers **invert** relative to native:

| | Bottom (dynamic) | Top (peak) |
|---|---|---|
| **native** | tree-walk → IR-interp | JIT/OSR, reached at runtime |
| **browser** | tree-walk → IR-interp | **AOT-wasm**, frozen at build time |

The IR interpreter stays on board the wasm bundle as the dynamic-code tier — for
`eval`, the REPL, freshly-`require`d namespaces, and macros — while AOT-wasm is
the frozen top tier. No in-sandbox JIT or on-stack-replacement hooks are
installed.

## What is shared

Everything upstream of code generation is backend-agnostic and reused unchanged
from the native path: ANF/SSA lowering, escape analysis + region inference,
scalar representation inference, and the runtime-bridge contract. The only
genuinely wasm-specific work is **relooping** (recovering structured control flow,
since wasm has no `goto`) and the **bytecode emitter**. How those work is covered
in [The AOT backend](aot-backend.md).

Because regions are a property of the IR, **bump allocation comes along for free**
in wasm: a region is a linear-memory arena, a region handle is an `i32` offset.
See [Memory Management](../memory/index.md).

## Status

**Working** (every emitted module is validated with `wasmparser`):

- Scalar, string, keyword, and symbol constants; all control flow
  (`if`/`cond`/`loop`/`recur` via the relooper).
- Boxed and unboxed arithmetic and comparison; collection and region allocation.
- Calls (direct, region-threaded, and dynamic), closures via a shared function
  table, and cross-function tail calls.
- Globals/vars and exceptions (`throw`/`try`/`catch`).
- The **typed parameter ABI** — `^long`/`^double` params passed unboxed, with a
  boxed-entry trampoline for dynamic callers.
- Whole-program **bundling** — the entry namespace and every lowerable required
  namespace compile into one module.

**Remaining** — linking the module against the wasm runtime (so its imported
`rt_*` bridges, memory, and function table are satisfied) and wiring the IR
interpreter in as the dynamic-code tier. Until then the module is the AOT
*artifact*, not yet a running program.

**Not yet supported** — the async poll-function ABI; the per-call-site inline
cache. **Deferred indefinitely** — WasmGC (the linear-memory GC stays) and an
in-browser JIT.

## See also

- [Compiling to WebAssembly](../cli/compile.md#targeting-webassembly) — the CLI
  options.
- [The AOT backend](aot-backend.md) — how the value model, relooper, emitter, and
  typed-parameter ABI work.
- [`docs/wasm-aot-plan.md`](https://github.com/csm/clojurust/blob/main/docs/wasm-aot-plan.md)
  — the complete design and the open-task list.
