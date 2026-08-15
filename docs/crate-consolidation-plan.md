# Crate Consolidation Plan

## Purpose

This plan reduces workspace package count and removes obsolete architecture.
It keeps package boundaries that isolate a real artifact, platform, or optional dependency.

The workspace contains 34 packages and approximately 111,000 lines of Rust and Clojure source.
The proposed target contains approximately 23 packages.

This plan does not remove the tree-walking interpreter, JIT, AOT compiler, no-GC mode, or supported platform targets.

## Progress

| Stage | Status | Notes |
|---|---|---|
| 0. Record the baseline | Complete | [`consolidation-baseline.md`](consolidation-baseline.md) |
| 1. Remove obsolete debris | Complete | `cljrs-ir-prebuild` folded into `cljrs::commands::ir`; 34 packages → 33 |
| 2. Create the merged runtime | Complete | `cljrs-env`/`-builtins`/`-interp`/`-eval` merged into `cljrs-runtime`; the four remain as re-export shims until Stage 6 |
| 3. Simplify runtime state | Complete | One builder, one dispatch path; per-instance IR cache |
| 4. Merge JIT and compiler packages | Complete | `cljrs-jit` folded into `cljrs-compiler::jit`; 33 packages → 32 |
| 5. Consolidate project and CLI tools | Not started | |
| 6. Remove compatibility packages | Not started | |

Package count: 34 at baseline, 32 now, approximately 23 at target. Stage 2 does
not change the count — it moves four packages' code into `cljrs-runtime` and
leaves them as re-export shims; Stage 6 deletes the shims and takes 32 → 28.

### Corrections found while measuring the baseline

Three Evidence items were already resolved before this work started, so they need
no Stage 1 change:

- `cljrs-stdlib/src/core_async.rs` does not exist. The only `core_async` source is
  `crates/cljrs-async/src/core_async.cljrs`, which is live Clojure source, not
  commented-out Rust.
- `cljrs-stdlib/Cargo.toml` no longer lists `tokio` or `lazy_static`. Only
  `Cargo.lock` still carried the stale entries.
- No Clojure compiler namespaces remain, so `cljrs-ir-prebuild` no longer loads
  any — but its own docs still claim it does, which Stage 1 corrects.

The baseline also found one thing the plan did not anticipate: **`no-gc` does not
build today**, for the CLI or for `cljrs-env`, `cljrs-builtins`, `cljrs-stdlib`,
and `cljrs-async` on their own. See
[`consolidation-baseline.md` §4.2](consolidation-baseline.md). Later stages state
"Default and `no-gc` builds pass" as a gate; until those pre-existing defects are
fixed, `no-gc` can only be held to "no worse than baseline".

## Background

The current core split came from an earlier compiler bootstrap design.
That design ran compiler code written in Clojure before it lowered functions to IR.

The compiler front end now uses Rust code in `cljrs-ir` and `cljrs-eval`.
No Clojure compiler namespaces remain in the repository.
The old split now adds callbacks, duplicate constructors, and broad dependency chains.

The main overlap is between these packages:

- `cljrs-env`
- `cljrs-builtins`
- `cljrs-interp`
- `cljrs-eval`
- `cljrs-runtime`
- `cljrs-stdlib`
- `cljrs-ir`
- `cljrs-compiler`
- `cljrs-jit`

## Evidence

### Obsolete packages and modules

`cljrs-runtime` is an unused nine-line stub.
The CLI depends on it only to propagate the `no-gc` feature.

`cljrs-compiler::ir` only re-exports `cljrs-ir`.
This module does not define a compiler boundary.

`cljrs-stdlib/build.rs` does not build IR.
It only emits a Cargo rerun directive.
The root README still describes an obsolete build-time compiler flow.

`cljrs-stdlib/src/core_async.rs` contains only commented-out code.
It retains unused `tokio` and `lazy_static` dependencies.

### Duplicate runtime entry points

`cljrs-interp`, `cljrs-eval`, and `cljrs-stdlib` each construct a standard environment.
These constructors represent modes of one runtime, not separate products.

`cljrs-eval` also re-exports environment and interpreter APIs.
This facade hides package ownership and increases direct dependencies in downstream packages.

### Callback seams

`GlobalEnv` stores function pointers for form evaluation, function calls, and function-definition hooks.
These pointers break dependency cycles between runtime packages.

The pointers are not extension APIs for users.
The runtime can use direct module calls after the packages merge.

### Compiler dependency fan-out

`cljrs-compiler` directly depends on 16 internal packages.
It also initializes async, I/O, networking, charset, and Base64 support during AOT compilation.

The compiler backend must not select product extensions.
The CLI or an embedding host must supply the required extension set.

### IR bundle tooling

`cljrs-ir-prebuild` duplicates the `cljrs ir build` and `cljrs ir dump` commands.
The standard runtime does not load the generated bundles.

The package documentation still says that the tool loads Clojure compiler namespaces.
That statement is no longer correct.

## Boundary rules

When at least one rule applies, keep a package:

