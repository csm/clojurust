# The runtime builder

`Runtime::builder()` is the **only** way to construct a runtime. There is no
`standard_env`, no `minimal_env`, no `_with_paths` variant: execution mode,
source paths, GC configuration, embedded sources, and tier enablement are all
inputs to one builder, and extensions install themselves into the finished
runtime afterwards.

```rust
use cljrs_runtime::{ExecutionMode, Runtime};

let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .source_paths(vec!["src".into(), "resources".into()])
    .gc_config(config)
    .build()?;
```

Every option has a default, so `Runtime::builder().build()?` is a valid runtime:
tiered execution, no source paths, GC limits taken from the environment,
namespace roots registered.

## Options

| Method | Default | Effect |
|---|---|---|
| `execution_mode(ExecutionMode)` | `Tiered` | Which call path the runtime uses, and which tier it is promoted to. See [Execution modes](execution-modes.md). |
| `source_paths(Vec<PathBuf>)` | empty | Directories searched when `require` resolves a namespace to a file on disk. |
| `gc_config(Arc<GcConfig>)` | none | Explicit soft/hard heap limits. Applied *after* the environment defaults, so it wins. |
| `gc_config_from_env(bool)` | `true` | Whether to apply `CLJRS_GC_SOFT_LIMIT_MB` / `CLJRS_GC_HARD_LIMIT_MB` (and their system-derived defaults) to the heap. |
| `register_gc_roots(bool)` | `true` | Whether this runtime's namespace table is registered as a GC root set. |
| `builtin_source(ns, &'static str)` | none | Embed a namespace's Clojure source in the binary so `require` resolves it without a file. Repeatable. |
| `eager_clojure_test(bool)` | `false` | Evaluate `clojure.test` during construction instead of on first `require`. |
| `build()` | — | Bootstrap and return `Result<Runtime, BuildError>`. |

### `execution_mode`

The one option worth deciding deliberately. It fixes how function calls
dispatch for the life of the runtime and cannot be changed afterwards —
[Execution modes](execution-modes.md) covers the four choices and when each one
is right.

### `source_paths`

These are the roots `require` searches, in order, when a namespace is not
already loaded and is not registered as an embedded source. A host that never
wants guest code reading from disk can leave them empty — though note that an
empty source path is not by itself a sandbox, since `clojure.core` still has
`slurp` and `spit`. See [Limits & sandboxing](sandboxing.md).

Paths can also be appended after construction through
`globals.source_paths` (an `RwLock<Vec<PathBuf>>`); that is how the CLI merges
`:paths` from `cljrs.edn` on top of its command-line flags.

### `gc_config` and `gc_config_from_env`

`GcConfig` carries two numbers: a **soft limit** that triggers a collection when
exceeded, and a **hard limit** that forces one.

```rust
use std::sync::Arc;
use cljrs_gc::GcConfig;

let runtime = Runtime::builder()
    .gc_config_from_env(false)                       // heap ignores CLJRS_GC_*
    .gc_config(Arc::new(GcConfig::with_limits(
        64 * 1024 * 1024,                            // soft: 64 MB
        128 * 1024 * 1024,                           // hard: 128 MB
    )))
    .build()?;
```

Constructors: `GcConfig::new()` (hard limit derived from system memory, soft at
75% of it), `GcConfig::with_hard_limit(bytes)` (soft at 75% of it), and
`GcConfig::with_limits(soft, hard)`.

The two options compose in a fixed order: environment settings are applied
first, then any explicit `gc_config` overwrites them. Leave
`gc_config_from_env` on if you want operators to be able to tune the heap
without a rebuild; turn it off if your host must be the only thing that decides
its own memory budget.

The heap those limits configure is **per thread**, not per process — each
thread that runs a runtime owns an independent heap and collects it
independently.

### `register_gc_roots`

On by default, and you almost certainly want it: it registers the runtime's
namespace table with the collector, so vars, their values, and everything
reachable from them stay alive. The tracer holds a **weak** handle to the
environment, so dropping the last `Runtime` handle stops it from being a root
rather than pinning it forever through the heap's tracer list — which is what
makes several sequential runtimes in one process safe.

Turn it off only if you are constructing an environment you will root yourself.

### `builtin_source`

Embeds Clojure source in your binary and makes it resolvable by name:

```rust
const RULES: &str = include_str!("../clj/acme/rules.cljc");

let runtime = Runtime::builder()
    .builtin_source("acme.rules", RULES)
    .build()?;
```

Guest code can now `(require '[acme.rules :as rules])` with no file on disk.
Sources registered this way are parsed and evaluated on their **first
`require`**, not at build time, so registering a dozen namespaces costs nothing
until they are used. This is exactly how `cljrs-stdlib` ships `clojure.string`
and friends.

### `eager_clojure_test`

Evaluates the embedded `clojure.test` during construction. Only useful when you
are *not* installing an extension that already registers it lazily
(`cljrs-stdlib` does), so most hosts leave it alone.

## What `build()` does, in order

