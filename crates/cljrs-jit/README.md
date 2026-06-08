# cljrs-jit

**Purpose:** In-process JIT compiler (Phase 10.1) that compiles hot clojurust
functions to native code via Cranelift, making `cljrs run` and the REPL
approach AOT-class throughput with no explicit compile step.

**Status:** Phase 10.1 — functional for non-capturing, non-variadic,
non-destructuring top-level `defn`s (the same set that Tier-1 handles).

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Public API: `init()` — installs the enqueue hook and spawns the worker |
| `src/jit_compiler.rs` | `compile_jit(arity_id, ir_func)` — builds `JITModule`, registers rt_abi symbols, calls shared codegen |
| `src/jit_worker.rs` | Background worker thread: receives `CompileRequest`s, publishes results to `cljrs_eval::jit_state` |

## Public API

```rust
/// Initialise the JIT tier. Call once at process startup.
/// Idempotent; safe to call multiple times.
pub fn init();
```

## Execution tiers (after `init()`)

```
call_cljrs_fn
  1. JIT-native  — cljrs_eval::jit_state::get_native_fn()   ← fastest
  2. Tier-1 IR   — cljrs_eval::ir_cache::get_cached()
  3. Tree-walk   — cljrs_interp::apply::call_cljrs_fn()
```

## Configuration

| Env var | Default | Description |
|---------|---------|-------------|
| `CLJRS_JIT_THRESHOLD` | `1000` | Calls before a function is JIT-compiled |
| `CLJRS_NO_JIT` | unset | Set to any value to disable JIT init |
| `CLJRS_NO_IR` | unset | Disables IR lowering (also disables JIT) |

## GC integration

- JIT-compiled code calls `rt_safepoint()` at function entry and loop back-edges.
- The Tier-1 dispatch path roots argument slices via `cljrs_env::gc_roots::root_values`
  before entering native code, so GC STW scans find all live `Value`s.
- Code unloading (Phase 10.2): `JITModule`s are kept alive in the worker thread
  for the process lifetime; epoch-tagged reclamation at STW safepoints is deferred.