1. Cargo requires a separate artifact, such as a procedural macro or `cdylib`.
2. The package isolates a large optional dependency or platform.
3. Multiple independent consumers use a stable low-level API.
4. The boundary prevents a dependency cycle without callback indirection.
5. The package has an independent release or embedding use case.

When none of these rules apply, use a Rust module.

## Target workspace

### Core packages

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs-types` | Keep | Source spans and shared error types. |
| `cljrs-reader` | Keep | Lexer, parser, and `Form` AST. |
| `cljrs-gc` | Keep | Garbage collection, regions, and allocation contexts. |
| `cljrs-value` | Keep | Runtime values, collections, callable values, and resources. |
| `cljrs-runtime` | Replace stub and expand | Environment, builtins, tree walker, tiered evaluation, and runtime construction. |
| `cljrs-stdlib` | Keep | Embedded standard-library sources and native namespace helpers. |
| `cljrs-ir` | Keep | IR model, Rust lowering, optimization, OSR transforms, and serialization. |
| `cljrs-compiler` | Expand | Shared code generation, JIT, native AOT, and optional WASM AOT. *(Stage 4: done.)* |

### Interop and extension packages

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs-interop` | Keep | Rust value conversion, native registration, and export support. |
| `cljrs-export-macro` | Keep | Procedural macro for exported Rust functions. |
| `cljrs-async` | Keep | Async runtime, channels, isolates, and worker pools. |
| `cljrs-io` | Keep | Async file I/O. |
| `cljrs-net` | Keep | Network transports and protocols. |
| `cljrs-charset` | Keep | Charset codecs and stream adapters. |

### Tools and artifacts

| Package | Decision | Responsibility |
|---|---|---|
| `cljrs` | Keep | Main CLI and internal command modules. |
| `cljrs-lsp` | Keep | Standalone LSP server and reusable backend. |
| `cljrs-nrepl` | Keep | nREPL server. |
| `cljrs-wasm` | Keep | Browser artifact and WASM runtime facade. |
| `cljrs-tx` | Keep | Isolated no-GC transaction execution. |
| `cljrs-project` | Create | Project configuration, dependency resolution, and VCS operations. |

### Consolidation map

| Current package | Target location |
|---|---|
| `cljrs-env` | `cljrs-runtime::env` *(done, stage 2)* |
| `cljrs-builtins` | `cljrs-runtime::builtins` *(done, stage 2)* |
| `cljrs-interp` | `cljrs-runtime::interp` *(done, stage 2)* |
| `cljrs-eval` | `cljrs-runtime::tiered` *(done, stage 2)* |
| `cljrs-jit` | `cljrs-compiler::jit` *(done, stage 4)* |
| `cljrs-ir-viz` | Internal `cljrs::commands::ir` module |
| `cljrs-ir-prebuild` | Internal CLI module or removal |
| `cljrs-deps` | `cljrs-project::config` |
| `cljrs-vcs` | `cljrs-project::vcs` |
| `cljrs-dylib` | Internal CLI native-package module |
| `cljrs-dom` | `cljrs-wasm::dom` |
| `cljrs-logging` | Standard `tracing` targets and filters |

Keep `cljrs-base64` and `cljrs-blake3` as native-package examples.
If no core package requires them, exclude them from default workspace builds.

## Runtime API

The merged runtime owns environment construction and execution-mode selection.
Downstream packages must not assemble runtime internals directly.

The target API has one construction path:

```rust
let runtime = Runtime::builder()
    .execution_mode(ExecutionMode::Tiered)
    .source_paths(paths)
    .gc_config(config)
    .build()?;

cljrs_stdlib::install(&runtime)?;
```

`ExecutionMode` replaces the current constructor variants:

- `TreeWalk`
- `Tiered`
- `TieredNoJit`
- `NoGcTransaction`

The builder also owns source paths, GC configuration, extension registration, and embedded namespace sources.

## Compiler API

The merged compiler contains common code generation and JIT state.
The JIT and AOT paths use the same type inference and runtime ABI definitions.

The compiler accepts a prepared compile session.
The session contains the runtime, source paths, target, and extension descriptors.

The CLI selects extensions from enabled features and project configuration.
The compiler does not contain direct calls to each extension package.

Generated AOT harnesses use generic initialization descriptors.
Optional extensions register their runtime ABI symbols through one registry.

## Work plan

Each stage must remain independently reviewable.
Each stage must pass its validation gate before the next stage starts.

### Stage 0: Record the baseline

1. Record the workspace package count and dependency graph.
2. Record build time, cold-start time, and CLI binary size.
3. Record the full test matrix for default and `no-gc` builds.
4. Record successful JIT, native AOT, transaction, and WASM examples.

Store the measurements in `docs/consolidation-baseline.md`.

Validation gate:

- `cargo build`
- `cargo test`
- `cargo clippy -- -D warnings`
- `cargo fmt --check`
- Existing CLI smoke tests

### Stage 1: Remove obsolete debris

