# cljrs-eval

IR-accelerated evaluation for clojurust. Wraps the tree-walking interpreter
(`cljrs-interp`) with IR lowering and interpretation for faster function
execution.

**Phase:** IR tier-1 interpreter — implemented.

---

## Purpose

When a Clojure function has been lowered to IR — by the warm-threshold
background lowering worker (Phase 10.7, the default), eagerly at definition
time via the `on_fn_defined` hook (`CLJRS_EAGER_LOWER=1`), or from a pre-built
cache — calls are dispatched to the tier-1 IR interpreter. Otherwise they fall
back to the tree-walking interpreter in `cljrs-interp`.

The crate also manages the Clojure compiler namespaces (`cljrs.compiler.anf`,
`cljrs.compiler.ir`, `cljrs.compiler.known`) that perform ANF lowering of
Clojure source to the IR representation defined in `cljrs-ir`.

---

## File layout

```
src/
  lib.rs          — module declarations, re-exports, standard_env_minimal/standard_env/standard_env_with_paths,
                    register_compiler_sources, ensure_compiler_loaded
  apply.rs        — IR-aware function dispatch: tries IR cache, falls back to cljrs_interp::apply
  ir_interp.rs    — tier-1 IR interpreter: executes IrFunction over a VarId→Value register file;
                    counts loop back-edges and transfers into compiled OSR entries (Phase 10.4);
                    LoadGlobal is version-aware: `name@<sha>` resolves via
                    cljrs_env::versioned, and lookups into a not-yet-loaded
                    `ns@<sha>` namespace trigger a lazy versioned load
  ir_cache.rs     — thread-safe cache of lowered IR keyed by arity ID (NotAttempted/Cached/Unsupported);
                    invalidate(id) drops an entry back to NotAttempted (cross-defn invalidation);
                    Cached entries carry a last-access timestamp, and sweep_idle (run at the STW
                    reclaim pass) evicts entries idle past CLJRS_IR_CACHE_TTL (Phase 10.7)
  lower_worker.rs — (Phase 10.7) background IR-lowering worker thread ("cljrs-ir-lower"): receives
                    macro-expanded LowerRequests from the dispatch seam, runs the Env-free half of
                    lowering (ANF + optimize), publishes to ir_cache, and registers defns; sole
                    consumer of relower marks (publish-validate-retry against concurrent rebinds)
  ir_convert.rs   — converts Clojure Value data structures (maps/vectors/keywords) → Rust IR types
  lower.rs        — bridges the Clojure compiler front-end (cljrs.compiler.anf/lower-fn-body) to produce IrFunction;
                    threads cross-defn externals into cljrs_ir::lower::optimize_with_externals (Phase 10.5)
  defn_registry.rs— (Phase 10.5) cross-defn IR registry: registers each eagerly-lowered top-level defn
                    (keyed by GlobalEnv identity + ns + name), supplies ExternalDefns to later lowerings
                    so stage-4 region promotion fires in the script/REPL flow, tracks inverse dependency
                    edges, and invalidates dependents on var rebind (cached IR dropped, native code
                    staled, lazy re-lower on next dispatch)
  jit_state.rs    — JIT invocation counters, native-fn-pointer dispatch table (keyed by ir_arity_id),
                    per-arity epoch, active-frame tracking for code unloading (Phase 10.2), the
                    OSR slot table keyed by (arity_id, loop header) (Phase 10.4), and the stale-epoch
                    hook + stale_native_code(arity_id) used by cross-defn invalidation (Phase 10.5)
```

---

## Public API

