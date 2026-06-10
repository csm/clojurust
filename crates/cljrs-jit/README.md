# cljrs-jit

**Purpose:** In-process JIT compiler that compiles hot clojurust functions to
native code via Cranelift, making `cljrs run` and the REPL approach AOT-class
throughput with no explicit compile step.

**Status:** Phase 10.4 — functional for top-level `defn`s **including
closure-bearing functions** (closure subfunctions are declared and compiled
into the same module, as AOT does; modules that materialize closure values are
pinned against unloading), with epoch-tagged **code unloading** (redefined
functions' native code is reclaimed at stop-the-world GC safepoints, keeping
executable memory bounded across a long REPL session) and **OSR** (on-stack
replacement): a single-call hot `loop*`/`recur` is promoted to native code
mid-run via loop back-edge counters and an OSR-entry compile.

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Public API: `init()` — installs the enqueue (function + OSR), var-rebind, STW-reclaim, pending-exception, and closure-escape hooks and spawns the worker; `on_var_rebind`/`arity_ids` drive staling on redefinition (whole-function and OSR epochs) |
| `src/jit_compiler.rs` | `compile_jit(func_name, ir_func)` — builds `JITModule`, registers rt_abi symbols, recursively declares + compiles closure subfunctions into the same module (mirroring AOT), calls shared codegen; returns `CompiledFn { fn_ptr, module, code_size }` |
| `src/jit_worker.rs` | Background worker thread: receives `CompileRequest::Function` and `CompileRequest::Osr` requests, registers each module in `code_cache`, publishes the function pointer + epoch (whole-function: `jit_state::store_native_fn`; OSR: `jit_state::store_osr_fn` with the live-in list). Each compile is wrapped in `catch_unwind` so a codegen panic on one function cannot kill the worker (the function just stays at Tier 1; failed OSR compiles are recorded via `mark_osr_failed`) |
| `src/code_cache.rs` | Epoch-tagged registry of compiled modules; `mark_stale` on redefinition, `pin_epoch` for modules whose code leaked into a closure value, `reclaim_at_stw` frees stale, unpinned modules with no live frame via `JITModule::free_memory` |
| `src/osr_integration.rs` | (test-only) end-to-end OSR test: lowers a real `loop*` from source, builds + compiles the OSR entry, and calls the native code with a mid-loop register snapshot |

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
| `CLJRS_OSR_THRESHOLD` | = JIT threshold | Loop back-edges (within one call) before an OSR entry is compiled |
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
5. **Closure pinning.** A closure value built by `rt_make_fn*` inside JIT code
   captures a raw pointer into the executing module and lives on the GC heap,
   where the frame scan cannot see it (it may be called long after every frame
   returned).  The closure-escape hook (installed by `init()`, fired by
   `rt_make_fn`/`rt_make_fn_variadic`/`rt_make_fn_multi`) therefore pins the
   executing frame's epoch (`code_cache::pin_epoch`): a pinned module is never
   freed.  This is a deliberate, bounded leak — precise reclamation would
   require the GC to report when the closure value dies.

## OSR — on-stack replacement (Phase 10.4)

A script or REPL form is often a *single* call containing one very hot
`loop*`/`recur`; it never returns to re-dispatch, so the invocation counter
above can never promote it.  OSR promotes it mid-run:

1. **Back-edge counters.** `interpret_ir_with_osr` (Tier 1, `cljrs-eval`)
   counts `RecurJump`s per loop header within one execution.  Crossing
   `osr_threshold()` calls `jit_state::osr_request`, whose hook (installed by
   `init()`) enqueues `CompileRequest::Osr { arity_id, header, ir_func }`.
2. **OSR-entry compile.** The worker calls
   `cljrs_ir::osr::build_osr_function`, which keeps only the blocks reachable
   from the loop header and turns the loop's live-in values (the header φs +
   pre-loop defs the loop reads) into parameters.  The variant is compiled by
   the ordinary backend and registered in the `code_cache` under its own
   epoch; `jit_state::store_osr_fn` publishes `(fn_ptr, epoch, live_ins)`.
3. **Mid-loop transfer.** At its next loop-header entry (after φ resolution,
   so the loop variables are current), the interpreter snapshots the live-in
   registers and calls the native entry; the native frame runs the remaining
   iterations *and* everything after the loop, and its return value becomes
   the call's result.  The transfer uses the same rooting + frame-epoch
   protocol as ordinary JIT-native calls, so GC and code unloading see OSR
   frames like any other native frame.
4. **Unloading.** On var rebind, `jit_state::take_osr_epochs` drops the
   arity's OSR entries and their epochs are staled alongside the
   whole-function epoch.

Failures anywhere (transform declined, codegen error, panic) mark the
`(arity_id, header)` slot failed; the loop simply finishes at Tier 1.