1. Remove the unused `cljrs-runtime` dependency from the CLI.
2. Remove the commented `cljrs-stdlib/src/core_async.rs` module.
3. Remove the unused `tokio` and `lazy_static` dependencies from `cljrs-stdlib`.
4. Remove the empty `cljrs-stdlib/build.rs` script.
5. Replace `cljrs-compiler::ir` imports with direct `cljrs_ir` imports.
6. Remove the re-export module.
7. Correct stale README and crate documentation.

Decide the IR bundle feature during this stage.
If baseline measurements show a required cold-start benefit, keep the feature.

If the feature remains, move its implementation into the CLI.
If the feature does not remain, remove its package, commands, and unused runtime API.

Validation gate:

- The default CLI has unchanged behavior.
- The workspace contains no dead build script or commented implementation module.
- Documentation describes only the Rust lowering path.

#### Stage 1 outcome

Items 2 and 3 needed no source change: `cljrs-stdlib/src/core_async.rs` does not
exist and `cljrs-stdlib/Cargo.toml` no longer lists `tokio` or `lazy_static`.
Only `Cargo.lock` still carried those entries; the Stage 0 commit refreshed it.

What changed:

1. Dropped `cljrs-runtime` from the CLI's dependencies and from its `no-gc`
   feature list. Nothing in the workspace depends on the stub now.
4. Deleted `crates/cljrs-stdlib/build.rs`.
5. and 6. Deleted `crates/cljrs-compiler/src/ir.rs` and rewrote all 27
   `crate::ir::` paths inside `cljrs-compiler` to `cljrs_ir::`. No package
   outside `cljrs-compiler` referenced `cljrs_compiler::ir`.
7. Corrected the root README's "Prebuilt IR pipeline" section (it described a
   build-time bootstrap through Clojure compiler namespaces that does not
   exist), its dependency graph, its crate table and repository layout, the
   `cljrs-stdlib` README's `build.rs` entry and incomplete file/API lists, the
   `cljrs-compiler` README's `ir.rs` entry, the `cljrs` README's dependency
   table, the `cljrs-eval` README's `load_prebuilt_ir` note, the
   `cljrs-value` README's forward reference to `cljrs-runtime`, and the
   `cljrs-runtime` README (which described a `clojure.core` implementation that
   actually lives in `cljrs-builtins` and `cljrs-interp`).

**IR bundle decision: keep the commands, delete the package.** Baseline §5 found
no cold-start benefit to preserve - no runtime path calls `load_prebuilt_ir`,
and cold start is already ~50 ms with no bundle loaded. But `ir build` and
`ir dump` are useful lowerer diagnostics and the public replay API matters for
targets with no background lowering worker, so the feature stays. `run_prebuild`
moved into `cljrs::commands::ir` and the `cljrs-ir-prebuild` package - library,
duplicate standalone binary, and all - is gone. `cljrs_eval::load_prebuilt_ir`
is retained, since the feature it serves remains.

This also creates `crates/cljrs/src/commands/`, the module tree Stage 5 splits
`main.rs` into. `IrCommands`, its dispatch, and `ir viz` moved there with the
prebuild code, so Stage 5 inherits a finished `commands::ir`.

Gate results:

| Check | Result |
|---|---|
| `cargo fmt --check` | pass |
| `cargo clippy --workspace -- -D warnings` | pass |
| `cargo test --workspace` | 1077 passed, 0 failed, 24 ignored - identical to baseline (144 suites vs 147: the three `cljrs-ir-prebuild` targets are gone, no test was lost) |
| Clojure test suite (AOT) | 242 suites, 629 tests, 11,005 assertions, 0 failures |
| `cljrs --help`, `cljrs ir --help`, `ir dump/viz --help` | byte-identical to the baseline binary |
| `cljrs ir build --help` | one intended text change: it no longer claims bundles are "loaded back at startup" |
| `cljrs ir build --ns clojure.core` | 151 functions lowered, 2 unsupported - identical to baseline. Bundle bytes differ run-to-run in *both* binaries (the bundle is a `HashMap`), and `ir dump` of one bundle is identical modulo ordering |
| `graph`, `life`, `core_async` samples | compile and run |
| `no-gc` | unchanged from baseline: same four packages fail with the same three defects |

Package count: 34 → 33.

### Stage 2: Create the merged runtime

1. Replace the `cljrs-runtime` stub with the new runtime package.
2. Move `cljrs-env` source files into `cljrs-runtime::env`.
3. Move `cljrs-builtins` source files into `cljrs-runtime::builtins`.
4. Move `cljrs-interp` source files into `cljrs-runtime::interp`.
5. Move `cljrs-eval` source files into `cljrs-runtime::tiered`.
6. Add temporary re-export packages for downstream migration.
7. Move tests with their source modules.

Do not change execution behavior during file movement.
Use mechanical import changes and small commits.

Validation gate:

- Tree-walk-only evaluation passes its current tests.
- Tiered evaluation passes its current tests.
- Default and `no-gc` builds pass.
- Existing public paths work through temporary re-exports.

#### Stage 2 outcome

`cljrs-runtime` is no longer a stub. The four packages moved in whole, one
module each, with `git mv` so history follows the files:

