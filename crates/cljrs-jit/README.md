# cljrs-jit

In-process JIT (Cranelift) execution tier for clojurust — compiles hot function
arities to native code on a background thread.

## Purpose

A fourth execution tier (Tier 2) that compiles the hottest function arities to
native machine code in-process, so ad-hoc code (`cljrs run`, the REPL, `eval`)
reaches AOT-class speed with no explicit compile step. Reuses the shared
Cranelift codegen from `cljrs-compiler` (the same `IrFunction` → CLIF lowering
that drives AOT), but targets a `JITModule` instead of an object file.

## Status

**Phase 10.1 — Minimal JIT tier (first working JIT).** Implemented. Compiles
the set Tier-1 already handles: flat, non-capturing, non-destructuring,
non-variadic, non-async function arities (no nested closures / `subfunctions`,
no rest param). Falls back cleanly to the interpreter for everything else.

Off by default; enabled with `cljrs --jit` (or the `CLJRS_JIT` env var). Tune
the promotion threshold with `CLJRS_JIT_THRESHOLD` (default 1000 invocations).
Observe promotions with `-X trace:jit` / `-X debug:jit`.

Deferred to later phases (tracked in `TODO.md` / `docs/jit-plan.md`):
- 10.2 code unloading (compiled modules currently live for the process),
- 10.3 destructuring / closures / variadics / special-op lowering,
- 10.4 OSR for single-call hot loops,
- 10.5 context-driven bump allocation,
- 10.6 specialization & inline caches.

## How it plugs in

The hot dispatch path lives **below** this crate, in `cljrs-eval`
(`jit_state.rs`), because `cljrs-jit` → `cljrs-compiler` → `cljrs-eval`. To
avoid a dependency cycle, `init` registers two function-pointer hooks that
`cljrs-eval` calls:

- **compile hook** — `cljrs-eval` hands `(arity_id, IrFunction, n_params)` to
  this crate, which sends it to a background worker thread. The worker compiles
  via `JITModule`, then atomically publishes the finalized code pointer back
  into the shared `JitState`. Until then, calls keep running the interpreter —
  never a stall.
- **invoke hook** — when native code is ready, `cljrs-eval` calls a finalized
  `extern "C" fn(*const Value, ...) -> *const Value` through this hook, which
  surfaces any thrown exception stashed by `rt_throw`.

Dispatch order at the `call_cljrs_fn` seam becomes **JIT-native → Tier-1 IR →
tree-walk**.

### Runtime bridge & constants

Every `rt_*` symbol (`cljrs_compiler::rt_abi::rt_symbols`) is registered with the
`JITBuilder` so emitted calls resolve in-process. Constants are materialized
through `rt_const_*` runtime calls (the AOT strategy), so emitted code embeds no
`GcPtr`s — which keeps conservative stack scanning and future code unloading
tractable, since the code itself holds no GC roots.

### GC integration

Native code runs on the calling mutator thread and allocates into that thread's
GC heap via the `rt_*` bridge. The shared codegen already emits `rt_safepoint`
polls at function entry and loop back-edges, so JIT code cooperates with the
stop-the-world collector. The invoke path roots its argument `Value`s (via
`cljrs_env::gc_roots`) and bounds in-flight allocations with an alloc frame for
the duration of the call.

## File layout

```
src/
  lib.rs — init(), background compile worker, JITModule construction + symbol
           registration, single-function compilation, and the native call ABI
           (transmute by arity, 0..=8 pointer params).
```

## Public API

```rust
/// Register the compile + invoke hooks and spawn the background compile worker.
/// Idempotent; no effect unless the JIT is also enabled via
/// `cljrs_eval::jit_state::set_enabled`.
pub fn init();

/// Largest arity the native call ABI supports (8). Higher arities fall back.
pub const fn max_arity() -> usize;
```

The dispatch-side surface (counters, thresholds, code slots, hook registration)
lives in [`cljrs_eval::jit_state`].

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-compiler` (workspace) | Shared Cranelift codegen (`new_compiler_from_module`, `Compiler::module_mut`) + `rt_abi` symbols / pending-exception accessor |
| `cljrs-eval` (workspace) | `jit_state` shared structure + hook registration |
| `cljrs-ir` (workspace) | `IrFunction` input |
| `cljrs-value` (workspace) | `Value` (call ABI) |
| `cljrs-gc` (workspace) | GC interaction |
| `cljrs-logging` (workspace) | `jit` trace/debug feature |
| `cranelift-jit` / `cranelift-module` / `cranelift-codegen` / `cranelift-native` | JIT backend |