```rust
/// Re-exports from cljrs-interp and cljrs-env:
pub use cljrs_interp::eval::eval;
pub use cljrs_env::env::{Env, GlobalEnv};
pub use cljrs_env::error::{EvalError, EvalResult};
pub use cljrs_env::callback::invoke;
pub use cljrs_env::loader::load_ns;

/// Create a minimal GlobalEnv with IR-accelerated eval/call dispatch.
/// Passes cljrs-eval's apply::call_cljrs_fn (IR + tree-walk fallback)
/// and eager_lower_fn hook to cljrs-interp's standard_env_minimal.
pub fn standard_env_minimal() -> Arc<GlobalEnv>;

/// Like standard_env_minimal() but also registers compiler sources.
pub fn standard_env() -> Arc<GlobalEnv>;

/// Like standard_env() but also sets user source paths.
pub fn standard_env_with_paths(source_paths: Vec<PathBuf>) -> Arc<GlobalEnv>;

/// Register the Clojure compiler namespace sources into the GlobalEnv.
pub fn register_compiler_sources(globals: &Arc<GlobalEnv>);

/// Load the Clojure compiler namespaces and mark the compiler as ready
/// for IR lowering. Thread-safe; idempotent.
pub fn ensure_compiler_loaded(globals: &Arc<GlobalEnv>, env: &mut Env) -> bool;

/// Load pre-built IR from a serialized bundle into the IR cache.
/// Walks all namespaces, matches bundle keys to runtime arity IDs.
/// Returns the number of arities loaded.
pub fn load_prebuilt_ir(globals: &Arc<GlobalEnv>, bundle: &IrBundle) -> usize;

/// IR lowering helpers (in module `lower`):
///
/// `lower_arity(name, params, rest, destructure_params, destructure_rest, body,
///     ns, env, is_async)` — ANF lowering only.
/// `lower_and_optimize_arity(name, params, rest, destructure_params,
///     destructure_rest, body, ns, env, is_async)` — also runs
///     region-optimization.  Both accept `is_async: bool` from the `CljxFn` and
///     propagate it to `IrFunction::is_async`.
///
/// `destructure_params: &[(usize, Form)]` carries the original destructuring
/// patterns for parameters the interpreter replaced with gensym placeholders
/// (paired with their index into `params`); `destructure_rest: Option<&Form>`
/// is the rest parameter's pattern when it is itself destructured.  Both are
/// expanded into explicit bindings in the IR prologue, so destructured-param
/// arities now lower to the IR/JIT tiers instead of falling back to the
/// tree-walker.
pub mod lower {
    pub fn lower_arity(..., is_async: bool) -> Result<IrFunction, LowerError>;
    pub fn lower_and_optimize_arity(..., is_async: bool) -> Result<IrFunction, LowerError>;
    /// Like lower_and_optimize_arity, but also returns the (ns, name) set of
    /// cross-defn externals the optimizer consulted (invalidation deps).
    pub fn lower_and_optimize_arity_tracked(..., is_async: bool)
        -> Result<(IrFunction, Vec<(Arc<str>, Arc<str>)>), LowerError>;

    // Phase 10.7 — the two halves of lowering, split for background use:
    /// Macro-expand a body on the calling thread (macros need the interpreter).
    pub fn macroexpand_body(body: &[Form], env: &mut Env) -> Vec<Form>;
    /// Env-free lowering of an already-expanded body; callable off-thread.
    /// `arity_id: Some(id)` uses defn_registry::snapshot_externals (atomic
    /// dependent recording, required off the mutator thread); `None` uses the
    /// legacy externals_for (synchronous callers record dependents themselves).
    pub fn lower_expanded_arity(name, params, rest, destructure_params,
        destructure_rest, expanded_body, ns, globals_id: usize,
        arity_id: Option<u64>, do_optimize: bool, is_async: bool)
        -> Result<(IrFunction, Vec<(Arc<str>, Arc<str>)>), LowerError>;
    /// Identity of the GlobalEnv behind `env` (scopes the cross-defn registry).
    pub fn globals_id(env: &Env) -> usize;
}

/// Cross-defn IR registry (in module `defn_registry`, Phase 10.5):
pub mod defn_registry {
    pub fn register_defn(globals_id, ns, name, arities: Vec<(usize, bool, Arc<IrFunction>)>);
    pub fn externals_for(globals_id, referenced) -> Vec<ExternalDefn>;
    pub fn record_dependents(arity_id, used);
    /// Phase 10.7: externals_for + record_dependents in one step, atomic with
    /// respect to on_redefined (holds the registry lock across the edge write).
    /// The background worker must use this — see lower_worker.rs.
    pub fn snapshot_externals(globals_id, arity_id, referenced) -> Vec<ExternalDefn>;
    pub fn on_redefined(ns, name) -> Vec<u64>;   // dependents to invalidate
    pub fn relower_pending() -> bool;            // dispatch fast-path check
    pub fn relower_marked(arity_id) -> bool;     // peek without consuming (dispatch)
    pub fn take_relower(arity_id) -> bool;       // consume (lowering worker only)
    pub fn install_invalidation_hook();          // idempotent; var-rebind hook
}
```

---

## IR dispatch flow

1. `apply::call_cljrs_fn` is registered as the `call_cljrs_fn` function pointer in `GlobalEnv`
2. On each call, it checks `ir_cache::get_cached(arity_id)` for a lowered IR function
3. If cached **and not async**: executes via `ir_interp::interpret_ir` (register-file interpreter)
4. If not cached **or async**: counts the call (`jit_state::record_interp_call`, Phase 10.7 —
   see "Background lowering" below) and falls back to `cljrs_interp::apply::call_cljrs_fn`
   (tree-walking).  For `^:async` functions the tree-walking path dispatches to `eval_async`
   in `cljrs-async`, which cooperatively yields to the Tokio `LocalSet` executor.
5. How IR gets into the cache:
   - **Warm-threshold background lowering (default, Phase 10.7)**: when a function's
     tree-walked call count crosses `ir_threshold()` (default 50), the dispatch seam
     macro-expands its arity bodies on the calling thread and enqueues them to the
     `cljrs-ir-lower` worker, which lowers + optimizes off-thread and publishes via
     `ir_cache::store_cached`.
   - **Eager lowering (opt-in, `CLJRS_EAGER_LOWER=1`)**: `ir_interp::eager_lower_fn` is
     registered as the `on_fn_defined` hook; when `compiler_ready` is true, new `fn*`
     definitions are lowered immediately.
   - **Pre-built bundles**: `load_prebuilt_ir`.
   The resulting `IrFunction::is_async` flag matches the `CljxFn::is_async` attribute.
6. `eval_call` in `cljrs_interp` routes `Value::Fn` calls through `globals.call_cljrs_fn`
   (the registered hook) rather than calling the tree-walker directly, so IR-cached
   arities are used on direct call paths too
7. JIT tier: before the IR cache, `call_cljrs_fn` checks `jit_state::get_native_fn(arity_id)`
   for compiled native code and, if present, dispatches to it.  `call_jit_native` brackets
   the native call with: a frame epoch (code unloading), GC roots for the caller env and
   args, **an eval context** (rt_abi bridges — `rt_call`, `rt_load_global`, the HOF
   bridges — dispatch through `cljrs_env::callback`; without it they silently return nil),
   and an alloc frame.  After the call it takes any pending exception stashed by an
   uncaught native `(throw …)` and re-raises it as `EvalError::Thrown` (same in
   `try_osr_enter` for OSR entries)

## Background lowering & cold-IR eviction (Phase 10.7)

The default tiering pipeline is count-driven end to end:

```
Tier 0 tree-walk ──(ir_threshold, 50 calls)──▶ background lower ──▶ Tier 1 IR
Tier 1 IR ──(jit_threshold, 1000 calls; counter restarts at IR publish)──▶ Tier 2 JIT
```

- The crossing call macro-expands the fn's arity bodies **on the calling
  thread** (macros are user Clojure functions and need the interpreter), then
  ships a `LowerRequest` (plain `Form` data) to the `cljrs-ir-lower` worker.
  The worker is not a GC mutator: it only runs the Env-free half of lowering.
- Skipped: macros, async fns, capturing closures, bootstrap-era definitions
  (arity id below the watermark snapshotted by `ensure_compiler_loaded`), and
  fns defined in builtin-source namespaces (clojure.test, clojure.string, the
  compiler namespaces, …).  Background lowering targets **user code only**:
  shipped namespaces only ever reached the IR tiers under opt-in eager
  lowering, and some of their patterns are known to miscompile (see TODO.md
  Phase 10.7 notes).
- Rebind safety: `snapshot_externals` records dependent edges atomically with
  reading the registry, and the worker is the only consumer of relower marks —
  after `store_cached` it re-peeks the mark and re-lowers (≤3 attempts) if a
  rebind landed mid-flight.  The dispatch seam only peeks
  (`relower_marked` + `lower_queued` dedup) and enqueues.
- Cold eviction: `Cached` entries track last access; `ir_cache::sweep_idle`
  runs at the stop-the-world reclaim pass and evicts entries idle past
  `CLJRS_IR_CACHE_TTL` (default 600 s) — deliberately *colder* than native
  code.  Entries backing published native code or an in-flight compile are
  never evicted (deopt fallback); `Unsupported` markers are kept forever.
  Eviction drops the `JitEntry` (the fn can re-warm) and stales any OSR code.
- Knobs: `CLJRS_IR_THRESHOLD` / `set_ir_threshold` / `--ir-threshold N`
  (0 disables background lowering), `CLJRS_IR_CACHE_TTL`, `CLJRS_NO_IR`
  (kills all IR), `CLJRS_EAGER_LOWER=1` (restores eager lowering — also the
  escape hatch for the known limitation that a long-running loop entered at
  Tier 0 cannot tier up mid-call, since the tree-walker has no OSR).

## JIT state & code unloading (`jit_state`)

`jit_state` is the seam between the Tier-1 interpreter and the background JIT
(`cljrs-jit`). Public surface:

```rust
pub fn set_jit_threshold(t: u32);                 // calls before compile (default 1000)
pub fn set_ir_threshold(t: u32);                  // Tier-0 calls before background lowering
                                                  // (default 50; u32::MAX disables)
pub fn record_interp_call(arity_id) -> bool;      // Tier-0 call accounting; true = snapshot+enqueue
pub fn lower_queued(arity_id) -> bool;            // dedup gate for the warm/relower paths
pub fn mark_lower_queued(arity_id);               // set on accepted enqueue
pub fn clear_lower_queued(arity_id);              // worker re-arms after abandoning an arity
pub fn on_ir_published(arity_id);                 // worker: restart counter at IR publish
pub fn evict_entry_if_cold(arity_id) -> bool;     // TTL sweep: drop entry unless native/queued
pub fn stale_osr_code(arity_id);                  // TTL sweep: stale published OSR entries
pub fn compile_queued(arity_id) -> bool;          // TTL sweep: in-flight JIT needs the IR
pub fn set_bootstrap_arity_watermark(w: u64);     // ensure_compiler_loaded snapshots the boundary
pub fn is_bootstrap_arity(arity_id) -> bool;      // bootstrap fns excluded from background lowering
pub fn record_call(arity_id, ir_func, profile_args);  // bump counter + arg-type profile; enqueue when hot
pub fn arg_type_profile(arity_id) -> Option<Vec<u8>>; // per-param type bitmasks (PROFILE_LONG/_DOUBLE/_OTHER)
pub fn set_enqueue_hook(f);                        // installed by cljrs_jit::init
pub fn store_native_fn(arity_id, ptr, epoch);      // worker publishes compiled code
pub fn get_native_fn(arity_id) -> Option<(*const (), u64)>;   // (fn_ptr, epoch)
pub fn take_native_epoch(arity_id) -> Option<u64>; // on redefinition: null ptr, drop entry, return epoch
pub fn push_jit_frame(epoch) -> JitFrameGuard;     // mark a native frame live for its call
pub fn current_jit_epoch() -> Option<u64>;         // innermost native frame's epoch (closure-escape pinning)
pub fn live_epochs() -> HashSet<u64>;              // epochs with a live frame (call at STW only)
pub fn set_pending_exception_hook(f);              // installed by cljrs_jit::init (rt_abi taker)
pub fn take_pending_exception() -> Option<Value>;  // uncaught native throw, taken at the dispatch seam
pub fn set_stale_epoch_hook(f);                    // installed by cljrs_jit::init (code_cache::mark_stale)
pub fn stale_native_code(arity_id);                // null ptr + route epochs to the stale hook (10.5)
pub unsafe fn dispatch_jit_call(fn_ptr, args) -> *const Value;

// Deoptimization (Phase 10.6):
pub fn set_deopt_sentinel_hook(f: fn() -> usize);  // installed by cljrs_jit::init (rt_abi sentinel addr)
pub fn is_deopt_result(ptr: *const Value) -> bool; // dispatch seam: did the entry guard fail?
pub fn record_deopt(arity_id);                     // count a guard failure; past deopt_limit():
                                                   // unpublish + stale the specialized code, ban
                                                   // the arity from re-specialization
pub fn specialization_allowed(arity_id) -> bool;   // worker: may this arity be specialized?
pub fn deopt_limit() -> u32;                       // CLJRS_JIT_DEOPT_LIMIT (default 10)
```

`call_jit_native` checks `is_deopt_result` on every native return: a
specialized function whose entry type guard failed returns rt_abi's sentinel
*before any side effect*, so the seam simply re-executes the call at Tier 1
(`execute_ir`) — exact interpreter semantics for the violating call.

Type profiles (Phase 10.6): `record_call` ORs each positional argument's type
class (`PROFILE_LONG` / `PROFILE_DOUBLE` / `PROFILE_OTHER`) into
`JitEntry::arg_profile` until the compile is queued; variadic arities profile
only the fixed prefix (the rest-list param is padded `PROFILE_OTHER` so it can
never be specialized).  The JIT worker reads the snapshot via
`arg_type_profile` to choose per-parameter specializations.

Each native call brackets itself with `push_jit_frame(epoch)` so the JIT code
cache can free a superseded module only once no frame is executing it
(`live_epochs` scanned at the stop-the-world GC safepoint).

## OSR — on-stack replacement (Phase 10.4)

A single hot call containing a `loop*`/`recur` never returns to re-dispatch, so
the invocation counter cannot promote it.  Instead:

1. `interpret_ir_with_osr` (the dispatch path used by `apply::execute_ir`,
   which passes the arity ID) counts back-edges per `RecurJump` target.  The
   counters are local to one execution on purpose: hot-within-one-call is
   exactly the case invocation tiering misses.
2. Crossing `osr_threshold()` calls `jit_state::osr_request`, which enqueues
   `(arity_id, header_block, IrFunction)` to the JIT worker exactly once.
3. The worker builds the OSR-entry variant (`cljrs_ir::osr::build_osr_function`),
   compiles it, and publishes `(fn_ptr, epoch, live_ins)` via `store_osr_fn`.
4. At each subsequent loop-header entry (after φ resolution, so the loop
   variables are current), the interpreter polls `osr_poll`; on `Ready` it
   snapshots the live-in registers and calls the native entry
   (`try_osr_enter`) — the native frame finishes the loop *and* the rest of
   the function, and its return value becomes the call's result.

OSR `jit_state` surface:

```rust
pub fn set_osr_threshold(t: u32);                  // back-edges before compile
pub fn osr_threshold() -> u32;                     // override → CLJRS_OSR_THRESHOLD → jit_threshold()
pub fn set_osr_enqueue_hook(f);                    // installed by cljrs_jit::init
pub fn osr_request(arity_id, header, ir_func);     // idempotent compile request
pub fn osr_poll(arity_id, header) -> OsrPoll;      // NotRequested | Pending | Ready(OsrSlot) | Failed
pub fn store_osr_fn(arity_id, header, ptr, epoch, live_ins);  // worker publishes
pub fn mark_osr_failed(arity_id, header);          // worker declines; interpreters stop polling
pub fn take_osr_epochs(arity_id) -> Vec<u64>;      // on redefinition: drop entries, return epochs
```

`OsrSlot { fn_ptr, epoch, live_ins }` carries the interpreter registers to pass
(in parameter order); the transfer uses the same rooting + `push_jit_frame`
protocol as ordinary JIT-native calls.  Scratch regions opened before the loop
stay open across the transfer (the OSR variant drops their `RegionEnd`s) and
unwind with the interpreter frame.

## Special-form coverage in the IR interpreter

Several `clojure.core` entries are sentinel stubs that error unconditionally
when called through the normal function-call path — the real logic lives in
`eval_call`'s special-form dispatch.  `ir_interp.rs` handles all of them
without going through the stubs:

| Operation | How handled in IR |
|---|---|
| `swap!` (`KnownFn::AtomSwap`) | `cljrs_interp::apply::eval_swap_bang` |
| `with-bindings*` (`KnownFn::WithBindings`) | `cljrs_interp::apply::eval_with_bindings_star` |
| `volatile!` | `dispatch_sentinel_by_name` → `eval_volatile` |
| `vreset!` | `dispatch_sentinel_by_name` → `eval_vreset_bang` |
| `vswap!` | `dispatch_sentinel_by_name` → `eval_vswap_bang` |
| `make-delay` | `dispatch_sentinel_by_name` → `make_delay_from_fn` |
| `alter-var-root` | `dispatch_sentinel_by_name` → `eval_alter_var_root` |
| `vary-meta` | `dispatch_sentinel_by_name` → `eval_vary_meta` |
| `send` / `send-off` | `dispatch_sentinel_by_name` → `eval_send_to_agent` |

Both `Inst::Call` (where the callee register holds a sentinel `NativeFunction`)
and `Inst::CallDirect` (where the callee is named directly) are intercepted.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-interp` | Tree-walking interpreter (fallback) |
| `cljrs-env` | `Env`, `GlobalEnv`, `EvalError`, callbacks, loader |
| `cljrs-builtins` | `form_to_value` (used by `lower.rs`) |
| `cljrs-ir` | IR types (`IrFunction`, `Block`, `Inst`, etc.) and compiler source strings |
| `cljrs-types` | `Span` |
| `cljrs-gc` | `GcPtr<T>` |
| `cljrs-reader` | `Form` AST (for compiler loading) |
| `cljrs-value` | `Value`, `CljxFn`, collections |