| Former package | Now |
|---|---|
| `cljrs-env` | `cljrs_runtime::env` |
| `cljrs-builtins` | `cljrs_runtime::builtins` |
| `cljrs-interp` | `cljrs_runtime::interp` |
| `cljrs-eval` | `cljrs_runtime::tiered` |

Each former `src/lib.rs` became the module's `mod.rs`; its crate-level
`#![allow(...)]` attributes now apply to the module, and the union sits at the
new crate root. The 29 integration tests in `cljrs-interp/tests` and
`cljrs-eval/tests` moved to `cljrs-runtime/tests`.

The only edits to moved code were path rewrites, applied mechanically:

- inside `env`: `crate::` → `crate::env::`
- inside `builtins`: `crate::` → `crate::builtins::`, `cljrs_env::` → `crate::env::`
- inside `interp`: `crate::` → `crate::interp::`, plus `cljrs_env::`/`cljrs_builtins::`
- inside `tiered`: `crate::` → `crate::tiered::`, plus `cljrs_env::`/`cljrs_builtins::`/`cljrs_interp::`
- in the moved tests: `cljrs_env::`/`cljrs_builtins::`/`cljrs_interp::`/`cljrs_eval::`
  → `cljrs_runtime::env::`/`::builtins::`/`::interp::`/`::tiered::`

Re-applying those same rewrites to the pre-merge files and diffing against the
merged tree leaves only `cargo fmt` re-wrapping (longer paths), rustfmt's
`use`-statement reordering, and four intentional edits: three stale comments
that named `cljrs_eval` paths, and two `#[allow(clippy::module_inception)]`
attributes — `env::env` and `builtins::builtins` keep their names so
`cljrs_runtime::env::env::GlobalEnv` matches the pre-merge
`cljrs_env::env::GlobalEnv`. No behavior changed.

`cljrs-env`, `cljrs-builtins`, `cljrs-interp`, and `cljrs-eval` are now
one-line shims (`pub use cljrs_runtime::<module>::*;`) that depend only on
`cljrs-runtime` and forward `no-gc` to it. **No file outside the four moved
packages and `cljrs-runtime` changed** — every downstream `Cargo.toml`, import,
and CLI source is untouched, so CLI help and command behavior are unchanged by
construction.

Deferred by design, per the plan's own staging: the `Runtime` / `RuntimeBuilder`
/ `ExecutionMode` API and the removal of the `GlobalEnv` callback seams are
Stage 3; `interp::standard_env{,_minimal,_with_paths}` and
`tiered::standard_env*` still coexist as before. The `env::add` template
function and its test moved unchanged rather than being deleted mid-move.
Prose in the twelve other crate READMEs that names `cljrs-eval`/`cljrs-interp`
as owning code is left for Stage 6, which is where the plan puts "update every
affected crate README"; their dependency tables are still literally correct
because the shims still exist.

Gate results:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 1148 passed, 0 failed, 24 ignored, 150 targets |
| Tree-walk-only tests (moved from `cljrs-interp`) | pass |
| Tiered tests (moved from `cljrs-eval`) | pass |
| Existing public paths through the shims | pass — the whole workspace, examples included, compiles with no import changes |
| `cljrs eval` / `cljrs run samples/graph.cljrs` | pass |
| `cljrs compile -o life-sample samples/life.cljrs` + run | pass (final population 355) |

The `cargo test` totals are measured against this branch's merge base, not
against the Stage 0 baseline's 1077 — `main` gained tests between the two. No
test was lost or skipped: the diff above shows the moved test files are
byte-identical modulo the path rewrites and formatting.

One incidental fix: `tests/defonce_metadata.rs` had an
`assert_eq!(x.contains(..), true)` that `clippy::bool_assert_comparison`
rejects. It never fired before because the documented gate runs clippy without
`--all-targets`, so test targets went unlinted. Rewritten as `assert!`.

`no-gc` — better than baseline, still not green:

| Package | Baseline | Now |
|---|---|---|
| `cljrs-env` | fail | **pass** |
| `cljrs-builtins` | fail | **pass** |
| `cljrs-runtime` | (stub) | pass |
| `cljrs-interp`, `cljrs-eval`, `cljrs-gc`, `cljrs-value`, `cljrs-tx` | pass | pass |
| `cljrs-stdlib` | fail | fail |
| `cljrs-async` | fail | fail |
| `cljrs` (both feature sets) | fail | fail |

The merge fixed baseline defect 3 outright: one `no-gc` feature on
`cljrs-runtime` forwards both `cljrs-gc/no-gc` and `cljrs-value/no-gc`, so the
env and builtins code no longer depends on a downstream package to unify its
features.

Baseline defect 2 was misdiagnosed. `cljrs-stdlib`'s `no-gc` feature *does*
reach the environment layer now, and it still fails — because
`cljrs-stdlib/src/lib.rs` calls `cljrs_gc::HEAP.set_config_from_env()` at two
sites and that method only exists in the GC build. That is `cljrs-stdlib`'s own
defect, not a feature-wiring bug, and it is left for Stage 3, which rewrites
this file to `install(&Runtime)`. Defect 1 (`cljrs-async` has no `no-gc`
feature) is untouched and remains Stage 4's.

