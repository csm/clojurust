# cljrs-env

Environments for running programs in.

## versioned module (non-WASM)

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
`PinnedNativeLoader` callback (provided by `cljrs-dylib`); the resolver
consults it before the HEAD fallback, and a successful load redirects the
lookup into the freshly registered `"<ns>@<commit>"` namespace.

Plain `require` of a native dep: `GlobalEnv::set_native_require_loader`
installs a `NativeRequireLoader` callback (also provided by `cljrs-dylib`).
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

## gc_roots module

The `gc_roots` module manages GC root registration for the interpreter's Rust call stack. Public API includes:

- `push_env_root(env: &Env) -> EnvRootGuard` — registers an `Env` pointer as a GC root; guard removes on drop
- `root_value(val: &Value) -> ValueRootGuard` — registers a single `Value` pointer as a GC root
- `root_values(vals: &[Value]) -> ValueRootGuard` — registers a slice of `Value` pointers as GC roots
- `root_option_values(vals: &[Option<Value>]) -> OptionValueRootGuard` — registers an `Option<Value>` slice (e.g. IR register file)
- `gc_safepoint(env: &Env)` — interpreter-level safepoint: parks if collection in progress, or initiates collection on memory pressure
- `force_collect(env: &Env)` — immediately initiates a GC collection bypassing memory-pressure threshold
- `async_gc_collect()` — services a pending GC request from a Tokio `LocalSet` task at a cooperative yield point; safe to call when no other tasks are polling, so thread-local root stacks are stable and fully describe all suspended-task `GcPtr`s
- `set_stw_reclaim_hook(f)` — registers a stop-the-world reclaim hook; multiple hooks may be registered and each runs (in registration order) inside the STW guard at the tail of every collection (`force_collect`, `gc_safepoint`, `async_gc_collect`), when all mutator threads are parked.  Registrants: `cljrs-jit` frees superseded native code (Phase 10.2); `cljrs-eval`'s lowering worker sweeps idle Tier-1 IR (Phase 10.7)

Root tracing covers all namespaces (including immutable `ns@commit`
namespaces) **and** the values in `GlobalEnv::version_cache`, so versioned
values that exist only in the cache (native HEAD fallbacks) survive
collection.

## apply module

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

## error module

`EvalError` / `EvalResult` are the evaluator's error types. Helpers:

- `EvalError::to_error_value(self) -> Value` — convert an error into a Clojure
  error *value*; `Thrown` is returned unchanged, anything else is wrapped in a
  fresh `ExceptionInfo`
- `value_error_to_eval_error(err: ValueError) -> EvalError` — surface a builtin's
  `ValueError` as a *catchable* `EvalError::Thrown(Value::Error(..))`, preserving
  the original variant and its plain message (no `runtime error:` prefix) so
  `(catch :default e ..)` / `ex-message` / `ex-data` behave the same as for a
  user `throw` / `ex-info`. A `ValueError::Thrown` re-surfaces the exact value.

## callback module

Thread-local eval context for Rust→Clojure callbacks (`invoke`, `with_eval_context`). The context is pushed automatically around native builtin calls and by the Tier-1 IR executor; rt_abi bridges (`rt_call`, `rt_load_global`, the HOF bridges) dispatch through it. Public API includes:

- `push_eval_context(env: &Env)` / `pop_eval_context()` — bracket a native call with the current env's globals + namespace
- `capture_eval_context() -> Option<(Arc<GlobalEnv>, Arc<str>)>` — snapshot the innermost context (e.g. to hand to another thread)
- `install_eval_context(globals, ns)` — push a previously captured context (spawned threads)
- `install_eval_context_guard(globals, ns) -> EvalContextGuard` — like `install_eval_context`, but pops on drop (including unwind); used by the JIT-native dispatch seam
- `current_is_async() -> bool` — whether the innermost context is inside an `^:async` body
- `invoke(f: &Value, args: Vec<Value>) -> ValueResult<Value>` — call a Clojure-callable value through the innermost context. Honors `^:async` dispatch (via `apply::dispatch_if_async`) so a native/compiled caller of an `^:async` fn gets a `Value::Future`, not a synchronously-run body
- `with_eval_context(f)` — run a closure with a temporary `Env` built from the innermost context

## async_hook module

The optional async-runtime seam (`AsyncRuntime` trait, installed by `cljrs-async`). Also hosts the async-JIT compile hook: `set_async_compile_hook` / `async_compile_hook` (`fn(&Value, usize, &mut Env)`), installed by `cljrs-jit::init` and called by the async dispatcher to lower + compile + register a native poll function for a called `^:async` arity (a no-op when the JIT is absent).
