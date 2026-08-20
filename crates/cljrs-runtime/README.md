# cljrs-runtime

The clojurust runtime: namespaces and environments, native `clojure.core`
builtins, the tree-walking interpreter, and the tiered (IR-accelerated)
evaluator.

**Status:** implemented. Stage 2 of
[`docs/crate-consolidation-plan.md`](../../docs/crate-consolidation-plan.md)
merged four packages into this one, one per module; Stage 3 gave it one
construction path and one dispatch path; Stage 6 deleted the four packages'
re-export shims, so these module paths are the only paths:

| Module | Former package | Responsibility |
|---|---|---|
| [`env`](#module-env) | `cljrs-env` | Namespace registry, vars, dynamic bindings, GC roots, loader, gas, policy |
| [`builtins`](#module-builtins) | `cljrs-builtins` | Native `clojure.core` functions plus the Clojure bootstrap source |
| [`interp`](#module-interp) | `cljrs-interp` | Tree-walking interpreter: special forms, macros, destructuring |
| [`tiered`](#module-tiered) | `cljrs-eval` | IR lowering, tier-1 IR interpreter, JIT dispatch state |

Use `cljrs_runtime::{env, builtins, interp, tiered}`. The `cljrs-env`,
`cljrs-builtins`, `cljrs-interp`, and `cljrs-eval` packages no longer exist.

Stage 3 added [`Runtime` / `RuntimeBuilder` / `ExecutionMode`](#runtime-construction)
at the crate root and removed the `GlobalEnv` callback seams: the `eval_fn`,
`call_cljrs_fn`, and `on_fn_defined` function pointers and the `compiler_ready`
flag are gone, replaced by an execution mode chosen at build time and a
[`TierState`](#execution-modes-and-tier-state) raised once when the bootstrap
finishes. The IR cache is now per-runtime instance state, and the cross-defn
registry is keyed by a counter-allocated `GlobalEnv::id` instead of the
environment's address.

---

## File layout

```
src/
  lib.rs                — crate root; declares the modules; re-exports Runtime,
                          RuntimeBuilder, BuildError, ExecutionMode, TierState
  mode.rs               — ExecutionMode (build-time choice) and TierState (live tiers)
  runtime.rs            — Runtime and RuntimeBuilder: the one construction path
  logging.rs            — (non-WASM) the gc/env/ir/jit tracing targets, the filters
                          that select them, and subscriber installation

  env/
    mod.rs              — module declarations; re-exports AsyncRuntime
    env.rs              — Env (lexical scope) and GlobalEnv (namespace registry, vars)
    error.rs            — EvalError / EvalResult and conversion helpers
    apply.rs            — apply_value: callee dispatch (fns, keywords, maps, sets,
                          vars, protocols, multimethods) and async dispatch
    callback.rs         — thread-local eval context for Rust→Clojure callbacks
    dynamics.rs         — dynamic var binding frames (binding, with-bindings*)
    gas.rs              — cooperative execution-credit metering
    gc_roots.rs         — GC root registration for the interpreter's Rust stack
    loader.rs           — namespace loading (require / load_ns) from source paths
    policy.rs           — capability policy for isolated transaction functions
    taps.rs             — tap> / add-tap registry
    async_hook.rs       — AsyncRuntime seam and the async-JIT compile hook
    depth.rs            — call-depth cap for ExecutionMode::NoGcTransaction
    vcs.rs              — VcsProvider trait (the runtime's git seam) and the
                          cljrs-project-backed ProjectVcs impl (`deps` feature)
    versioned.rs        — (non-WASM) versioned symbol/namespace resolution

  builtins/
    mod.rs              — module declarations; re-exports special::* and util::*
    builtins.rs         — the native fn registry (register_all) and BUILTIN_DOCS
    special.rs          — special-form stub vars and shared dispatch helpers
    form.rs             — reader-conditional resolution; form → value conversion
    util.rs             — shared argument-coercion and error helpers
    transients.rs       — transient collection builtins
    array_list.rs       — java.util.ArrayList-alike interop shim
    bitops.rs           — bit-and / bit-or / bit-shift-* and friends
    new.rs              — (new Type ...) construction dispatch
    regex.rs            — re-pattern / re-find / re-seq / re-matches
    taps.rs             — add-tap / tap> builtins over env::taps
    time.rs             — clock and duration builtins
    bootstrap.cljrs     — Clojure source evaluated into clojure.core at startup
    clojure_test.cljrs  — embedded clojure.test source

  interp/
    mod.rs              — module declarations
    eval.rs             — top-level eval dispatch; symbol/keyword/collection eval
    special.rs          — special-form evaluators (def, fn*, let*, loop*, try, ns, …)
    apply.rs            — eval_call: macro expansion, native dispatch, recur trampoline
    arity.rs            — fresh arity ID generator
    destructure.rs      — pattern destructuring (vector, map, & rest)
    macros.rs           — macro expansion helpers
    syntax_quote.rs     — syntax-quote (backtick) expansion
    virtualize.rs       — let-chain virtualization: assoc/conj chains → transients
    versioned.rs        — (non-WASM) tree-walker entry point for versioned resolution

  tiered/
    mod.rs              — re-exports; load_prebuilt_ir
    apply.rs            — IR-aware dispatch: JIT → IR cache → tree-walk fallback
    ir_interp.rs        — tier-1 IR interpreter over a VarId→Value register file
    ir_cache.rs         — IrCache: per-runtime cache of lowered IR keyed by arity ID
    lower.rs            — orchestrates the pure-Rust cljrs_ir::lower pipeline
    lower_worker.rs     — background IR-lowering worker thread ("cljrs-ir-lower")
    defn_registry.rs    — cross-defn IR registry and invalidation edges
    jit_state.rs        — JitState: per-runtime JIT counters, native-fn table, epochs, OSR slots
    tiers.rs            — Tiers: one runtime's IrCache + JitState, and the index of live ones
    backend.rs          — JitBackend: the seam a compiler installs on a runtime

tests/
  no_gc_eval.rs                    — (no-gc) arithmetic, def provenance, region stack
  versioned_resolution.rs          — versioned resolution against a real git fixture
  vcs_provider.rs                  — the VcsProvider seam: default provider, degradation
                                     with none installed, signature-check routing and
                                     cache invalidation on provider/trust-set change
                                     (passes with and without the `deps` feature)
  require_spec_reader_conditional.rs — reader conditionals in ns require specs
  declare_macro.rs, doc.rs, gas_meter.rs, into_seq_target.rs, map_entry.rs,
  named_fn_identity.rs, ns_metadata.rs, partition_arities.rs, shared_atom.rs,
  symbolic_nan.rs, threading_macros.rs, auto_gensym.rs, auto_keyword_macro.rs,
  assoc_in_metadata.rs, empty_metadata.rs, into_metadata.rs, vec_metadata.rs,
  defonce_metadata.rs, defonce_metadata_properties.rs,
  auto_resolution_properties.rs   — tree-walker behavior
  gas_meter_ir.rs, versioned_ir.rs, partition_ir.rs, destructure_lowering.rs,
  osr_transfer.rs, region_phi_uaf.rs — tiered behavior
```

---

## Runtime construction

One path builds a runtime. Execution mode, source paths, GC configuration, and
embedded namespace sources are builder inputs; extensions install themselves
into a finished runtime.

```rust
use cljrs_runtime::{ExecutionMode, Runtime};

let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .source_paths(paths)
    .gc_config(config)
    .build()?;
cljrs_stdlib::install(&runtime);
```

### `Runtime`

A cheap, cloneable handle. All instance state lives in the shared `GlobalEnv`,
so clones name the same runtime rather than a new one.

```rust
pub fn builder() -> RuntimeBuilder;
/// Adopt an existing environment (AOT harness, embedding host, package loader).
pub fn from_globals(globals: Arc<GlobalEnv>) -> Runtime;
pub fn globals(&self) -> &Arc<GlobalEnv>;
pub fn into_globals(self) -> Arc<GlobalEnv>;
pub fn env(&self, ns: &str) -> Env;
pub fn execution_mode(&self) -> ExecutionMode;
pub fn tier_state(&self) -> TierState;
```

### `RuntimeBuilder`

```rust
pub fn execution_mode(self, mode: ExecutionMode) -> Self;   // default Tiered
pub fn source_paths(self, paths: Vec<PathBuf>) -> Self;
pub fn gc_config(self, config: Arc<GcConfig>) -> Self;
pub fn gc_config_from_env(self, enabled: bool) -> Self;     // default true
pub fn register_gc_roots(self, enabled: bool) -> Self;      // default true
pub fn builtin_source(self, ns: impl Into<String>, src: &'static str) -> Self;
pub fn eager_clojure_test(self, enabled: bool) -> Self;     // default false
pub fn build(self) -> Result<Runtime, BuildError>;
```

`build` registers native `clojure.core`, evaluates `bootstrap.cljrs`, sets up
the `user` namespace, applies GC and source-path configuration, and finally
raises the tier state — the bootstrap itself always tree-walks, because nothing
can be lowered before `clojure.core` exists. `BuildError::EmbeddedSource` is
returned when an embedded source fails to *parse* (the binary's own text is
broken); an individual bootstrap form that fails to evaluate is reported on
stderr and skipped, as before.

The GC root tracer registered by `register_gc_roots` holds a **weak** handle to
the environment, so a runtime that is dropped stops being a root instead of
keeping itself alive forever through the heap's tracer list.

## Execution modes and tier state

`ExecutionMode` is chosen once, at build time, and never changes: it selects the
function-call path. Before Stage 3 each mode was a different `fn` pointer stored
in `GlobalEnv`, and those pointers existed only to let the tree walker reach
the tiered evaluator across a package boundary. With both in one package the mode is
data and the dispatch is a direct call.

| Mode | `GlobalEnv::call_cljrs_fn` routes to | Target tier |
|---|---|---|
| `TreeWalk` | `interp::apply::call_cljrs_fn` | `TreeWalk` |
| `Tiered` (default) | `tiered::apply::call_cljrs_fn` | `Jit` |
| `TieredNoJit` | `tiered::apply::call_cljrs_fn` | `Ir` |
| `NoGcTransaction` | `env::depth::call_cljrs_fn` | `TreeWalk` |

`TierState` is the *current* state of that mode, and it does change. It starts
at `TreeWalk` and the builder raises it once to the mode's target tier when the
bootstrap finishes. It replaces the `compiler_ready` flag, which said only
"not tree-walk" and could not distinguish the IR interpreter from native JIT
dispatch — `TieredNoJit` stops at Tier 1 even with a JIT backend linked in.
`CLJRS_NO_IR` pins any runtime at `TreeWalk`.

```rust
pub enum ExecutionMode { TreeWalk, Tiered, TieredNoJit, NoGcTransaction }
impl ExecutionMode {
    pub fn target_tier(self) -> TierState;
    pub fn is_tiered(self) -> bool;
}

pub enum TierState { TreeWalk = 0, Ir = 1, Jit = 2 }
impl TierState {
    pub fn ir_enabled(self) -> bool;   // >= Ir
    pub fn jit_enabled(self) -> bool;  // == Jit
}
```

### `logging` (non-WASM)

Diagnostic logging is `tracing`, not an API of this crate: the runtime, GC, and
compiler emit under four **targets** — `gc`, `env`, `ir`, `jit` — with plain
`tracing::debug!` / `tracing::trace!`. This module only builds the filter that
selects them, so the CLI and a generated AOT harness agree by construction. An
embedding host can ignore all of it and install its own subscriber.

```rust
pub const FEATURE_TARGETS: &[&str];   // ["gc", "env", "ir", "jit"]
pub const NOISY_TARGETS:   &[&str];   // cranelift_*, regalloc2 — pinned to warn

/// Everything at `default`, codegen crates at `warn`, FEATURE_TARGETS pinned
/// OFF — a blanket `--debug` must not turn the firehoses on.
pub fn base_filter(default: impl Into<LevelFilter>) -> Targets;

/// Fold one `-X` / `CLJRS_X_FLAG` spec (`debug:gc,jit`) into a filter.
pub fn apply_x_flag(filter: Targets, spec: &str) -> Result<Targets, String>;

/// Let `RUST_LOG` replace `base`. An unparseable value is reported on stderr
/// and `base` is kept: `RUST_LOG` is the ecosystem's variable, so a value we
/// reject may not be aimed at us, and degrading to the host's default beats
/// both silence and a hard failure. Both hosts use this, so a bad `RUST_LOG`
/// means the same thing to the CLI and to an AOT binary.
pub fn apply_rust_log(base: Targets) -> Targets;

/// Install `filter` globally, formatting to stderr. Idempotent.
pub fn init(filter: Targets);

/// What a generated AOT harness calls. Enables nothing unless `CLJRS_X_FLAG`
/// or `RUST_LOG` asks. `CLJRS_X_FLAG` is ours alone and has one meaning, so an
/// unparseable value is an `Err` (the harness reports it and exits 2) rather
/// than being ignored — matching `cljrs -X`, and never leaving someone who
/// asked for diagnostics silently without them.
pub fn init_from_env() -> Result<(), String>;
```

---

## Module `env`

### `env` submodule

`Env` is a lexical scope chained to a parent; `GlobalEnv` is one runtime
instance's namespace registry (namespaces, interned vars, source paths,
refer/alias tables, the version cache) plus its execution mode, tier state,
IR cache, and identity.

```rust
/// Raw constructor: no builtins, no bootstrap. Use Runtime::builder().
pub fn GlobalEnv::new(execution_mode: ExecutionMode) -> Arc<GlobalEnv>;

/// Process-unique identity, allocated from a counter. Scopes the cross-defn
/// registry and the IR cache index. Not the Arc's address: an address is
/// unique only while its allocation is live, so a dropped runtime could hand
/// its key to the next one and let it inherit the dead runtime's IR.
pub fn id(&self) -> u64;

pub fn execution_mode(&self) -> ExecutionMode;
pub fn tier_state(&self) -> TierState;
pub fn set_tier_state(&self, tier: TierState);   // raise only (fetch_max)
pub fn ir_enabled(&self) -> bool;                // tier_state().ir_enabled()
pub fn ir_cache(&self) -> &Arc<tiered::ir_cache::IrCache>;

/// The single evaluation and function-call entry points. `eval` always runs
/// the tree walker; `call_cljrs_fn` matches on the execution mode; and
/// `on_fn_defined` eagerly lowers only for a tiered runtime with IR live.
pub fn eval(&self, form: &Form, env: &mut Env) -> EvalResult;
pub fn call_cljrs_fn(&self, f: &CljxFn, args: &[Value], env: &mut Env) -> EvalResult;
pub fn on_fn_defined(&self, f: &CljxFn, env: &mut Env);

/// Copy `src_ns`'s interns into `dst_ns` as refers.  Both are *explicit*
/// refers (`(:require [x :refer :all])` / `:refer [...]`) and are never
/// narrowed by `dst_ns`'s `ReferClojureFilter` — matching
/// `clojure.core/refer`, where naming a namespace explicitly re-maps even
/// names an earlier `refer-clojure` left out.
pub fn refer_all(&self, dst_ns: &str, src_ns: &str);
pub fn refer_named(&self, dst_ns: &str, src_ns: &str, names: &[Arc<str>]);

/// The automatic `clojure.core` refer every namespace starts with, narrowed
/// by `dst_ns`'s `ReferClojureFilter`.  This — not `refer_all` — is what
/// runtime init, the loader, `in-ns`, `ns` and versioned-namespace setup use
/// to seed a namespace with core.
pub fn refer_core(&self, dst_ns: &str);

/// Install (or, with `None`, remove) the `(:refer-clojure ...)` filter for
/// `dst_ns` and re-apply the automatic core refer under it.  Refers inherited
/// from `clojure.core` are dropped and re-added under one lock, so a filter
/// installed after the namespace was pre-referred takes effect without
/// exposing a half-referred namespace to a concurrent reader.  Errors when
/// the filter names something `clojure.core` does not define (`:only`,
/// `:rename`) or would refer two core names under one local name.
pub fn set_refer_clojure_filter(
    &self,
    dst_ns: &str,
    filter: Option<ReferClojureFilter>,
) -> Result<(), String>;
```

`:exclude` stays permissive where `:only`/`:rename` are validated: it is
subtractive, so excluding a name core does not define is harmless and keeps a
file portable across core versions.  The collision check is stricter than Clojure,
which warns (`Namespace.checkReplacement`) and then lets whichever mapping
`(keys (ns-publics ...))` yields last win.

### `depth` submodule

The call-depth cap for `ExecutionMode::NoGcTransaction`. Every interpreted
application consumes real Rust stack, so an unbounded recursion inside a
transaction would overflow the host thread's stack and abort the process.
`DepthGuard::install(limit)` scopes a thread-local budget to one invocation;
`call_cljrs_fn` refuses to nest past it and returns
`EvalError::Runtime(DEPTH_EXCEEDED_MSG)`. This is the one call-path override
that survived Stage 3 — as a runtime-owned mode rather than a `GlobalEnv` hook
installed by `cljrs-tx`.

### `error` submodule

`EvalError` / `EvalResult` are the evaluator's error types. Helpers:

- `EvalError::to_error_value(self) -> Value` — convert an error into a Clojure
  error *value*; `Thrown` is returned unchanged, anything else is wrapped in a
  fresh `ExceptionInfo`
- `value_error_to_eval_error(err: ValueError) -> EvalError` — surface a builtin's
  `ValueError` as a *catchable* `EvalError::Thrown(Value::Error(..))`, preserving
  the original variant and its plain message (no `runtime error:` prefix) so
  `(catch :default e ..)` / `ex-message` / `ex-data` behave the same as for a
  user `throw` / `ex-info`. A `ValueError::Thrown` re-surfaces the exact value.

### `gas` submodule

Cooperative execution-credit metering shared dynamically across tree-walker,
IR-interpreter, and JIT callbacks. `GasMeter::new(credits)` creates a shared
budget, `GasGuard::install(meter)` scopes it to the current evaluation thread,
`active_meters() -> Vec<Arc<GasMeter>>` and `install_meters(&[Arc<GasMeter>])`
propagate complete nested scopes to async polls, `charge(cost) -> bool` consumes
an all-or-nothing checkpoint, and
`take_exhausted() -> bool` transfers a native-tier exhaustion signal back to
the evaluator. `EvalError::GasExhausted` is the dedicated caller-facing error.
Exhaustion state is scoped per guard, so an exhausted inner evaluation cannot
poison a healthy outer evaluation after the inner guard drops.

### `policy` submodule

Dynamic capability policy used by isolated transaction functions.
`TransactionPolicyGuard::install()` denies filesystem and output operations,
clocks, randomness, process-global mutable facilities, blocking/concurrency,
versioned namespace loading, and Rust object construction. `check_native`,
`check_special`, and `check_versioned_lookup` are enforced at the interpreter's
final dispatch seams. `next_transaction_gensym()` supplies an invocation-local
deterministic sequence for syntax-quote hygiene. Violations return
`EvalError::ForbiddenEffect(String)`.

### `vcs` submodule

The runtime's seam to version control. Everywhere the interpreter would touch
git — locating a source file's repository, reading a file out of history,
checking a commit signature — it goes through a `VcsProvider` trait object held
by `GlobalEnv`, rather than calling `cljrs-project::vcs` directly. That is what
lets the `deps` feature (see [Features](#features)) drop gitoxide, rPGP and
`ssh-key` from an embedding without the rest of the runtime noticing.

```rust
pub trait VcsProvider: Send + Sync {
    /// Walk up from `start` to the enclosing git working-tree root.
    fn find_repo_root(&self, start: &Path) -> Option<PathBuf>;
    /// Read `rel_path` (relative to `repo_root`) as of `commit`.
    fn file_at_commit(&self, repo_root: &Path, rel_path: &str, commit: &str)
        -> Result<String, String>;
    /// Ok only when `commit` carries a valid signature by a trusted key.
    fn verify_commit_signature(&self, repo_root: &Path, commit: &str)
        -> Result<(), SignatureFailure>;
    /// Install the trusted-signer key set, replacing any previous one;
    /// returns the number of keys loaded.
    fn load_trusted_signers(&self, signers: &[TrustedSigner]) -> usize;
}

/// Untrusted → EvalError::CommitSignatureVerificationFailed;
/// Error     → EvalError::Runtime.
pub enum SignatureFailure {
    Untrusted { commit: String, reason: String },
    Error(String),
}

/// What `GlobalEnv::new` installs: `Some(ProjectVcs)` with the `deps` feature
/// on a native target, `None` otherwise.
pub fn default_provider() -> Option<Arc<dyn VcsProvider>>;

/// `cljrs-project`-backed implementation (`deps` feature, non-WASM).  Owns the
/// trusted-key set, which used to be the `GlobalEnv::trusted_keys` field.
pub struct ProjectVcs { /* … */ }
```

On `GlobalEnv`:

- `vcs(&self) -> Option<Arc<dyn VcsProvider>>` — the installed provider.
  Callers must degrade gracefully: `None` means "this file is not in a git
  repository".
- `set_vcs_provider(&self, Option<Arc<dyn VcsProvider>>)` — supply your own
  implementation, or pass `None` to strip git access from a sandboxed host.
  Drops every cached signature verdict: those were reached by the outgoing
  provider under its trust set, so carrying them over would let a permissive
  provider launder an approval for source a stricter one would reject.
- `invalidate_signature_cache(&self)` — forget those verdicts explicitly.
  `load_trusted_signers` calls it too, since a new key set invalidates
  conclusions drawn under the old one.

Callers: `loader::do_load` (records a namespace's repo root),
`versioned::versioned_source_available` and `versioned::fetch_versioned_source`,
and `GlobalEnv::check_commit_signature` / `load_trusted_signers`.

### `versioned` submodule (non-WASM)

Shared versioned-symbol/namespace resolution service used by **every**
execution tier (tree-walker, IR interpreter, JIT/AOT `rt_load_global*`
bridges). Resolving `ns/name@commit` ensures the immutable versioned
namespace `"ns@commit"` is loaded — from an embedded builtin source first,
falling back to fetching the file from git history — then performs a plain
`lookup_in_ns("ns@commit", name)`. Native (Rust-backed) symbols with no
Clojure source fall back to the HEAD implementation. Public API:

- `resolve_versioned_value(globals, defining_ns, ns_part, name, commit) -> EvalResult<Value>`
  — full resolution: alias handling, lazy namespace load, native HEAD fallback
- `ensure_versioned_ns_loaded(globals, base_ns, commit) -> EvalResult<Arc<str>>`
  — idempotent load of `"base_ns@commit"` (same cycle/cross-thread coordination
  as the unversioned loader); returns the versioned namespace name
- `base_ns_name(ns: &str) -> &str` — strip a trailing `@<commit>` suffix

Sources fetched from git are recorded in `GlobalEnv::versioned_sources`
(`record_versioned_source` / `versioned_sources_snapshot`) so the AOT
compiler can embed them in produced binaries.
`pin_if_available(globals, base_ns, commit) -> EvalResult<bool>` is the AOT
discovery hook: force-loads a pin when its source is locatable, skips
otherwise.  `GlobalEnv::set_versioned_offline(true)` (called by AOT harness
binaries) restricts versioned loading to embedded sources — a missing
embedding fails with a clear "was not embedded at compile time" error
instead of fetching from git.

Native (Rust-backed) packages get a **verified HEAD binding**: the fallback
checks the pin against `GlobalEnv::native_provenance` (recorded via
`set_native_provenance` / `Registry::set_provenance`; prefix-match in either
direction for abbreviated hashes).  Mismatching or missing provenance warns
once per pin (`provenance_warned`), or errors when
`set_enforce_native_versions(true)` is set (`--enforce-native-versions`,
cljrs.edn `:enforce-native-versions`).

Opt-in pinned native code: `GlobalEnv::set_pinned_native_loader` installs a
`PinnedNativeLoader` callback (provided by the CLI); the resolver
consults it before the HEAD fallback, and a successful load redirects the
lookup into the freshly registered `"<ns>@<commit>"` namespace.

Plain `require` of a native dep: `GlobalEnv::set_native_require_loader`
installs a `NativeRequireLoader` callback (also provided by the CLI).
The unversioned namespace loader (`loader::do_load`) consults it when a
`require`d namespace has no Clojure source on the source path; a successful
load registers a `:rust/load :dylib` dep's exports into the **unversioned**
namespace (built at the dep's pinned `:git/sha`), so a plain
`(require '[my.native.lib :as l])` of a pure-native package succeeds.

AOT-compiled namespaces: the binary produced by `cljrs compile` registers a
`CompiledNsLoader` per required namespace via
`GlobalEnv::register_compiled_ns_loader`.  `loader::do_load` checks
`GlobalEnv::compiled_ns_loader` **first** — before builtin source, disk, and
native fallbacks — and, when one is present, runs it instead of interpreting
Clojure source.  The loader evaluates the namespace's small interpreted
preamble (its `ns`/`require` form and any `defmacro`/protocol/multimethod
definitions) and then calls the namespace's natively compiled initializer, so
the bulk of a required namespace runs as machine code rather than being
tree-walked at startup.  `*ns*` is saved/restored around the loader so the
caller's namespace is undisturbed.

### `gc_roots` submodule

Manages GC root registration for the interpreter's Rust call stack. Public API:

- `push_env_root(env: &Env) -> EnvRootGuard` — registers an `Env` pointer as a GC root; guard removes on drop
- `root_value(val: &Value) -> ValueRootGuard` — registers a single `Value` pointer as a GC root
- `root_values(vals: &[Value]) -> ValueRootGuard` — registers a slice of `Value` pointers as GC roots
- `root_option_values(vals: &[Option<Value>]) -> OptionValueRootGuard` — registers an `Option<Value>` slice (e.g. IR register file)
- `gc_safepoint(env: &Env)` — interpreter-level safepoint: parks if collection in progress, or initiates collection on memory pressure
- `force_collect(env: &Env)` — immediately initiates a GC collection bypassing memory-pressure threshold
- `async_gc_collect()` — services a pending GC request from a Tokio `LocalSet` task at a cooperative yield point; safe to call when no other tasks are polling, so thread-local root stacks are stable and fully describe all suspended-task `GcPtr`s
- `set_stw_reclaim_hook(f)` — registers a stop-the-world reclaim hook; multiple hooks may be registered and each runs (in registration order) inside the STW guard at the tail of every collection (`force_collect`, `gc_safepoint`, `async_gc_collect`), when all mutator threads are parked.  Registrants: the compiler's JIT code cache frees superseded native code (Phase 10.2); the `tiered` lowering worker sweeps idle Tier-1 IR (Phase 10.7)

Root tracing covers all namespaces (including immutable `ns@commit`
namespaces) **and** the values in `GlobalEnv::version_cache`, so versioned
values that exist only in the cache (native HEAD fallbacks) survive
collection.

### `apply` submodule

`apply_value` applies an evaluated callee to evaluated args (functions,
keywords, maps, sets, vars, protocol/multimethod dispatch). For a
`Value::ProtocolFn` callee whose protocol has `extend_via_metadata` set (`(defprotocol
Name :extend-via-metadata true ...)`), dispatch first checks the first arg's
metadata for an entry keyed by the `ProtocolFn` itself (e.g. `(with-meta {}
{my-method (fn [this] ...)})`) before falling back to the type-tag `impls`
lookup — this lets a value implement a protocol without a matching
`extend-type`/`extend-protocol`. Protocol dispatch helpers shared with the
Phase 10.6 inline caches:

- `type_tag_of(val: &Value) -> Arc<str>` — canonical protocol dispatch tag of a value
- `type_tag_matches(val: &Value, tag: &str) -> bool` — allocation-free equality
  against a cached tag; must agree exactly with `type_tag_of` (used by
  `rt_call_ic`'s hot path in `cljrs-compiler`)
- `dispatch_if_async(callee, args, env)` — spawn `^:async` callees on the async runtime

### `callback` submodule

Thread-local eval context for Rust→Clojure callbacks (`invoke`, `with_eval_context`). The context is pushed automatically around native builtin calls and by the Tier-1 IR executor; rt_abi bridges (`rt_call`, `rt_load_global`, the HOF bridges) dispatch through it. Public API includes:

- `push_eval_context(env: &Env)` / `pop_eval_context()` — bracket a native call with the current env's globals + namespace
- `capture_eval_context() -> Option<(Arc<GlobalEnv>, Arc<str>)>` — snapshot the innermost context (e.g. to hand to another thread)
- `install_eval_context(globals, ns)` — push a previously captured context (spawned threads)
- `install_eval_context_guard(globals, ns) -> EvalContextGuard` — like `install_eval_context`, but pops on drop (including unwind); used by the JIT-native dispatch seam
- `current_is_async() -> bool` — whether the innermost context is inside an `^:async` body
- `invoke(f: &Value, args: Vec<Value>) -> ValueResult<Value>` — call a Clojure-callable value through the innermost context. Honors `^:async` dispatch (via `apply::dispatch_if_async`) so a native/compiled caller of an `^:async` fn gets a `Value::Future`, not a synchronously-run body
- `with_eval_context(f)` — run a closure with a temporary `Env` built from the innermost context

### `async_hook` submodule

The optional async-runtime seam (`AsyncRuntime` trait, installed by `cljrs-async`).  Async-JIT activation is *not* here: the dispatcher reaches it through the calling runtime's own backend (`GlobalEnv::jit_backend()` → `JitBackend::compile_async_arity`), so a runtime without a JIT simply keeps tree-walking `^:async` bodies.

---

## Module `builtins`

The `clojure.core`-equivalent runtime implemented in Rust, registered into a
name → fn dispatch table by `builtins::register_all(&globals, ns)`.
`BOOTSTRAP_SOURCE` (`bootstrap.cljrs`) and `CLOJURE_TEST_SOURCE`
(`clojure_test.cljrs`) are the embedded Clojure sources evaluated on top of it.

### Map entries

Map entries are a dedicated type, not plain 2-element vectors: seq'ing a map,
`find`, and the `map-entry` constructor produce vectors tagged as entries
(`PersistentVector::map_entry` in `cljrs-value`).

- `(map-entry k v)` / `(map-entry coll)` — build an entry from a key and
  value, or from any seqable of exactly two elements.
- `(map-entry? x)` — true only for real entries; `(map-entry? [:a 1])` is
  false.
- `key` / `val` (bootstrap) — accept only real map entries and throw
  otherwise.

Entries otherwise behave exactly like 2-element vectors (equality, hashing,
printing, `nth`, destructuring), and, as in Clojure, any vector derived from
an entry (`conj`, `assoc`, `pop`, `subvec`, ...) is a plain vector again.

### Unchecked arithmetic

Includes the `unchecked-*` integer arithmetic family — `unchecked-add`,
`unchecked-subtract`, `unchecked-multiply`, `unchecked-inc`, `unchecked-dec`,
`unchecked-negate` (and their `-int` aliases) — which wrap on overflow, in
contrast to `+`/`-`/`*`, which promote overflowing Long results to BigInt at
every tier.

### Docstrings (`doc` / `doc-data`)

`register_all` attaches `:doc` var metadata to native builtins from the
`BUILTIN_DOCS: &[(&str, &str)]` table (in `builtins.rs`), keyed by the name
the builtin is interned under. Not every builtin has an entry — special-form
stub vars and rarely-used internals are skipped, and a builtin later
redefined in `bootstrap.cljrs` (e.g. `swap!`, `partition`, `range`) carries
its docstring there instead, since the Clojure-level `defn`/`defmacro`
re-interns the var (see the `interp` module below for how `def`/`defn`/
`defmacro` capture docstrings into var meta). Any builtin *may* carry a
docstring simply by adding a `BUILTIN_DOCS` entry; `#[cfg(test)] mod
doc_tests` in `builtins.rs` asserts every entry names something actually
registered, and that there are no duplicate names.

`doc-data` (`builtin_doc_data`, registered as a native fn) takes a `Var`
(`#'foo`), a value carrying attached metadata (`with-meta`), or a bare
function value, and returns `{:doc <string-or-nil> :arities <vector-or-nil>}`.
`:arities` prefers `:arglists` var metadata when present (real parameter
names, from `def`/`defn`/`defmacro`); otherwise it synthesizes placeholder
parameter names (`arg1`, `arg2`, ...) from a native fn's `Arity` shape, since
native fns don't carry real parameter names.

`clojure.core/doc` (a macro, defined in `bootstrap.cljrs`) wraps `(var sym)` +
`doc-data` in a `try`/`catch` so `(doc some-unbound-symbol)` returns `nil`
instead of throwing, and returns just the `:doc` string.

### Reader-conditional resolution (`form.rs`)

The reader is platform-agnostic: it parses every branch of `#?(...)` / `#?@(...)`
and hands back a `FormKind::ReaderCond` node. Selecting the `:rust` branch is
therefore the job of each form-consuming boundary, and this module holds the
calculations they share.

```rust
/// The `:rust` branch of a conditional's clauses, or the `:default` branch.
pub fn select_reader_cond(clauses: &[Form]) -> Option<&Form>;

/// Expand `#?`/`#?@` across a sibling slice: a non-splicing conditional
/// becomes its selected branch (or is dropped), a splicing one contributes
/// that branch's elements inline.
pub fn expand_reader_conds(forms: &[Form]) -> Vec<Form>;

/// As above, borrowing the input unchanged when it holds no conditional.
pub fn expand_reader_conds_cow(forms: &[Form]) -> Cow<'_, [Form]>;

/// A slice that gets chunked by two was left with an odd number of forms.
pub struct OddArity(pub usize);

/// Expand, then require even length. Used by every construct that chunks
/// siblings into pairs - map literals and `let*`/`loop*`/`binding` vectors,
/// in both evaluators - since a splice's contribution is branch-dependent
/// and the written parity does not decide the expanded parity.
pub fn expand_pairs(forms: &[Form]) -> Result<Cow<'_, [Form]>, OddArity>;

/// Convert a form to the value it denotes, without evaluating. Resolves
/// conditionals in every container arm. Errors on a map whose expansion has
/// odd length, and on a `#?@` with no sibling sequence to splice into.
pub fn form_to_value(form: &Form) -> EvalResult<Value>;
```

Callers phrase `OddArity` in their own words (`map literal must have an even
number of forms`, `let* binding vector must have even length`, ...), so the
parity rule lives here while the message stays at the boundary.

### Reader metadata on values

`form_to_value` attaches a `^meta` annotation to the value it denotes, so
`(meta '^{:x 1} [1])` answers `{:x 1}` as on the JVM. Shorthands expand as the
reader does (`^:kw` → `{:kw true}`, `^Sym` → `{:tag Sym}`), stacked
annotations merge with the outer one winning (`merge_meta_values`), and values
that cannot carry metadata (`supports_meta` — the JVM's `IObj`) drop it rather
than growing a wrapper. A nil annotation carries nothing, and
`(with-meta x nil)` clears metadata instead of storing a nil-meta wrapper.

### Phase B3 — `shared-atom` (cross-isolate, two-tier atom ADR)

`shared-atom` is the cross-isolate tier of the two-tier atom design in
`docs/async-worker-pool-plan.md`.  Unlike `atom` (isolate-local, GC-backed),
its contents are promoted to a `Send + Sync` `SharedValue`
(`cljrs_value::shared`) behind a lock-free `ArcSwap`, so the reference can cross
the isolate boundary and be mutated concurrently:

- `(shared-atom x)` — construct, promoting `x` (non-promotable values such as
  closures and native resources are rejected here).
- `(shared-atom? x)` — predicate.
- `deref` / `reset!` / `swap!` / `compare-and-set!` — dispatch on
  `Value::SharedAtom` alongside the local `atom` path; writes promote, reads
  demote, and `swap!`/`compare-and-set!` use a single lock-free CAS with retry.

---

## Module `interp`

Self-contained tree-walking interpreter for Clojure.

**Phase:** Core interpreter — implemented.  `no-gc` region/static-sink support (Phases 4–5), blacklist integration (Phase 6), and integration tests (Phase 8) of `docs/archive/no-gc-plan.md` — implemented.

Evaluates Clojure `Form` ASTs produced by `cljrs-reader`, managing lexical
environments, special forms, function application, and the recur trampoline.

Allocations are scoped per function call and per loop iteration: under GC, each
trampoline iteration (`call_cljrs_fn`, `eval_loop`) runs inside its own
`cljrs_gc::push_alloc_frame()`, so that iteration's intermediates — and a
`recur`'s now-dead values — become collectable when the frame drops, instead of
being pinned in `ALLOC_ROOTS` for the lifetime of the enclosing top-level form.
The return value / recur args are moved out before the frame drops and re-rooted
at the next iteration (or by the caller on return); no GC safepoint runs in the
interval (GC fires only at explicit safepoints, with a one-cycle grace period —
see `cljrs-gc`). Under the `no-gc` Cargo feature the same scoping is achieved
with the allocation-context stack protocol (scratch regions for function/loop
scopes; `StaticArena` for static-sink expressions).
When the `env` module's transaction policy and `InvocationGuard` are active, the
same tree walker denies external capabilities and routes all allocations into
one invocation-lifetime region instead.

### `eval(form, env) -> EvalResult`

Evaluate a single `Form` in `env`.  Entry point for the interpreter.

### `eval_with_gas(form, env, credits) -> EvalResult`

Evaluate a form with a cooperative execution-credit budget. Tree-walker form
entries cost one credit; Tier-1 IR and JIT basic blocks use the same weighted
`phis + instructions + terminator` approximation. Exhaustion returns
`EvalError::GasExhausted`; ordinary `eval` calls remain unmetered.

This is a cooperative mechanism and currently a host API rather than a CLI or
nREPL policy. Native builtins that do substantial work without re-entering the
evaluator may consume fewer credits than equivalent interpreted code. Compiled
code emits a checkpoint call at every basic block even when no meter is active;
avoiding that always-on JIT cost requires a future metering-mode fast path.

### `eval_call(func_form, arg_forms, env) -> EvalResult`

Evaluate a function-call form.  Handles macros, native-function special cases,
and user-defined `CljxFn` application with the recur trampoline.

### `eval_body(forms, env) -> EvalResult`

Evaluate a sequence of forms, returning the value of the last one.

### `eval_loop(args, env) -> EvalResult`

Evaluate a `loop*` form.  Each iteration is scoped in its own allocation frame
so intermediate allocations are freed per iteration: under GC a
`cljrs_gc::push_alloc_frame()` that drops at the end of the iteration; under
`no-gc` a `ScratchGuard` popped before the tail expression (recur args or return
value).

### `eval_defn(args, env) -> EvalResult`

Evaluate a `defn` form.  Accepts metadata on the name (`(defn ^:async f …)`) and
an attr-map (`(defn f {:async true} …)`); `^:async` marks the resulting `CljxFn`
as async.  Under `no-gc`, wraps fn creation in `StaticCtxGuard` so the `CljxFn`
object lands in the `StaticArena`.

### Docstring / `:arglists` metadata (`def`, `defn`, `defmacro`)

`eval_def`, `eval_defn`, and `eval_defmacro` all recognize an optional
docstring positional arg (`(def name "doc" val)`, `(defn name "doc" [..] ..)`,
`(defmacro name "doc" [..] ..)`) and store it as `{:doc "..."}` in the
resulting Var's metadata, merged with any reader/attr-map metadata via
`merge_meta`.  `defn`/`defmacro` additionally derive `{:arglists (...)}` from
the evaluated `CljxFn`'s parsed arities (`arglists_meta`, in `special.rs`);
for `defmacro` the implicit `&form`/`&env` params are elided from the shown
signature.  This is what `clojure.core/doc` and `doc-data` (in the `builtins`
module) read back, and what `cljrs-nrepl`'s `op_lookup` surfaces to editors.

### `meta_form_is_async(meta: &Form) -> bool`

Returns true when a `^meta` form (or attr-map literal) requests `:async` — either
the keyword shorthand `^:async` or an explicit `{:async true}` map.  `fn`/`defn`
use it to set `CljxFn::is_async`, which `env::apply::dispatch_if_async`
checks at call time to route through the async runtime.

### Special handlers in `apply.rs`

Each handler evaluates its key expressions under the correct allocation context:

| Handler | Static-sink guard coverage |
|---|---|
| `handle_atom_call` | initial value |
| `handle_reset_bang` | new value |
| `handle_swap_call` | function return value |
| `handle_volatile` | initial value |
| `handle_vreset` | new value |
| `handle_vswap` | function return value |
| `handle_agent_call` | initial value |
| `handle_alter_var_root` | function return value |
| `handle_intern` | value expression (3-arg form) |

### Value-level special form helpers (IR interpreter API)

The IR interpreter receives already-evaluated `Vec<Value>` arguments rather than
`&[Form]` AST nodes.  These public functions mirror the `handle_*` form-level
handlers but accept pre-evaluated args, allowing the IR interpreter to
implement sentinel operations without hitting the stub errors registered in
`clojure.core`:

| Function | Operation |
|---|---|
| `eval_swap_bang(args, env)` | `swap!` — apply f to atom, store result |
| `eval_volatile(args)` | `volatile!` — create a new volatile |
| `eval_vreset_bang(args)` | `vreset!` — reset volatile value |
| `eval_vswap_bang(args, env)` | `vswap!` — apply f to volatile value, store result |
| `make_delay_from_fn(f, globals, ns)` | `make-delay` — wrap zero-arg fn in a `Delay` |
| `eval_alter_var_root(args, env)` | `alter-var-root` — apply f to var root, store result |
| `eval_vary_meta(args, env)` | `vary-meta` — apply f to obj metadata |
| `eval_with_bindings_star(args, env)` | `with-bindings*` — push binding frame, call f |
| `eval_send_to_agent(args, env)` | `send` / `send-off` — dispatch action to agent |
| `dispatch_method(method, target, args)` | `(.method target args…)` — interop method dispatch on an evaluated target (strings, vectors, seqs) |

`make_lazy_seq_from_fn(f, globals, ns)` (already public) creates a `LazySeq`
from a zero-arg callable; the above `make_delay_from_fn` is the analogous
helper for `Delay`.

### `special.rs` notes

`parse_arity` peels primitive type hints (`^long x`, `^doubles a`) off params
into `CljxFnArity::param_hints`; `let*`/`loop*` binding hints are stripped via
`bind_pattern`'s `Meta` arm (`destructure.rs`); `desugar_pre_post_conditions`
rewrites `{:pre [...] :post [...]}` maps into assertion forms (binding `%` to
the return value in `:post` conditions); `spec_element` resolves a reader
conditional in ANY slot of an `ns` require spec, namespace included, so
`[#?(:clj clojure.core :cljs cljs.core) :as core]` reads — an option selecting
no branch is dropped, a namespace selecting none is an error.

---

## Module `tiered`

IR-accelerated evaluation. Wraps the tree-walking interpreter in `interp` with
IR lowering and interpretation for faster function execution.

**Phase:** IR tier-1 interpreter — implemented.

When a Clojure function has been lowered to IR — by the warm-threshold
background lowering worker (Phase 10.7, the default), eagerly at definition
time via the `on_fn_defined` hook (`CLJRS_EAGER_LOWER=1`), or from a pre-built
cache — calls are dispatched to the tier-1 IR interpreter. Otherwise they fall
back to the tree-walking interpreter.

Lowering itself is pure Rust (`cljrs_ir::lower`); the `lower` submodule here
orchestrates macro expansion (interpreter) and the Env-free lowering half.

### Public API

```rust
/// Re-exports from the interp and env modules:
pub use crate::env::env::{Env, GlobalEnv};
pub use crate::env::error::{EvalError, EvalResult};
pub use crate::interp::eval::{eval, eval_with_gas};
pub use crate::env::callback::invoke;
pub use crate::env::loader::load_ns;

/// Load pre-built IR from a serialized bundle into the IR cache.
/// Walks all namespaces, matches bundle keys to runtime arity IDs.
/// Returns the number of arities loaded.
pub fn load_prebuilt_ir(globals: &Arc<GlobalEnv>, bundle: &IrBundle) -> usize;

/// IR lowering helpers (in submodule `lower`):
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
        destructure_rest, expanded_body, ns, globals_id: u64,
        arity_id: Option<u64>, do_optimize: bool, is_async: bool)
        -> Result<(IrFunction, Vec<(Arc<str>, Arc<str>)>), LowerError>;
}

/// Cross-defn IR registry (in submodule `defn_registry`, Phase 10.5):
/// `globals_id` is `GlobalEnv::id` — a counter value, so a dropped runtime
/// never leaks its registrations to the next one built.
pub mod defn_registry {
    pub fn register_defn(globals_id: u64, ns, name, arities: Vec<(usize, bool, Arc<IrFunction>)>);
    pub fn externals_for(globals_id: u64, referenced) -> Vec<ExternalDefn>;
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

### IR dispatch flow

1. `GlobalEnv::call_cljrs_fn` routes to `tiered::apply::call_cljrs_fn` when the runtime's
   `ExecutionMode` is `Tiered` or `TieredNoJit`
2. On each call, it checks `globals.ir_cache().get(arity_id)` for a lowered IR function
3. If cached **and not async**: executes via `ir_interp::interpret_ir` (register-file interpreter)
4. If not cached **or async**: counts the call (`jit_state::record_interp_call`, Phase 10.7 —
   see "Background lowering" below) and falls back to `interp::apply::call_cljrs_fn`
   (tree-walking).  For `^:async` functions the tree-walking path dispatches to `eval_async`
   in `cljrs-async`, which cooperatively yields to the Tokio `LocalSet` executor.
5. How IR gets into the cache:
   - **Warm-threshold background lowering (default, Phase 10.7)**: when a function's
     tree-walked call count crosses `ir_threshold()` (default 50), the dispatch seam
     macro-expands its arity bodies on the calling thread and enqueues them to the
     `cljrs-ir-lower` worker, which lowers + optimizes off-thread and publishes via
     `IrCache::store`.
   - **Eager lowering (opt-in, `CLJRS_EAGER_LOWER=1`)**: `GlobalEnv::on_fn_defined`
     calls `ir_interp::eager_lower_fn` for a tiered runtime whose tier state has
     reached `Ir`, so new `fn*` definitions are lowered immediately.
   - **Pre-built bundles**: `load_prebuilt_ir` — public API for embedders, called by
     nothing in this workspace. `cljrs ir build` writes the bundles it consumes.
   The resulting `IrFunction::is_async` flag matches the `CljxFn::is_async` attribute.
6. `eval_call` in `interp` routes `Value::Fn` calls through `GlobalEnv::call_cljrs_fn`
   rather than calling the tree-walker directly, so IR-cached arities are used on
   direct call paths too
7. JIT tier: before the IR cache, and only when the tier state is `Jit`
   (`ExecutionMode::Tiered`), `call_cljrs_fn` checks `jit_state::get_native_fn(arity_id)`
   for compiled native code and, if present, dispatches to it.  `call_jit_native` brackets
   the native call with: a frame epoch (code unloading), GC roots for the caller env and
   args, **an eval context** (rt_abi bridges — `rt_call`, `rt_load_global`, the HOF
   bridges — dispatch through `env::callback`; without it they silently return nil),
   and an alloc frame.  After the call it takes any pending exception stashed by an
   uncaught native `(throw …)` and re-raises it as `EvalError::Thrown` (same in
   `try_osr_enter` for OSR entries)

### Background lowering & cold-IR eviction (Phase 10.7)

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
  (arity id below the watermark the runtime builder snapshots), and
  fns defined in builtin-source namespaces (clojure.test, clojure.string, …).  Background lowering targets **user code only**:
  shipped namespaces only ever reached the IR tiers under opt-in eager
  lowering, and some of their patterns are known to miscompile (see TODO.md
  Phase 10.7 notes).
- Rebind safety: `snapshot_externals` records dependent edges atomically with
  reading the registry, and the worker is the only consumer of relower marks —
  after `store_cached` it re-peeks the mark and re-lowers (≤3 attempts) if a
  rebind landed mid-flight.  The dispatch seam only peeks
  (`relower_marked` + `lower_queued` dedup) and enqueues.
- Cold eviction: `Cached` entries track last access; `ir_cache::sweep_idle`
  runs at the stop-the-world reclaim pass over every live runtime's cache and
  evicts entries idle past
  `CLJRS_IR_CACHE_TTL` (default 600 s) — deliberately *colder* than native
  code.  Entries backing published native code or an in-flight compile are
  never evicted (deopt fallback); `Unsupported` markers are kept forever.
  Eviction drops the `JitEntry` (the fn can re-warm) and stales any OSR code.
- Knobs: `CLJRS_IR_THRESHOLD` / `set_ir_threshold` / `--ir-threshold N`
  (0 disables background lowering), `CLJRS_IR_CACHE_TTL`, `CLJRS_NO_IR`
  (kills all IR), `CLJRS_EAGER_LOWER=1` (restores eager lowering — also the
  escape hatch for the known limitation that a long-running loop entered at
  Tier 0 cannot tier up mid-call, since the tree-walker has no OSR).

### Per-runtime tier state (`tiers`)

`Tiers` is the Tier-1 + Tier-2 state of one runtime, owned by its `GlobalEnv`
and reached through it:

```rust
pub struct Tiers { /* IrCache + JitState */ }

impl Tiers {
    pub fn globals_id(&self) -> u64;               // identity of the owning runtime
    pub fn ir_cache(&self) -> &IrCache;            // Tier-1 lowered IR
    pub fn jit(&self) -> &JitState;                // Tier-2 counters + native code
    pub fn handle(&self) -> Weak<Tiers>;           // for a background worker's request
    pub fn sweep(&self, now, ttl_secs) -> Vec<u64>;// TTL evict IR + drop its JIT bookkeeping
}

pub fn live() -> Vec<Arc<Tiers>>;                  // every live runtime's tier state
pub fn sweep_idle(now, ttl_secs) -> Vec<u64>;      // STW reclaim hook: sweep them all
```

`GlobalEnv::tiers()`, `::ir_cache()`, `::jit()`, and `::jit_backend()` are the
accessors dispatch uses.  Two runtimes in one process never read, evict,
promote, or deoptimize each other's code, and dropping one releases its IR
and stales its published native code (`^:async` poll functions excepted —
they are registered outside the epoch-tagged code cache and live for the
process).

Background workers cannot hold an `Arc<GlobalEnv>` (it owns `GcPtr`s and is
not `Send`); `Tiers` is `Send + Sync`, so a lowering or compile request
carries `Tiers::handle()` and the worker publishes into exactly the runtime
that asked — or finds it dropped and discards the result.  `live()` exists for
the one path with no runtime in hand at all: `Var::bind`'s rebind
notification.

### The JIT seam (`backend`)

```rust
pub trait JitBackend: Send + Sync {
    fn enqueue_function(&self, tiers: Weak<Tiers>, arity_id: u64, ir: Arc<IrFunction>);
    fn enqueue_osr(&self, tiers: Weak<Tiers>, arity_id: u64, header: u32, ir: Arc<IrFunction>);
    fn mark_stale(&self, epoch: u64);
    fn take_pending_exception(&self) -> Option<Value>;
    fn deopt_sentinel(&self) -> usize;
    fn compile_async_arity(&self, callee: &Value, nargs: usize, env: &mut Env);
}
```

One optional-system seam, installed per runtime by
`cljrs_compiler::jit::install(&runtime)` (`JitState::install_backend`).  It
replaces the five process-global `OnceLock` hooks this module used to carry
(enqueue, OSR enqueue, stale-epoch, pending-exception, deopt-sentinel) plus
the async-compile hook in `env::async_hook`: the *state* those hooks reached
is now runtime-owned, and only the call into the compiler remains — which is
unavoidable, since `cljrs-compiler` depends on this package.

### JIT state & code unloading (`jit_state`)

`JitState` is one runtime's Tier-2 state.  Public surface (methods on
`GlobalEnv::jit()`):

```rust
pub fn install_backend(&self, backend: Arc<dyn JitBackend>);
pub fn backend(&self) -> Option<&Arc<dyn JitBackend>>;
pub fn record_interp_call(&self, arity_id) -> bool;      // Tier-0 accounting; true = snapshot+enqueue
pub fn lower_queued(&self, arity_id) -> bool;            // dedup gate for the warm/relower paths
pub fn mark_lower_queued(&self, arity_id);               // set on accepted enqueue
pub fn clear_lower_queued(&self, arity_id);              // worker re-arms after abandoning an arity
pub fn on_ir_published(&self, arity_id);                 // worker: restart counter at IR publish
pub fn evict_entry_if_cold(&self, arity_id) -> bool;     // TTL sweep: drop entry unless native/queued
pub fn stale_osr_code(&self, arity_id);                  // TTL sweep: stale published OSR entries
pub fn compile_queued(&self, arity_id) -> bool;          // TTL sweep: in-flight JIT needs the IR
pub fn pins_ir(&self, arity_id) -> bool;                 // TTL sweep: native published or compile queued
pub fn set_bootstrap_watermark(&self, w: u64);           // the runtime builder snapshots the boundary
pub fn is_bootstrap_arity(&self, arity_id) -> bool;      // bootstrap fns excluded from background lowering
pub fn record_call(&self, arity_id, ir_func, profile_args);  // bump counter + arg-type profile; enqueue when hot
pub fn arg_type_profile(&self, arity_id) -> Option<Vec<u8>>; // per-param bitmasks (PROFILE_LONG/_DOUBLE/_OTHER)
pub fn store_native_fn(&self, arity_id, ptr, epoch);     // worker publishes compiled code
pub fn get_native_fn(&self, arity_id) -> Option<(*const (), u64)>;  // (fn_ptr, epoch)
pub fn take_native_epoch(&self, arity_id) -> Option<u64>;// on redefinition: null ptr, drop entry, return epoch
pub fn stale_native_code(&self, arity_id);               // null ptr + hand epochs to the backend (10.5)
// Drop: hands every still-published epoch to the backend, so a dropped
// runtime's compiled modules are reclaimed instead of leaking in the
// process-global code cache.
pub fn take_pending_exception(&self) -> Option<Value>;   // uncaught native throw, taken at the dispatch seam

// Deoptimization (Phase 10.6):
pub fn is_deopt_result(&self, ptr: *const Value) -> bool;// dispatch seam: did the entry guard fail?
pub fn record_deopt(&self, arity_id);                    // count a guard failure; past deopt_limit():
                                                         // unpublish + stale the specialized code, ban
                                                         // the arity from re-specialization
pub fn specialization_allowed(&self, arity_id) -> bool;  // worker: may this arity be specialized?
```

Process-wide items in the same module — configuration and thread state, not
runtime state:

```rust
pub fn set_jit_threshold(t: u32) / jit_threshold() -> u32;   // calls before compile (default 1000)
pub fn set_ir_threshold(t: u32)  / ir_threshold() -> u32;    // Tier-0 calls before background lowering
                                                             // (default 50; u32::MAX disables)
pub fn set_osr_threshold(t: u32) / osr_threshold() -> u32;   // back-edges before an OSR compile
pub fn deopt_limit() -> u32;                                 // CLJRS_JIT_DEOPT_LIMIT (default 10)
pub fn push_jit_frame(epoch) -> JitFrameGuard;   // mark a native frame live for its call
pub fn current_jit_epoch() -> Option<u64>;       // innermost native frame's epoch (closure-escape pinning)
pub fn live_epochs() -> HashSet<u64>;            // epochs with a live frame (call at STW only)
pub unsafe fn dispatch_jit_call(fn_ptr, args) -> *const Value;
```

Frame tracking is per *thread*, not per runtime: one thread can hold native
frames from several runtimes, and reclamation asks whether any thread is
executing a module.

`call_jit_native` checks `is_deopt_result` on every native return: a
specialized function whose entry type guard failed returns the compiler's
sentinel *before any side effect*, so the seam simply re-executes the call at
Tier 1 (`execute_ir`) — exact interpreter semantics for the violating call.

Type profiles (Phase 10.6): `record_call` ORs each positional argument's type
class (`PROFILE_LONG` / `PROFILE_DOUBLE` / `PROFILE_OTHER`) into
`JitEntry::arg_profile` until the compile is queued; variadic arities profile
only the fixed prefix (the rest-list param is padded `PROFILE_OTHER` so it can
never be specialized).  The JIT worker reads the snapshot via
`arg_type_profile` to choose per-parameter specializations.

Each native call brackets itself with `push_jit_frame(epoch)` so the JIT code
cache can free a superseded module only once no frame is executing it
(`live_epochs` scanned at the stop-the-world GC safepoint).

### OSR — on-stack replacement (Phase 10.4)

A single hot call containing a `loop*`/`recur` never returns to re-dispatch, so
the invocation counter cannot promote it.  Instead:

1. `interpret_ir_with_osr` (the dispatch path used by `apply::execute_ir`,
   which passes the arity ID) counts back-edges per `RecurJump` target.  The
   counters are local to one execution on purpose: hot-within-one-call is
   exactly the case invocation tiering misses.
2. Crossing `osr_threshold()` calls `JitState::osr_request`, which enqueues
   `(arity_id, header_block, IrFunction)` on the runtime's backend exactly once.
3. The worker builds the OSR-entry variant (`cljrs_ir::osr::build_osr_function`),
   compiles it, and publishes `(fn_ptr, epoch, live_ins)` via `store_osr_fn`.
4. At each subsequent loop-header entry (after φ resolution, so the loop
   variables are current), the interpreter polls `osr_poll`; on `Ready` it
   snapshots the live-in registers and calls the native entry
   (`try_osr_enter`) — the native frame finishes the loop *and* the rest of
   the function, and its return value becomes the call's result.

OSR `JitState` surface (plus the process-wide `set_osr_threshold` /
`osr_threshold` above):

```rust
pub fn osr_request(&self, arity_id, header, ir_func);  // idempotent compile request
pub fn osr_poll(&self, arity_id, header) -> OsrPoll;   // NotRequested | Pending | Ready(OsrSlot) | Failed
pub fn store_osr_fn(&self, arity_id, header, ptr, epoch, live_ins);  // worker publishes
pub fn mark_osr_failed(&self, arity_id, header);       // worker declines; interpreters stop polling
pub fn take_osr_epochs(&self, arity_id) -> Vec<u64>;   // on redefinition: drop entries, return epochs
```

With no backend installed `osr_request` marks the header failed immediately,
so interpreters stop polling instead of waiting for a compile that will never
come.

`OsrSlot { fn_ptr, epoch, live_ins }` carries the interpreter registers to pass
(in parameter order); the transfer uses the same rooting + `push_jit_frame`
protocol as ordinary JIT-native calls.  Scratch regions opened before the loop
stay open across the transfer (the OSR variant drops their `RegionEnd`s) and
unwind with the interpreter frame.

### Special-form coverage in the IR interpreter

Several `clojure.core` entries are sentinel stubs that error unconditionally
when called through the normal function-call path — the real logic lives in
`eval_call`'s special-form dispatch.  `ir_interp.rs` handles all of them
without going through the stubs:

| Operation | How handled in IR |
|---|---|
| `swap!` (`KnownFn::AtomSwap`) | `interp::apply::eval_swap_bang` |
| `with-bindings*` (`KnownFn::WithBindings`) | `interp::apply::eval_with_bindings_star` |
| `volatile!` | `dispatch_sentinel_by_name` → `eval_volatile` |
| `vreset!` | `dispatch_sentinel_by_name` → `eval_vreset_bang` |
| `vswap!` | `dispatch_sentinel_by_name` → `eval_vswap_bang` |
| `make-delay` | `dispatch_sentinel_by_name` → `make_delay_from_fn` |
| `alter-var-root` | `dispatch_sentinel_by_name` → `eval_alter_var_root` |
| `vary-meta` | `dispatch_sentinel_by_name` → `eval_vary_meta` |
| `send` / `send-off` | `dispatch_sentinel_by_name` → `eval_send_to_agent` |
| `with-out-str` (`KnownFn::WithOutStr`) | native: `push_output_capture` → apply body thunk → `pop_output_capture` (the clojure.core var is a nil stub and must never be called) |
| `(.method target args…)` interop | `dispatch_sentinel_by_name` intercepts dot-prefixed `CallDirect` names → `interp::apply::dispatch_method` |

Both `Inst::Call` (where the callee register holds a sentinel `NativeFunction`)
and `Inst::CallDirect` (where the callee is named directly) are intercepted.

`load_global_value` additionally mirrors `eval_symbol`'s whole-symbol lookup:
when `(ns, name)` resolution fails, it retries `"{ns}/{name}"` in the defining
namespace (with the clojure.core refers fallback), so slash-named builtins
like `Math/abs` — registered in clojure.core under their full name but split
by the lowerer — resolve at Tier 1 exactly as they do tree-walked.
(`rt_load_global` in cljrs-compiler does the same for compiled code.)

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `no-gc` | off | Forwards to `cljrs-gc/no-gc` and `cljrs-value/no-gc`; switches `env::gc_roots`, `interp::special`, and `interp::apply` to the region/`StaticArena` allocation protocol |
| `deps` | **on** | Enables `cljrs-project/vcs` and installs the `cljrs-project`-backed [`VcsProvider`](#vcs-submodule) in `GlobalEnv::new`, so versioned vars (`ns/name@commit`) resolve out of git history and `:verify-commit-signatures` can check signatures |
| `regex-full` | **on** | Forwards `cljrs-value/regex-full`: `Value::Pattern` uses the `regex` crate |
| `small-regex` | off | Forwards `cljrs-value/small-regex`: `Value::Pattern` uses `regex-lite` instead, dropping ~277 KB of text at the cost of Unicode character classes. `regex-full` wins if both are on, so this needs `--no-default-features` — and `deps` off as well, since `cljrs-project/vcs` pulls `regex` in through `pgp`. See [cljrs-value's README](../cljrs-value/README.md#features) |

### Turning `deps` off

`--no-default-features` drops the whole gitoxide/rPGP/`ssh-key` tree — roughly
290 transitive crates, or three quarters of a minimal embedding's build — for
anything that links the interpreter but never resolves a git-hosted dependency:
an editor plugin, a sandboxed evaluator, `cljrs-tx`, an embedded host. It is
also the first prerequisite for any `no_std`/embedded profile, since none of
those crates build for a bare-metal target.

What changes when it is off:

* `GlobalEnv::vcs()` returns `None`, so every source file is treated as living
  outside a git repository. Versioned resolution then works only against
  sources embedded at AOT-compile time; a live fetch reports the existing
  "not in a git repository" error.
* `check_commit_signature` **fails** rather than passing when
  `:verify-commit-signatures` is on — a build that cannot verify must not
  silently accept.
* `load_trusted_signers` returns 0.

The `GlobalEnv` field holding the provider is present in either configuration,
so the struct's layout does not vary with the feature and an embedder cannot be
caught out by Cargo feature unification. An embedder that wants git without
`cljrs-project` can implement `VcsProvider` itself and install it with
`set_vcs_provider`; passing `None` there also lets a sandboxed host *remove*
the default provider at runtime.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` | `Span` |
| `cljrs-gc` | `GcPtr<T>`, alloc frames, safepoints |
| `cljrs-value` | `Value`, `CljxFn`, persistent collections, `shared`, `regex::Pattern` (the regex engine is selected there, not here) |
| `cljrs-reader` | `Form` AST and `Parser` |
| `cljrs-ir` | IR types (`IrFunction`, `Block`, `Inst`, `IrBundle`) and lowering |
| `tracing` / `tracing-subscriber` (non-WASM) | the `gc`/`env`/`ir`/`jit` diagnostic targets, and the `logging` module that filters and installs them |
| `cljrs-project` | `config` — project configuration consulted by the namespace loader (always; no external dependencies of its own). `vcs` (non-WASM, `deps` feature) — git history access for versioned namespace resolution. Never `vcs-net`: the interpreter reads local repositories and never fetches, so no HTTP/TLS stack is linked. |
| `num-bigint`, `num-rational`, `bigdecimal`, `num-traits` | numeric tower |
| `rand`, `rpds`, `uuid` | builtin implementations |
| `log`, `thiserror` | diagnostics and error derivation |