### Stage 3: Simplify runtime state

1. Add `Runtime`, `RuntimeBuilder`, and `ExecutionMode`.
2. Move standard-environment construction into the builder.
3. Change `cljrs-stdlib` to expose `install(&Runtime)`.
4. Remove duplicate standard-environment constructors.
5. Remove evaluator function pointers from `GlobalEnv`.
6. Remove the function-definition hook from `GlobalEnv`.
7. Replace `compiler_ready` with explicit tier state.
8. Move IR caches and tier counters into the runtime instance.
9. Remove cache keys based on the address of `GlobalEnv`.

Keep explicit hooks only for optional systems.
Examples include the async runtime and native-package loaders.

Validation gate:

- One builder creates every supported runtime mode.
- One function-call path selects tree walk, IR, or JIT execution.
- Two runtime instances do not share IR caches or tier counters.
- The runtime has no callback that exists only to break an old package cycle.

#### Stage 3 outcome

Runtime construction and execution-mode selection now belong to
`cljrs-runtime`. What changed, item by item:

**1-4. One construction path.** Nine "standard environment" constructors across
three layers are replaced by `Runtime::builder()`:

| Removed | Replacement |
|---|---|
| `cljrs_interp::standard_env{,_minimal,_with_paths}` | `.execution_mode(TreeWalk)` |
| `cljrs_eval::standard_env{,_minimal,_with_paths}` | `.execution_mode(Tiered)` |
| `cljrs_eval::standard_env_minimal_no_ir` | `.execution_mode(TreeWalk)` |
| `cljrs_eval::mark_compiler_ready` | `build()` raises the tier state |
| `cljrs_stdlib::standard_env{,_no_ir}` | builder + `cljrs_stdlib::install` |
| `cljrs_stdlib::standard_env_with_paths{,_and_config}` | `.source_paths()` / `.gc_config()` |

The builder owns the bootstrap, source paths, GC configuration and root
registration, embedded namespace sources, and tier enablement. `cljrs-stdlib`
is now an extension: `install(&Runtime)` adds namespaces to a runtime the
caller already built, and it no longer decides execution modes or GC limits.

That removes its direct `cljrs_gc::HEAP.set_config_from_env()` calls — the
`no-gc` defect the Stage 2 outcome left for this stage. `cljrs-gc` grows a
no-op `set_config_from_env()` in the `no-gc` build so the API is uniform in
both builds.

**5-6. Callback seams removed.** `GlobalEnv::eval_fn`, `call_cljrs_fn`, and
`on_fn_defined` were function pointers that let `cljrs-interp` reach
`cljrs-eval` without a dependency cycle. With both in one package since Stage 2
the mode is data and the dispatch is a direct call: `GlobalEnv::call_cljrs_fn`
matches on `ExecutionMode`, and `eval` calls the tree walker directly — its
pointer only ever held one implementation.

The one call-path override with a reason to exist is `cljrs-tx`'s call-depth
cap, which keeps interpreted recursion from overflowing the host thread's Rust
stack. It survives as `ExecutionMode::NoGcTransaction` plus
`cljrs_runtime::env::depth`, i.e. as a runtime-owned mode rather than an
arbitrary hook a downstream package installs.

**7. Explicit tier state.** `compiler_ready` was a bool that said only
"not tree-walk". `TierState` (`TreeWalk` / `Ir` / `Jit`) starts at `TreeWalk` —
nothing can be lowered before `clojure.core` exists — and the builder raises it
once to the mode's target tier when the bootstrap finishes. This is what gives
`ExecutionMode::TieredNoJit` real meaning: it stops at Tier 1 even with a JIT
backend linked in, where before the only way to not reach native code was to
not link `cljrs-jit`. `CLJRS_NO_IR` pins any runtime at `TreeWalk`.

Five test crates spun on `compiler_ready` waiting for a background
compiler-namespace loader that has not existed since the Rust lowering path
landed; those loops are deleted.

**8. IR cache in the runtime instance.** `IrCache` moved from a process-global
static into `GlobalEnv`. Two runtimes never read or evict each other's entries,
and a runtime's IR is freed when the runtime is — the accumulation
`cljrs_stdlib::standard_env_no_ir`'s doc comment worked around ("hundreds of MB
over 233 namespaces") is structural now, not something callers avoid by
picking a different constructor.

Three callers hold only an arity id and have no route back to the runtime that
minted it: the background lowering worker, the JIT worker's publish guard, and
the process-global var-rebind hook. They resolve through a weak index of live
caches. Arity ids come from one process-wide counter, so at most one live cache
can hold a given id and the lookup is unambiguous.

