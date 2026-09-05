# cljrs (Clojurust CLI)

The `cljrs` binary — command-line interface for running, compiling, and
interactively exploring clojurust programs.

**[Full documentation →](https://docs.clj.rs)**

---

## File layout

```
src/
  main.rs           — the binary: a one-line shim over `cli::main`
  lib.rs            — module index; the CLI lives in a library so its own
                      integration tests can reach it (not an embedding API)
  cli.rs            — global flags, the miette error hook, the tracing
                      subscriber, the large-stack worker thread, and the
                      subcommand dispatcher
  session.rs        — everything more than one subcommand needs: `setup_globals`
                      (runtime + stdlib + `cljrs.edn` wiring + JIT policy),
                      source-path helpers, `eval_in` / `eval_form`, the async
                      driver, and error formatting
  native/           — loading native (Rust) code into a running environment
    mod.rs          — the project's own `:rust` cdylib and the cargo/path helpers
    pinned.rs       — pinned native packages (`:rust/load :dylib`): wrapper
                      generation, cargo build + cache, dlopen + ABI handshake
  extensions.rs     — `default_set()`: the runtime extensions this build ships,
                      handed to the compiler for `cljrs compile` (the compiler
                      backend does not choose them)
build.rs            — captures `rustc -V` for the pinned-package ABI fingerprint
  commands/         — one module per subcommand: its clap `Args` and its `run`
    mod.rs          — module index
    run.rs          — `run`: interpret a file, then call `-main`
    repl.rs         — `repl`: the interactive loop
    compile.rs      — `compile`: `CompileTarget`, entry-namespace resolution,
                      opacity policy, native and wasm AOT
    eval.rs         — `eval`: one expression
    ir/             — `ir`: `IrCommands` enum, dispatch, bundle pre-lowering
      mod.rs        —   (`ir build`) and bundle dump
      viz/          — `ir viz`: the self-contained HTML IR visualizer
        mod.rs      —   `render_html` / `RenderOptions`
        render.rs   —   HTML assembly, region colouring, source pane
        region.rs   —   `RegionStart`/`RegionEnd` pairing and membership
        blame.rs    —   escape-verdict badges and the blamed use
    test.rs         — `test`: namespace discovery, the runner, the summary
    deps.rs         — `deps fetch` / `deps status`
    build_native.rs — `build-native`: cargo-build the project's `:rust` crate
    lsp.rs          — `lsp`: run the language server over stdio
    nrepl.rs        — `nrepl`: serve an nREPL session
tests/
  embedding_book_examples.rs — the snippets from the book's Embedding chapter
                      (`docs/book/src/embedding/`), compiled and run so the
                      documented host API cannot drift
  pinned_dylib_e2e.rs — gated (`CLJRS_DYLIB_E2E=1`) pinned-native end-to-end
                      test; described under Pinned native packages below
  core_shadowing_tiers.rs — shadowed `clojure.core` names (a `let`-bound `inc`,
                      a parameter named `inc`, a redefined var, an excluded
                      name) must answer the same before and after a function
                      crosses the IR promotion threshold, and once more when
                      the file is AOT-compiled (issue #337)
  self_name_shadowing_tiers.rs — a parameter sharing the function's OWN name
                      (`(defn text [text] ...)`, plus rest, multi-arity and
                      hot-loop forms) must shadow it, and tree-walk and eager
                      IR must agree (PR #353)
```

---

## Subcommands

| Subcommand    | Purpose                                                                |
|---------------|------------------------------------------------------------------------|
| `run`         | Interpret a `.cljrs` / `.cljc` source file                             |
| `repl`        | Start an interactive REPL                                              |
| `compile`     | AOT-compile a source file or project (via `cljrs.edn`) to a native binary, or a `.wasm` module with `--target wasm` |
| `eval`        | Evaluate a single Clojure expression and print the result              |
| `ir build`    | Pre-lower namespaces to IR and write a serialized bundle               |
| `ir dump`     | Print a human-readable dump of a serialized IR bundle                  |
| `ir viz`      | Render the optimized IR + source as a self-contained HTML visualizer   |
| `test`        | Run `clojure.test` namespaces (named on the CLI or auto-discovered)    |
| `deps fetch`  | Clone / update git dependencies declared in `cljrs.edn`                |
| `deps status` | Show which dependencies are cached and which are missing               |

### -main entry point

After all top-level forms in the source file are evaluated, `cljrs run` looks
up `-main` in the current namespace.  If the var exists it is called with the
arguments that follow `--` on the command line, each as an individual string:

```bash
cljrs run app.cljrs -- hello world   # calls (-main "hello" "world")
cljrs run app.cljrs                  # calls (-main) if -main is defined
```

The same convention applies to AOT binaries produced by `cljrs compile`: the
compiled binary calls `-main` after `__cljrs_main` finishes, passing all
`argv` entries (skipping the program name) as individual string arguments.

If `-main` is not defined the program exits normally without error.

An `^:async` `-main` is supported: calling it returns a `Future` immediately,
so `cljrs run` awaits that future on the shared async `LocalSet` (see
implementation notes) before exiting, ensuring the body and anything it spawns
run to completion. A synchronous `-main` is awaited as a no-op pass-through.

### Per-subcommand flags

`run`, `repl`, `compile`, `test` accept:
- `--src-path <DIR>` — repeatable; directories searched by `require`
- `--gc-soft-limit-mb <MB>` — soft GC threshold
- `--gc-hard-limit-mb <MB>` — hard GC threshold

`run` additionally accepts:
- `[-- ARGS…]` — positional arguments forwarded verbatim to `-main`

`compile` additionally accepts:
- `-o, --out <PATH>` — output path (required): a native binary, or a `.wasm` module with `--target wasm`
- `--target <native|wasm>` - code-generation target (default `native`), a closed set validated by clap. `wasm` emits a WebAssembly module via the AOT wasm backend (the entry namespace's functions; the `"rt"` imports are satisfied by the runtime built for `wasm32-unknown-unknown`). `--test` is not yet supported with `wasm`.
- `--main <NS>` — namespace containing `-main`; overrides `:main` in `cljrs.edn` and auto-detection
- `--test` — compile a test harness that runs every test in the given file/directory
- `--require-fully-compiled` - fail the build if the artifact would not fully represent the program. On `--target native` that means embedded readable Clojure source (interpreted preambles, bundled namespaces); on `--target wasm`, which embeds no source, it means a namespace or entry form the backend dropped. `--test` cannot satisfy it (the harness bundles every test namespace as source) and is refused.

`ir build` accepts:
- `-n, --ns <NS>` — repeatable; namespaces to lower (default `clojure.core`)
- `-o, --output <PATH>` — output bundle path (default `ir_bundle.bin`)
- `--src-path <DIR>` — repeatable; source directories for `require`-ing non-`clojure.core` namespaces
- `-v, --verbose` — print per-arity lowering progress

`ir dump` takes a single positional bundle path and prints the IR of every function it contains.

`ir viz` accepts:
- `-o, --out <PATH>` — output HTML path (defaults to `<file>.ir.html`)
- `--src-path <DIR>` — repeatable
- `--quiet` — suppress the `[aot] …` progress output

`test` additionally accepts:
- `[namespaces…]` — positional list; if empty, namespaces are auto-discovered under `--src-path`
- `-v, --verbose` — print each passing assertion (helps isolate hangs)

`eval` takes a single positional expression string.

`deps fetch` accepts an optional positional dependency name; without it all git
deps are fetched.  `deps status` takes no arguments.

### `cljrs.edn` auto-discovery

When any command that runs code (`run`, `repl`, `eval`, `test`, `compile`)
starts, it walks up the directory tree from the current working directory
looking for a `cljrs.edn` file.  If found, its `:paths` entries are appended
to the source search path (after any `--src-path` CLI flags), and the parsed
`DepsConfig` is stored in `GlobalEnv.deps_config` so that versioned symbol
resolution can use it without a second parse.

#### `compile` and `cljrs.edn`

`cljrs compile` reads `cljrs.edn` to determine:

1. **Source paths** — `:paths` entries are added to `--src-path` (CLI flags come first).
2. **Dependency source roots** — each dep's source directories are resolved and appended so `require` resolves correctly during compilation.
3. **Entry-point namespace** — determined by the following priority:
   - `--main <NS>` CLI flag (highest priority)
   - `:main` key in `cljrs.edn` (e.g. `:main my.app.core`)
   - Auto-detection: scans `:paths` for a unique `-main` function; errors if zero or multiple are found

When `cljrs.edn` is present and the entry-point namespace is known, the
`file` positional argument may be omitted; the compiler finds the source file
for the main namespace automatically.

Each declared dependency's own source roots are also appended to the search
path, so a plain `(require '[dep.ns :as …])` resolves namespaces provided by a
dependency:

- **Local deps** (`:local/root`) contribute their `cljrs.edn` `:paths` (or
  `src/`) from the directory on disk.
- **Git deps** are materialized from the local bare cache at their pinned
  `:git/sha` (no network — run `cljrs deps fetch` first; a missing cache warns
  and is skipped), and contribute the checkout's `:paths` (or `src/`).
- **Native deps** (`:rust/load :dylib`) carry no Clojure source; they are built
  and registered on demand by the native-`require` loader (`native::pinned`) when
  their namespace is first `require`d.

### Global flags

These appear before the subcommand and apply to every command:

- `--stack-size-mb <MB>` — thread stack size (default 64).  Raise if you hit stack overflows in deeply recursive code.
- `--debug` — enable debug logging
- `--trace` — enable trace logging (implies `--debug`)

  Codegen crates (`cranelift_*`, `regalloc2`) are pinned to `warn` at all
  verbosity levels — `cranelift-jit`/`cranelift-object` log every compiled
  function's whole CLIF body at `info`, which otherwise buries real output.
  Set `RUST_LOG` (`tracing` target=level syntax) to replace the defaults and
  get them back, e.g. `RUST_LOG=info,cranelift_jit=info cljrs run app.cljrs`.

- `-X <LEVEL:FEATURES>` — feature-level logging, repeatable.  Format: `<level>:<feat1>,<feat2>,…`.  Levels: `debug`, `trace`.  Features: `gc`, `env`, `ir`, `jit`.  Example: `-X debug:gc,jit`.  These are `tracing` targets: `RUST_LOG=gc=debug` does the same thing, and `-X` is layered on top of `RUST_LOG` so both can be used together.  A blanket `--debug`/`--trace` deliberately leaves them off — they are firehoses.  A malformed `-X` is a hard error; a malformed `RUST_LOG` is reported on stderr and ignored, leaving the `--debug`/`--trace` default in place.  An AOT binary reads the same two variables (`CLJRS_X_FLAG` in place of `-X`) and treats a bad value in each exactly the same way.
- `--gc-stats [FILE]` — print a `cljrs_gc::GC_STATS` snapshot at program exit (allocations, region/bump usage, GC pause count + total duration, freed objects/bytes).  No value → stdout; with a path → that file.  Honoured by `run`, `eval`, and `test`.
- `--jit-stats [FILE]` — print a JIT specialization / inline-cache counter snapshot at program exit (boxed arithmetic bridge calls, entry-guard deopts, keyword IC fills, protocol IC hits/misses; Phase 10.6, `cljrs_compiler::rt_abi::jit_stats`).  No value → stdout; with a path → that file.  Honoured by `run`, `eval`, and `test`.

---

## Examples

```bash
# Interpret a file
cljrs run hello.cljrs
cljrs run main.cljrs --src-path src --src-path lib
cljrs run app.cljrs -- arg1 arg2    # args forwarded to -main

# REPL
cljrs repl --src-path src

# AOT compile to a native binary
cljrs compile app.cljrs -o app
cljrs compile -o app                       # project mode: reads cljrs.edn
cljrs compile --main my.app.core -o app   # specify entry namespace explicitly
cljrs compile tests/ -o run-tests --test --src-path src

# One-shot expression
cljrs eval '(+ 1 2)'

# Render IR visualizer (writes samples/graph.cljrs.ir.html, open in any browser)
cljrs ir viz samples/graph.cljrs
cljrs ir viz samples/graph.cljrs -o /tmp/graph.html --quiet

# Pre-lower namespaces to an IR bundle (replayed by cljrs_runtime::tiered::load_prebuilt_ir;
# no cljrs runtime path loads one today - these are lowerer diagnostics)
cljrs ir build --ns clojure.core -o core.ir.bin
cljrs ir build --ns my.app.core --src-path src -o app.ir.bin -v
cljrs ir dump app.ir.bin

# Tests
cljrs test --src-path src/ --src-path test/ my-ns.my-tests
cljrs test --src-path src/ -v       # auto-discover, verbose

# GC stats
cljrs run main.cljrs --gc-stats              # → stdout
cljrs eval '(reduce + (range 1e6))' --gc-stats stats.txt
cljrs test --src-path test/ --gc-stats /tmp/test-gc.log

# Bigger stack + tracing for one feature
cljrs --stack-size-mb 256 -X trace:gc run heavy.cljrs

# Dependency management (reads cljrs.edn from the current directory tree)
cljrs deps fetch               # clone/update all git deps
cljrs deps fetch my.lib        # fetch one dep by name
cljrs deps status              # show cached vs missing deps
```

---

## Build features

| Feature             | Effect                                                                        |
|---------------------|-------------------------------------------------------------------------------|
| `async` (default **on**) | Pulls in `cljrs-async` and `cljrs-io` and builds the Tokio runtime that drives top-level async evaluation (see implementation notes). Without it, `^:async`/`core.async`/`clojure.rust.io.async` are unavailable and evaluation is purely synchronous. |
| `net`, `charset`, `base64` (default **on**) | Network transports and protocols, charset codecs, Base64.  Each feature adds its package to both the interpreted runtime (`setup_globals`) and the compile-time extension set (`extensions::default_set`), so `cljrs run` and `cljrs compile` of the same program see the same namespaces. |
| `no-gc` (default off) | Propagated to `cljrs-gc`/`cljrs-value`/`cljrs-runtime`/`cljrs-compiler`/`cljrs-stdlib` (and weakly to `cljrs-async`).  Disables the tracing GC; only region-allocated and stack values are permitted.  Compiles fail (`AotError::NoGcBlacklist`) if the program contains allocations the optimizer can't lift onto regions. |
| `enable-rustyline`  | Pulls in `rustyline` for a line-editing REPL.  Without it, `cljrs repl` falls back to a plain `BufRead` loop.                                                                                |

Build with e.g. `cargo build --release --features enable-rustyline,no-gc`.

---

## Implementation notes

- Argument parsing uses [Clap](https://docs.rs/clap) derive macros (`Parser`, `Subcommand`).
- The miette error hook is installed at startup so `CljxError` propagated to `main` renders with terminal-linked source snippets.
- A worker thread is spawned with the configured stack size to run the actual command; the main thread only handles signal/exit setup.
- The REPL prints results, paginates errors via `miette`, and persists multi-line input across blank prompts.
- **Top-level async (with the `async` feature).** `session::with_async_driver` builds a single-threaded Tokio runtime + `LocalSet` and stashes it in a thread-local `AsyncDriver` rather than wrapping the whole session in one `block_on`. Each top-level form is then evaluated through `cljrs_async::eval_async` via `LocalSet::block_on` in `eval_form`, so spawned tasks (core.async producers, `^:async` calls, `clojure.rust.io.async` readers/writers) make progress and a top-level `await` resolves. Tasks that outlive a form — e.g. a channel `def`d at one REPL prompt and consumed at the next — stay queued on the shared `LocalSet` and continue on the next form's drive. Note: blocking ops (`<!!`/`>!!`) still park the single executor thread and so are not usable at the top level; use `(await (take! ch))` / `go` instead.
- `ir viz` runs the AOT pipeline through region optimization (via `cljrs_compiler::aot::lower_file_to_ir`) and hands the resulting `IrFunction` to `commands::ir::viz::render_html`.
- `ir build` boots a standard environment, walks every var in the requested namespaces, and lowers each function arity with `cljrs_runtime::tiered::lower::lower_arity` into an `IrBundle`. It lives in `commands/ir/mod.rs`; there is no separate pre-build crate or binary. No `cljrs` runtime path loads a bundle — `cljrs_runtime::tiered::load_prebuilt_ir` is the public API an embedder would call to replay one.

---

## Dependencies

| Crate                       | Role                                                              |
|-----------------------------|-------------------------------------------------------------------|
| `cljrs-types` (workspace)   | `CljxError` for `miette::Result` propagation; `Span`              |
| `cljrs-gc` (workspace)      | GC root, configuration, `GC_STATS` snapshot                       |
| `cljrs-reader` (workspace)  | Lexer + parser                                                    |
| `cljrs-value` (workspace)   | `Value` and persistent collections                                |
| `cljrs-runtime` (workspace) | Runtime construction (`Runtime::builder`) and evaluation; `env::Env`, the `interp` tree walker, and `tiered` lowering |
| `cljrs-stdlib` (workspace)  | Standard library installed into the runtime (`install`)           |
| `cljrs-compiler` (workspace)| AOT pipeline (`compile_file`, `compile_test_harness`, `lower_file_to_ir`) |
| `cljrs-ir` (workspace)      | `IrBundle`, `serialize_bundle`, `deserialize_bundle` — used by `ir build` / `ir dump` |
| `cljrs-interop` (workspace) | Rust ↔ Clojure FFI                                                |
| `cljrs-async` (workspace, optional) | `clojure.core.async` runtime + `eval_async`; enabled by `async`  |
| `cljrs-io` (workspace, optional) | `clojure.rust.io.async` async file I/O; enabled by `async`       |
| `tokio` (workspace, optional) | Single-threaded runtime + `LocalSet` driving async; enabled by `async` |
| `tracing` (workspace)       | `Level` for the `--debug` / `--trace` default; `--debug` / `--trace` / `-X` all build one `Targets` filter and install the stderr subscriber through `cljrs_runtime::logging`, which owns the `tracing-subscriber` dependency |
| `cljrs-project` (workspace) | `config` — `cljrs.edn` parser, `DepsConfig` / `Dependency` types; `vcs` — pure-Rust (gitoxide) git helpers: `fetch_remote`, `cache_path_for_url`, native signature verification |
| `clap` (workspace)          | CLI argument parsing                                              |
| `miette` (workspace, `fancy`) | Rich terminal error rendering. `fancy` — the renderer half of miette — is enabled *here only*; the library crates take miette without it |
| `rustyline` (workspace, optional) | Line-editing REPL when `enable-rustyline` is on              |
| `libloading` (workspace)    | `dlopen` for the project `:rust` cdylib and pinned native packages |
| `serde_json`                | Reading `target_directory` out of `cargo metadata` output          |

---

## Pinned native packages (`:rust/load :dylib`)

### Purpose

Pinned native packages: build a dependency's Rust crate at a pinned git
commit as a cdylib and load it, so versioned symbols (`my.lib/f@<sha>`) can
resolve to **truly pinned** native code instead of the default verified HEAD
binding (`:rust/load :dylib` in `cljrs.edn`).  The same machinery also makes a
`:rust/load :dylib` dependency loadable by a **plain `require`** of its
namespace, registering the package's exports into the live (unversioned)
namespace.

### Status

Versioned-namespaces plan, Phase 5 (see `docs/archive/versioned-namespaces-plan.md`).
Implemented and tested end-to-end, but **experimental**: the init call
crosses a Rust-ABI boundary guarded only by the fingerprint handshake
(feature-flag skew between host and wrapper is not detected), and a Rust
toolchain is required at runtime.  Statically linking pinned native crates
into AOT harnesses is deferred (open problem: `#[export]` inventory
collisions between two versions of one crate).

### File layout

```
src/native/pinned.rs — install (both loader hooks), wrapper crate generation,
              cargo build + cache, dlopen + ABI handshake, versioned/unversioned
              Registry init
build.rs    — captures `rustc -V` for the host side of the ABI fingerprint
tests/
  pinned_dylib_e2e.rs — gated end-to-end test (CLJRS_DYLIB_E2E=1): two-commit
              native crate fixture; pinned (versioned-symbol) resolution loads
              the v1 dylib while HEAD stays untouched, and a plain `require`
              loads the v1 dylib into the unversioned namespace
```

### Public API

```rust
/// Install both native loader hooks on the environment (idempotent): the
/// pinned-native loader (versioned-symbol resolution) and the native-require
/// loader (plain `require` of a `:rust/load :dylib` dep).  Called by the
/// cljrs CLI during setup_globals.
pub fn install(globals: &Arc<GlobalEnv>);   // cljrs::native::pinned

/// The host's ABI fingerprint: "cljrs <version>; <rustc -V>; <debug|release>".
/// A wrapper dylib is loaded only when its baked fingerprint equals this.
pub fn abi_fingerprint() -> String;

pub const ABI_SYMBOL: &[u8];   // b"cljrs_dylib_abi\0"
pub const INIT_SYMBOL: &[u8];  // b"cljrs_dylib_init\0"
```

### How it works

1. The versioned resolver (`cljrs_runtime::env::versioned`) calls the installed
   `PinnedNativeLoader` when a pinned lookup is about to fall back to a
   native function.
2. The loader finds a `:rust/load :dylib` git dep covering the namespace
   (exact or dotted-prefix match) with a `:rust/init` function.
3. `cljrs_project::vcs::fetch_remote` + a gitoxide worktree checkout of the pinned
   commit's tree (`~/.cljrs/cache/dylibs/checkouts/<crate>@<commit>`, no
   `.git`; a `.cljrs-checkout-complete` sentinel marks a finished checkout).
4. A wrapper cdylib crate is generated
   (`~/.cljrs/cache/dylibs/<crate>@<commit>/fp-<hash>/`), pinning the same
   `cljrs-interop` as the host (local checkout path when found —
   `CLJRS_WORKSPACE_ROOT` override honored — else the published `=version`),
   and built with cargo **in the host's profile** (debug/release —
   `cljrs-gc` object headers differ between profiles).
5. dlopen → `cljrs_dylib_abi()` fingerprint must equal
   `abi_fingerprint()` exactly, else refuse → `cljrs_dylib_init(*mut
   Registry)` registers the package's exports through
   `Registry::versioned(commit)`, landing every definition in the immutable
   `"<ns>@<commit>"` namespace.
6. The namespace is marked loaded; subsequent pinned lookups are plain
   namespace hits.

#### Plain `require` of a native dep

When `(require '[my.native.lib :as l])` finds no Clojure source for the
namespace, `cljrs-runtime`'s unversioned loader consults the installed
`NativeRequireLoader`.  It runs the same fetch/checkout/wrapper-build pipeline
(steps 2–4 above), keyed on the dep's pinned `:git/sha`, then runs
`cljrs_dylib_init` through `Registry::for_require(...)` — an **unversioned**
view — so the exports land in the live `my.native.lib` namespace.  The loader
returns and the unversioned loader marks the namespace loaded, so `l/encode`
resolves like any other namespace.

---

## The IR visualizer (`cljrs ir viz`)

**Purpose:** debug the bump-allocation optimizer.  When a value escapes
or otherwise misses region promotion, the visualizer flags it with the
escape-analysis verdict and the use that "blamed" it — making it
obvious why the optimizer left it on the GC heap.

**Status:** implemented and tested against hand-written snippets; not
integrated with the AOT compiler's `--emit-ir-html` flag — `cljrs ir viz`
is the interface.  This was the `cljrs-ir-viz` package until consolidation
stage 5; the CLI was its only consumer.

---

### File layout

```
src/commands/ir/viz/
  mod.rs    — public entry point: `render_html` and `RenderOptions`
  render.rs — top-level HTML assembly, function/block/inst rendering,
              source-pane rendering, region color assignment
  region.rs — collect `RegionStart`/`RegionEnd` pairs, compute the set
              of `(block, inst_index)` positions covered by each region
  blame.rs  — pick a representative "blame" use for a non-promoted
              allocation; format use-kind labels and escape-state badges
tests/
  ir_viz.rs — lower a small snippet, render to HTML, and assert the
              output is well-formed and contains expected markers
examples/
  ir_viz_dump.rs — `cargo run -p cljrs --example ir_viz_dump > /tmp/ir.html`
              renders a hand-written demo to stdout
```

---

### Usage

#### CLI

```sh
cljrs ir viz path/to/file.cljrs        # writes path/to/file.cljrs.ir.html
cljrs ir viz path/to/file.cljrs -o out.html
cljrs ir viz path/to/file.cljrs --src-path src/    # for require resolution
```

#### From Rust

```rust
use cljrs::commands::ir::viz::{render_html, RenderOptions};
use cljrs_ir::lower::{lower_fn_body, optimize};

let ir = optimize(lower_fn_body(Some("f"), "user", &[], &forms)?);
let html = render_html(&ir, Some(source_text), &RenderOptions::default());
std::fs::write("ir.html", html)?;
```

---

### Public API

```rust
pub fn render_html(
    ir: &cljrs_ir::IrFunction,
    source: Option<&str>,
    opts: &RenderOptions,
) -> String;

pub struct RenderOptions {
    pub title: Option<String>,
}
```

`render_html` walks `ir` plus all subfunctions, runs escape analysis with
an inter-procedural context, and produces a complete HTML document.  The
return value is a self-contained string suitable for writing to disk and
opening in any browser.

---

### What the visualizer shows

For each function:

* **Header** — function name (with parent path for subfunctions),
  parameter list, and source span when known.
* **Allocation summary** — count of region-allocated, heap, and closure
  allocations.
* **Per-block IR** — every instruction with its index, with kinds
  color-coded:
  * `alloc` (heap) — orange
  * `ralloc` (region) — green, with strong tint matching the region's
    color
  * `rstart` / `rend` — italic gray
  * `call`, `store`, `loc`, etc.
* **Region coloring** — every `RegionStart`/`RegionEnd` pair gets a
  deterministic hue (golden-angle spacing).  Instructions inside the
  region get a pale tint of that hue; the actual `RegionAlloc` /
  `RegionStart` / `RegionEnd` markers get a stronger tint plus an accent
  border.  Source lines that produced any of the region's
  `RegionAlloc`s get the same accent border in the gutter.
* **Escape badges** — every `Alloc*` instruction (i.e. one that did
  *not* get promoted) shows its escape verdict (`no-escape`,
  `arg-escape`, `returns`, `escapes`) and the blamed use (e.g. *"return
  value"*, *"stored into heap object in bb1"*, *"arg 0 of known call
  Map"*).  Pure `no-escape` allocations are unusual after optimization
  and indicate a missed promotion opportunity.
* **Hover linking** — hovering an IR instruction highlights its source
  line; hovering a source line highlights all IR insts derived from it.
  Lookup is by line number via `data-line` attributes.

---

### Notes on source mapping

ANF lowering emits `Inst::SourceLoc(span)` markers at the head of each
form's lowering, deduped per `(file, line)` within a block.  These are
pure no-op instructions (`Effect::Pure`, no `dst`) so all existing
analysis and code-generation passes ignore them — they exist only for
this visualizer and other downstream tooling.

The `IrFunction.span` field is currently populated only for
hand-constructed IR; the ANF lowering path does not yet set it for
top-level functions.  Subfunction headers therefore show only their
first `SourceLoc` marker rather than a span range.

---

---

## Regex engine and pass-through features

Beyond `async`/`net`/`charset`/`base64`/`no-gc`/`enable-rustyline`, the CLI
re-exports what its workspace dependencies' defaults used to provide, because all
of them are now taken with default features off (see the note in the root
`Cargo.toml`). All three are in `default`, so a plain `cargo build -p cljrs` is
unchanged:

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to every workspace dependency — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`, which trades Unicode character classes for ~277 KB of text. |
| `deps` | **on** | Forwards `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |

The forwards to the optional extension crates use Cargo's weak `?/` syntax, so
they apply only when the feature that pulls the package in (`async`, `net`,
`charset`, `base64`) is also enabled.

`cljrs-compiler`'s `wasm-aot` is *not* a pass-through: `commands::compile` calls
`aot::compile_file_to_wasm` unconditionally for `--target wasm`, so the CLI's
dependency pins that feature on rather than offering a knob that would fail to
compile.

One caveat specific to the CLI: `--no-default-features --features small-regex`
does switch `Value::Pattern` to `regex-lite`, but it does **not** remove the
`regex` crate from the binary. `cljrs` depends on `cljrs-project` with
`features = ["vcs", "vcs-net", "ssh"]` unconditionally — `cljrs deps` needs git
fetch and commit-signature verification — and `pgp` pulls `regex` in. A CLI built
that way therefore links both engines and is *larger*, not smaller. The engine
swap only pays off for an embedding that has no `pgp` in its graph; see
[cljrs-value's README](../cljrs-value/README.md#features).
