# cljrs-eval

IR-accelerated evaluation for clojurust. Wraps the tree-walking interpreter
(`cljrs-interp`) with IR lowering and interpretation for faster function
execution.

**Phase:** IR tier-1 interpreter — implemented.

---

## Purpose

When a Clojure function has been lowered to IR (eagerly at definition time via
the `on_fn_defined` hook, or from a pre-built cache), calls are dispatched to
the tier-1 IR interpreter. Otherwise they fall back to the tree-walking
interpreter in `cljrs-interp`.

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
  ir_interp.rs    — tier-1 IR interpreter: executes IrFunction over a VarId→Value register file
  ir_cache.rs     — thread-safe cache of lowered IR keyed by arity ID (NotAttempted/Cached/Unsupported)
  ir_convert.rs   — converts Clojure Value data structures (maps/vectors/keywords) → Rust IR types
  lower.rs        — bridges the Clojure compiler front-end (cljrs.compiler.anf/lower-fn-body) to produce IrFunction
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
/// `lower_arity` — runs only `cljrs.compiler.anf/lower-fn-body`.
/// `lower_and_optimize_arity` — also runs `cljrs.compiler.optimize/optimize`,
///     which inserts `RegionStart`/`RegionAlloc`/`RegionEnd` instructions for
///     non-escaping allocations.  Caller must have required
///     `cljrs.compiler.optimize` first.  Used by `cljrs-stdlib`'s prebuild
///     (`prebuild-ir` feature) and by AOT compilation, so prebuilt/AOT code
///     gets bump-allocated regions.
pub mod lower {
    pub fn lower_arity(...) -> Result<IrFunction, LowerError>;
    pub fn lower_and_optimize_arity(...) -> Result<IrFunction, LowerError>;
}
```

---

## IR dispatch flow

1. `apply::call_cljrs_fn` is registered as the `call_cljrs_fn` function pointer in `GlobalEnv`
2. On each call, it checks `ir_cache::get_cached(arity_id)` for a pre-lowered IR function
3. If cached: executes via `ir_interp::interpret_ir` (register-file interpreter)
4. If not cached: falls back to `cljrs_interp::apply::call_cljrs_fn` (tree-walking)
5. Eager lowering: `ir_interp::eager_lower_fn` is registered as the `on_fn_defined` hook;
   when `compiler_ready` is true, new `fn*` definitions are lowered immediately
6. `eval_call` in `cljrs_interp` routes `Value::Fn` calls through `globals.call_cljrs_fn`
   (the registered hook) rather than calling the tree-walker directly, so IR-cached
   arities are used on direct call paths too

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