**Deferred to Stage 4, by the plan's own division:** the JIT tier counters and
native-code tables in `tiered::jit_state` stay process-global. They are reached
from `cljrs-jit` and from JIT-compiled native code through arity ids and
`OnceLock` hooks with no runtime handle anywhere on the path, and Stage 4 item 4
is exactly "replace global JIT hooks with compiler state owned by the runtime".
Moving them here would have pulled that work forward and made both stages
unreviewable. The weak IR-cache index goes away with them.

**9. No address-derived cache keys.** The cross-defn registry was keyed by
`Arc::as_ptr(&env.globals) as usize`. An address is unique only while its
allocation is live, so a dropped runtime could hand its key to the next one
built and let it inherit — and stage-4 inline — the dead runtime's IR. Keys are
now `GlobalEnv::id`, allocated from a counter. The GC root tracer holds a `Weak`
handle for the same reason: a dropped runtime stops being a root instead of
pinning itself alive forever through the heap's tracer list.

Gate results:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 1148 passed, 0 failed, 23 ignored — same pass count as Stage 2 |
| One builder creates every supported runtime mode | pass — `TreeWalk`, `Tiered`, `TieredNoJit`, `NoGcTransaction` all come from `Runtime::builder()`; no other constructor remains |
| One function-call path selects tree walk, IR, or JIT | pass — `GlobalEnv::call_cljrs_fn` is the single dispatch point |
| Two runtime instances do not share IR caches | pass — covered by `ir_cache::tests::caches_are_per_runtime`. Tier counters are Stage 4's, see above |
| No callback that exists only to break an old package cycle | pass — the three `GlobalEnv` pointers are gone; the loader/async/JIT hooks that remain are for optional systems, which the plan keeps |
| Clojure test suite (interpreter) | 240 namespaces, 308 tests, 5,486 assertions, 0 failures |
| Clojure test suite (AOT) | 240 namespaces compiled and run: 308 tests, 5,486 assertions, 0 failures |
| `cljrs eval`, `run samples/graph.cljrs`, `compile -o life-sample samples/life.cljrs` + run | pass. `samples/life.cljrs` seeds its grid randomly, so its final population differs run to run in any build |
| `cljrs ir build --ns clojure.core` | 151 functions lowered, 2 unsupported — identical to Stage 1 and Stage 2 |

`no-gc` — **now green for the CLI**, the first time in this plan:

| Package | Baseline | After Stage 2 | Now |
|---|---|---|---|
| `cljrs-env`, `cljrs-builtins` | fail | pass | pass |
| `cljrs-runtime`, `cljrs-interp`, `cljrs-eval`, `cljrs-gc`, `cljrs-value`, `cljrs-tx` | pass | pass | pass |
| `cljrs-stdlib` | fail | fail | **pass** |
| `cljrs` (both feature sets) | fail | fail | **pass** |
| `cljrs-async` | fail | fail | fail |

Baseline defect 2 is fixed: `cljrs-stdlib` called `cljrs_gc::HEAP.set_config_from_env()`
at two sites, and that method existed only in the GC build. Moving GC
configuration into the runtime builder removed both calls, and `cljrs-gc` gained
a no-op `set_config_from_env()` in the `no-gc` build so the API is the same
shape in both. With `cljrs-stdlib` building, the CLI builds too, with and
without default features.

Defect 1 — `cljrs-async` has no `no-gc` feature at all — is untouched and
remains Stage 4's.

### Stage 4: Merge JIT and compiler packages

1. Move `cljrs-jit` into `cljrs-compiler::jit`.
2. Share type inference directly between JIT and AOT code generation.
3. Share code-cache and runtime-ABI definitions inside the compiler.
4. Replace global JIT hooks with compiler state owned by the runtime.
5. Add an extension registry for AOT harness generation.
6. Remove hard-coded extension initialization from `aot.rs`.
7. Put the WASM backend behind a `wasm-aot` feature.
8. Remove the `cljrs-jit` compatibility package after downstream migration.

Validation gate:

- JIT promotion and deoptimization tests pass.
- Native AOT end-to-end tests pass.
- A compiler build without network extensions does not compile `cljrs-net`.
- The compiler does not select optional product extensions.

#### Stage 4 outcome

**1, 8. `cljrs-jit` is gone.** Its five source files moved with `git mv` into
`crates/cljrs-compiler/src/jit/` and its three integration tests into
`crates/cljrs-compiler/tests/jit_*.rs`. The only edits to moved code were path
rewrites. It satisfied no boundary rule: the shared codegen was its only
reason to be separate, and the CLI was its only consumer. Package count
33 → 32.

**2, 3. One compiler, shared surfaces.** JIT and AOT now run the same
`typeinfer::infer` (the JIT seeds it from Tier-1 argument profiles, AOT and
wasm from static `^long`/`^double` hints), the same `codegen::Compiler<M>`,
and the same `rt_abi`. The pieces that existed only to cross the old package
boundary stopped being public API — `codegen::new_compiler_from_module`,
`rt_abi::take_pending_exception_value`, `rt_abi::deopt_sentinel_addr` are
`pub(crate)` — and one hook disappeared outright: `rt_abi`'s closure-escape
`OnceLock` is now a direct call to `jit::code_cache::pin_epoch`, with the same
`current_jit_epoch()` guard that made it a no-op under AOT.