1. Creates a `GlobalEnv` carrying the chosen execution mode.
2. Registers the native `clojure.core` functions.
3. Creates the `user` namespace and refers `clojure.core` into it.
4. Parses and evaluates the Clojure bootstrap source (the higher-order
   functions that are defined in Clojure rather than Rust), then re-refers
   `clojure.core` so the new definitions are visible, and marks it loaded.
5. Registers any `builtin_source` namespaces, and evaluates `clojure.test` if
   `eager_clojure_test` was set.
6. Applies source paths, then GC configuration (environment first, explicit
   config second), then registers namespace GC roots.
7. Resynchronises `*ns*` to `user`.
8. Raises the tier state to what the execution mode targets.

Step 8 is last for a reason: the bootstrap itself always tree-walks, because
nothing can be lowered to IR before `clojure.core` exists. Functions defined
before that point are marked as bootstrap and stay excluded from background
lowering.

### `BuildError`

```rust
pub enum BuildError {
    EmbeddedSource { origin: String, message: String },
}
```

The only failure mode, and it means the *binary's own* embedded text failed to
parse — a broken bootstrap or a malformed `builtin_source`, not anything a user
did. An individual bootstrap form that fails to *evaluate* is reported on stderr
and skipped rather than failing the build.

## The `Runtime` handle

```rust
impl Runtime {
    pub fn builder() -> RuntimeBuilder;
    pub fn from_globals(globals: Arc<GlobalEnv>) -> Runtime;
    pub fn globals(&self) -> &Arc<GlobalEnv>;
    pub fn into_globals(self) -> Arc<GlobalEnv>;
    pub fn env(&self, ns: &str) -> Env;
    pub fn execution_mode(&self) -> ExecutionMode;
    pub fn tier_state(&self) -> TierState;
}
```

`Runtime` is a cheap, cloneable handle: all state lives in the shared
`GlobalEnv`, so a clone **names the same runtime** rather than creating a new
one. Clone it freely to hand to extension `install` functions.

- `env(ns)` gives you a fresh evaluation context in namespace `ns` — this is
  what you evaluate against. Creating one is cheap; make one per request, per
  REPL session, or per task as suits your host.
- `globals()` / `into_globals()` reach the `Arc<GlobalEnv>` underneath, which is
  what extension `init` functions and the nREPL/LSP servers take.
- `from_globals(...)` goes the other way, for code that is handed an
  `Arc<GlobalEnv>` (a native package loader, an AOT harness) and needs a
  `Runtime` to pass to an `install`.

## Environment variables that affect construction

| Variable | Read by | Effect |
|---|---|---|
| `CLJRS_GC_SOFT_LIMIT_MB` | `build()`, when `gc_config_from_env` is on | Per-isolate soft heap limit in MB (default: a third of system memory). Also a deprecated alias for `CLJRS_MEMORY_SOFT_LIMIT_MB`; the governor warns when it reads it. |
| `CLJRS_GC_HARD_LIMIT_MB` | same | Per-isolate hard heap limit in MB (defaults to, and is never below, the soft limit; not enforced). Also a deprecated alias for `CLJRS_MEMORY_HARD_LIMIT_MB`. |
| `CLJRS_MEMORY_SOFT_LIMIT_MB` | first allocation in the process | Process-wide managed-memory soft limit (default: 75% of the hard limit). |
| `CLJRS_MEMORY_HARD_LIMIT_MB` | same | Process-wide managed-memory hard limit (default: the cgroup limit, else half of physical memory). Observed, not yet enforced. |
| `CLJRS_MEMORY_CRITICAL_RESERVE_MB` | same | Reserve for runtime control operations (default: max(4 MiB, 1% of the hard limit), at most 256 MiB). |
| `CLJRS_MEMORY_CREDIT_KB` | same | Allocation credit chunk an isolate takes from the governor at a time (default: 64). |
| `CLJRS_ISOLATE_QUEUE_LIMIT_MB` | same | Default byte limit for one isolate channel (default: 64, at most the hard limit). `(isolate-chan limit-bytes)` overrides it per channel. |
| `CLJRS_NO_IR` | `build()` | Pins the runtime at `TierState::TreeWalk` regardless of the execution mode. |
| `CLJRS_GC_STATS` | `cljrs_gc::dump_stats_from_env()` | Where to write a `GC_STATS` snapshot — unset does nothing, empty or `-` means stdout, anything else is a file path. Nothing reads it on its own; call `dump_stats_from_env()` at exit if you want the behaviour the CLI's `--gc-stats` flag gives. |

Runtime-tuning variables that are read later (IR/JIT thresholds, eager
lowering) are listed under [Execution modes](execution-modes.md#tuning).

## Several runtimes in one process

Two situations, with different answers:

- **Sequentially on one thread** — build, use, drop, build again. Runtime
  identity is a counter-allocated `GlobalEnv::id`, not a memory address, so a
  fresh runtime can never inherit a dropped one's IR cache. Combined with the
  weak root tracer, dropping a runtime really does release it.
- **Concurrently** — one runtime per thread, each built on the thread that uses
  it. Since `Runtime` is `!Send` there is no other option, and the heaps stay
  independent. Move data between them with isolate channels or `shared-atom`;
  see [Worker isolation](../async-io/isolation.md).
