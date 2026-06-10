# cljrs-jit

**Purpose:** In-process JIT compiler that compiles hot clojurust functions to
native code via Cranelift, making `cljrs run` and the REPL approach AOT-class
throughput with no explicit compile step.

**Status:** Phase 10.2 — functional for non-capturing, non-variadic,
non-destructuring top-level `defn`s (the same set that Tier-1 handles), with
epoch-tagged **code unloading**: redefined functions' native code is reclaimed
at stop-the-world GC safepoints, keeping executable memory bounded across a long
REPL session.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Public API: `init()` — installs the enqueue, var-rebind, and STW-reclaim hooks and spawns the worker; `on_var_rebind`/`arity_ids` drive staling on redefinition |
| `src/jit_compiler.rs` | `compile_jit(arity_id, ir_func)` — builds `JITModule`, registers rt_abi symbols, calls shared codegen; returns `CompiledFn { fn_ptr, module, code_size }` |
| `src/jit_worker.rs` | Background worker thread: receives `CompileRequest`s, registers each module in `code_cache`, publishes the function pointer + epoch to `cljrs_eval::jit_state`. Each compile is wrapped in `catch_unwind` so a codegen panic on one function cannot kill the worker (the function just stays at Tier 1) |
| `src/code_cache.rs` | Epoch-tagged registry of compiled modules; `mark_stale` on redefinition, `reclaim_at_stw` frees stale modules with no live frame via `JITModule::free_memory` |

## Public API

```rust
/// Initialise the JIT tier. Call once at process startup.
/// Idempotent; safe to call multiple times.
pub fn init();

/// code_cache — code unloading (Phase 10.2)
pub fn reclaim_at_stw() -> usize;   // free stale modules with no live frame (STW only)
pub fn live_count() -> usize;       // live (in-use) modules
pub fn stale_count() -> usize;      // modules awaiting reclamation
pub fn reclaimed_count() -> u64;    // cumulative modules freed
pub fn reclaimed_bytes() -> u64;    // cumulative machine-code bytes freed
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

## Code unloading (Phase 10.2)

Emitted code holds no GC pointers (constants are materialized via `rt_abi`
runtime calls), so a module can be freed without disturbing the heap. The
lifecycle of a compiled arity's native code:

1. **Compile & publish.** The worker compiles the module, `code_cache::register`
   assigns it a monotonic **epoch** and takes ownership, and the worker publishes
   `(fn_ptr, epoch)` into the `cljrs_eval::jit_state` dispatch table.
2. **Track frames.** Each native call pushes its epoch onto a per-thread stack
   (`jit_state::push_jit_frame`) for the duration of the call.
3. **Stale on redefinition.** When a var holding the function is rebound, the
   value layer's rebind hook (`cljrs_value::set_var_rebind_hook` →
   `on_var_rebind`) nulls the dispatch pointer (future calls fall back to the
   interpreter) and calls `code_cache::mark_stale(epoch)`.
4. **Reclaim at STW.** `code_cache::reclaim_at_stw` runs at the existing
   stop-the-world GC safepoint (installed via `cljrs_eval::set_stw_reclaim_hook`).
   With all mutators parked, it scans active frames across all threads
   (`jit_state::live_epochs`) and frees every stale module whose epoch has **no**
   live frame — resolving the unload-vs-execute race without a separate protocol.