**4. The JIT state belongs to the runtime.** This is the item Stage 3
explicitly deferred, along with the weak `IrCache` index it left behind.

`Tiers` (`cljrs_runtime::tiered::tiers`) holds one runtime's `IrCache` plus
`JitState` and is owned by its `GlobalEnv`. `jit_state`'s four process-global
tables — `JIT_TABLE`, `OSR_TABLE`, `SPEC_BANNED`, `BOOTSTRAP_ARITY_WATERMARK` —
became fields of `JitState`, and its free functions became methods reached
through `GlobalEnv::jit()` on paths that already had the environment in hand.
Two runtimes in one process no longer share invocation counters,
argument-type profiles, published native pointers, OSR entries, or
specialization bans.

Six process-global `OnceLock` hooks collapsed into one per-runtime handle.
`JitBackend` (`tiered::backend`) carries enqueue, OSR enqueue, mark-stale,
pending-exception, deopt-sentinel, and async-compile; `cljrs_compiler::jit::install(&runtime)`
attaches it. It stays a trait — `cljrs-compiler` depends on `cljrs-runtime`,
so the call can only go one way — but it is now a stateless seam for an
optional system, which is exactly the kind of hook the plan keeps. The
async-JIT hook in `env::async_hook` is gone: the dispatcher asks the calling
runtime for its backend.

Background workers carry `Weak<Tiers>` in their requests instead of an arity
id resolved through a process-wide index (`GlobalEnv` owns `GcPtr`s and is not
`Send`; `Tiers` is). A compile or lowering publishes into exactly the runtime
that asked, or is discarded when that runtime was dropped meanwhile. Stage 3's
weak `IrCache` index is deleted.

Three things stay process-wide, because they describe the process rather than
a runtime, and the outcome is stated here so a later reader does not mistake
them for leftovers:

- **Thresholds** (`CLJRS_JIT_THRESHOLD`, `--ir-threshold`, …) — configuration,
  set before any runtime is built.
- **Active native frames** (`push_jit_frame` / `live_epochs`) — a *thread* can
  hold frames from several runtimes, and reclamation asks whether any thread
  is executing a module.
- **The code cache and compile worker** — one executable-memory budget, one
  background thread.

`Var::bind`'s rebind notification is also still global: it sees two values and
no runtime at all. It iterates the live tier states; arity ids come from one
process-wide counter, so that is exact for the owning runtime and a no-op in
every other.

**5, 6. The compiler does not choose extensions.** `aot.rs` named the optional
packages directly — five `init(&globals)` calls at two compile-time sites, a
hard-coded registration block in the generated harness, and the packages in
`cljrs-compiler`'s own `[dependencies]`. `extensions::Extension` now describes
one extension generically (the package to add to the harness, a function
pointer to register it at compile time, and the path to that same function for
generated source), and `CompileSession` carries the set with the source paths,
`:rust` crate config, signature policy, and opacity policy. `compile_file` and
`compile_file_to_wasm` take `&CompileSession`.

`cljrs::extensions::default_set()` builds the set from the CLI's own Cargo
features, so `cljrs run` and `cljrs compile` of one program see the same
namespaces — default CLI behavior preserved, which the plan's risk section
asks for. `cljrs-io`, `cljrs-net`, `cljrs-charset`, and `cljrs-base64` left
the compiler's dependencies (dev-dependencies now; the end-to-end tests supply
them the way a host does). `cljrs-async` stays, because `codegen` and `rt_abi`
implement its state-machine poll ABI — the compiler's own contract, not a
product choice — and `cljrs-async` stays in the harness's base crate list for
the same reason. Registering `clojure.core.async` is still the host's call.

**7. `wasm-aot`.** The WebAssembly backend, `compile_file_to_wasm`, and the
`wasm-encoder` dependency sit behind a default-on feature;
`--no-default-features` builds a native-only compiler.

Gate results:

| Check | Result |
|---|---|
| `cargo fmt --all --check` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | 1153 passed, 0 failed, 26 ignored, 148 targets (Stage 3: 1148/23/150 — three targets fewer because `cljrs-jit`'s tests moved into `cljrs-compiler`, and the new tests are the per-runtime isolation cases) |
| JIT promotion | pass — `osr_promotion::single_call_hot_loop_promotes_mid_run`, `background_lowering::tiering_up_mid_run_keeps_results_correct`, `jit_specialization::monomorphic_long_profile_unboxes_hot_loop_arithmetic` |
| Deoptimization | pass — `jit_specialization::type_guard_violation_deopts_to_tier1_with_correct_results` |
| Native AOT end-to-end | pass — `cljrs-compiler`'s `aot_e2e` (harness generation now emits host-supplied extension registration), plus the Clojure suite compiled and run below |
| A compiler build without network extensions does not compile `cljrs-net` | pass — `cargo tree -p cljrs-compiler --edges normal` contains no `cljrs-net`, `cljrs-io`, `cljrs-charset`, or `cljrs-base64` |
| The compiler does not select optional product extensions | pass — no `cljrs_*::init` call remains in `aot.rs`; the harness emits `ExtensionSet::harness_init_code()` |
| Clojure test suite (interpreter) | 240 namespaces, 308 tests, 5,486 assertions, 0 failures |
| Clojure test suite (AOT) | same: 308 tests, 5,486 assertions, 0 failures |
| `cljrs eval`, `run samples/graph.cljrs`, `run samples/core_async.cljrs`, `compile -o life-sample samples/life.cljrs` + run, `compile --target wasm` | pass |
| `cljrs ir build --ns clojure.core` | 151 functions lowered, 2 unsupported — identical to stages 1, 2, and 3 |

`no-gc` — **green for the whole workspace**, the first time in this plan:

| Package | Baseline | After Stage 3 | Now |
|---|---|---|---|
| `cljrs-gc`, `cljrs-value`, `cljrs-runtime`, `cljrs-env`, `cljrs-builtins`, `cljrs-interp`, `cljrs-eval`, `cljrs-tx`, `cljrs-stdlib`, `cljrs-compiler`, `cljrs` | mixed | pass | pass |
| `cljrs-async` | fail | fail | **pass** |

Baseline defect 1 was the last one standing: `cljrs-async` had no `no-gc`
feature at all, so a `no-gc` workspace build stopped at the CLI. It now
forwards `no-gc` to its runtime dependencies, `cljrs-compiler` forwards to it,
and `cljrs` forwards weakly (`cljrs-async?/no-gc`) since its async dependency
is optional.

Deferred by design: `defn_registry`'s cross-defn registry stays keyed by
`GlobalEnv::id` in process-global maps rather than moving into `Tiers`. It is
correct as it stands (Stage 3 replaced its address-derived keys with counter
ids), it is not JIT state, and moving it would have enlarged an already large
stage. `cljrs-compiler`'s `cljrs-env`/`cljrs-interp`/`cljrs-eval` imports are
still the Stage 2 shims; Stage 6 rewrites them.

### Stage 5: Consolidate project and CLI tools

1. Create `cljrs-project` from `cljrs-deps` and `cljrs-vcs`.
2. Move dynamic native-package loading into the CLI.
3. Move IR visualization into `cljrs::commands::ir`.
4. Move retained IR bundle code into the same command module.
5. Move `cljrs-dom` into `cljrs-wasm::dom`.
6. Replace `cljrs-logging` with `tracing` targets and filters.
7. Split `cljrs/src/main.rs` into command modules.

Keep the `cljrs-lsp` and `cljrs-nrepl` packages.
They produce distinct tools and have reusable server APIs.

Validation gate:

- The CLI depends on product-level packages instead of internal runtime layers.
- Project configuration and VCS operations use one package.
- Each standalone artifact still builds independently.
- CLI help and command behavior remain compatible.

### Stage 6: Remove compatibility packages

1. Update all downstream imports to the target packages.
2. Remove temporary re-export packages.
3. Remove unused workspace dependencies and feature propagation.
4. Update the root README and every affected crate README.
5. Archive or remove obsolete implementation plans.
6. Regenerate the dependency graph and baseline measurements.

Validation gate:

- The workspace contains approximately 23 packages.
- `cljrs-compiler` depends mainly on `cljrs-runtime`, `cljrs-ir`, and `cljrs-project`.
- The CLI does not depend directly on internal execution modules.
- All required build and test configurations pass.

## Risks

### GC roots across moved modules

The package merge changes paths but must not change root lifetimes.
Move root-tracing tests before any state redesign.

### Global state leaks between runtimes

Current IR and JIT tables contain process-global state.
Move this state into `Runtime` before removal of compatibility packages.

### JIT code reclamation

The JIT uses hooks for stale code, active epochs, and pending exceptions.
Preserve these invariants during the move into the compiler package.

### Optional extension behavior

AOT currently initializes several extensions automatically.
An extension registry must preserve default CLI behavior and custom embedding behavior.

### Public Rust API changes

The project has version `0.1.0`, but examples and external users can use current package paths.
Keep re-export packages for one migration stage and document replacement paths.

### Large review size

Package moves can hide behavior changes.
Separate mechanical moves from API redesign and state redesign.

## Completion criteria

When all criteria are true, the cleanup is complete:

- The workspace contains approximately 23 packages.
- Every remaining package satisfies at least one boundary rule.
- The runtime has one builder and one execution dispatch path.
- Runtime instances own their tier and cache state.
- The compiler does not initialize optional product extensions directly.
- JIT and AOT code generation share one compiler package.
- The CLI imports product APIs instead of internal layers.
- Default, `no-gc`, JIT, AOT, transaction, LSP, nREPL, and WASM validation passes.
- All crate READMEs describe their current files and public APIs.

## Recommended first pull request

Implement Stage 1 only.
This stage removes obsolete files and dependencies.
It does not change the runtime architecture.

The pull request must include corrected documentation and a new dependency graph.
The runtime merge starts only after this pull request passes the full validation gate.
