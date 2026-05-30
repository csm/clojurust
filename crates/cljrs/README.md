# cljrs (Clojurust CLI)

The `cljrs` binary — command-line interface for running, compiling, and
interactively exploring clojurust programs.

**[Full documentation →](https://docs.clj.rs)**

---

## File layout

```
src/
  main.rs   — CLI entry point: Clap structs, miette error hook, subcommand
              dispatch, REPL loop, test harness, GC-stats reporter
```

---

## Subcommands

| Subcommand    | Purpose                                                                |
|---------------|------------------------------------------------------------------------|
| `run`         | Interpret a `.cljrs` / `.cljc` source file                             |
| `repl`        | Start an interactive REPL                                              |
| `compile`     | AOT-compile a source file (or test directory) to a native binary       |
| `eval`        | Evaluate a single Clojure expression and print the result              |
| `ir-viz`      | Render the optimized IR + source as a self-contained HTML visualizer   |
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

### Per-subcommand flags

`run`, `repl`, `compile`, `test` accept:
- `--src-path <DIR>` — repeatable; directories searched by `require`
- `--gc-soft-limit-mb <MB>` — soft GC threshold
- `--gc-hard-limit-mb <MB>` — hard GC threshold

`run` additionally accepts:
- `[-- ARGS…]` — positional arguments forwarded verbatim to `-main`

`compile` additionally accepts:
- `-o, --out <PATH>` — output binary path (required)
- `--test` — compile a test harness that runs every test in the given file/directory

`ir-viz` accepts:
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

When any command that runs code (`run`, `repl`, `eval`, `test`) starts, it
walks up the directory tree from the current working directory looking for a
`cljrs.edn` file.  If found, its `:paths` entries are appended to the source
search path (after any `--src-path` CLI flags), and the parsed `DepsConfig` is
stored in `GlobalEnv.deps_config` so that versioned symbol resolution can use
it without a second parse.

### Global flags

These appear before the subcommand and apply to every command:

- `--stack-size-mb <MB>` — thread stack size (default 64).  Raise if you hit stack overflows in deeply recursive code.
- `--debug` — enable debug logging
- `--trace` — enable trace logging (implies `--debug`)
- `-X <LEVEL:FEATURES>` — feature-level logging, repeatable.  Format: `<level>:<feat1>,<feat2>,…`.  Levels: `debug`, `trace`.  Example: `-X debug:gc,jit`.
- `--gc-stats [FILE]` — print a `cljrs_gc::GC_STATS` snapshot at program exit (allocations, region/bump usage, GC pause count + total duration, freed objects/bytes).  No value → stdout; with a path → that file.  Honoured by `run`, `eval`, and `test`.

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
cljrs compile tests/ -o run-tests --test --src-path src

# One-shot expression
cljrs eval '(+ 1 2)'

# Render IR visualizer (writes samples/graph.cljrs.ir.html, open in any browser)
cljrs ir-viz samples/graph.cljrs
cljrs ir-viz samples/graph.cljrs -o /tmp/graph.html --quiet

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
| `no-gc` (default off) | Propagated to `cljrs-gc`/`cljrs-value`/`cljrs-eval`/`cljrs-compiler`/`cljrs-runtime`/`cljrs-stdlib`.  Disables the tracing GC; only region-allocated and stack values are permitted.  Compiles fail (`AotError::NoGcBlacklist`) if the program contains allocations the optimizer can't lift onto regions. |
| `enable-rustyline`  | Pulls in `rustyline` for a line-editing REPL.  Without it, `cljrs repl` falls back to a plain `BufRead` loop.                                                                                |

Build with e.g. `cargo build --release --features enable-rustyline,no-gc`.

---

## Implementation notes

- Argument parsing uses [Clap](https://docs.rs/clap) derive macros (`Parser`, `Subcommand`).
- The miette error hook is installed at startup so `CljxError` propagated to `main` renders with terminal-linked source snippets.
- A worker thread is spawned with the configured stack size to run the actual command; the main thread only handles signal/exit setup.
- The REPL prints results, paginates errors via `miette`, and persists multi-line input across blank prompts.
- **Top-level async (with the `async` feature).** `main` builds a single-threaded Tokio runtime + `LocalSet` and stashes it in a thread-local `AsyncDriver` rather than wrapping the whole session in one `block_on`. Each top-level form is then evaluated through `cljrs_async::eval_async` via `LocalSet::block_on` in `eval_form`, so spawned tasks (core.async producers, `^:async` calls, `clojure.rust.io.async` readers/writers) make progress and a top-level `await` resolves. Tasks that outlive a form — e.g. a channel `def`d at one REPL prompt and consumed at the next — stay queued on the shared `LocalSet` and continue on the next form's drive. Note: blocking ops (`<!!`/`>!!`) still park the single executor thread and so are not usable at the top level; use `(await (take! ch))` / `go` instead.
- `ir-viz` runs the AOT pipeline through region optimization (via `cljrs_compiler::aot::lower_file_to_ir`) and hands the resulting `IrFunction` to `cljrs_ir_viz::render_html`.

---

## Dependencies

| Crate                       | Role                                                              |
|-----------------------------|-------------------------------------------------------------------|
| `cljrs-types` (workspace)   | `CljxError` for `miette::Result` propagation; `Span`              |
| `cljrs-gc` (workspace)      | GC root, configuration, `GC_STATS` snapshot                       |
| `cljrs-reader` (workspace)  | Lexer + parser                                                    |
| `cljrs-value` (workspace)   | `Value` and persistent collections                                |
| `cljrs-eval` (workspace)    | Tree-walking interpreter, `Env`                                   |
| `cljrs-stdlib` (workspace)  | Bootstrapped standard library (`standard_env*`)                   |
| `cljrs-runtime` (workspace) | Concurrency primitives consumed by stdlib                         |
| `cljrs-compiler` (workspace)| AOT pipeline (`compile_file`, `compile_test_harness`, `lower_file_to_ir`) |
| `cljrs-ir-viz` (workspace)  | HTML IR visualizer used by `ir-viz`                                |
| `cljrs-interop` (workspace) | Rust ↔ Clojure FFI                                                |
| `cljrs-async` (workspace, optional) | `clojure.core.async` runtime + `eval_async`; enabled by `async`  |
| `cljrs-io` (workspace, optional) | `clojure.rust.io.async` async file I/O; enabled by `async`       |
| `tokio` (workspace, optional) | Single-threaded runtime + `LocalSet` driving async; enabled by `async` |
| `cljrs-logging` (workspace) | `--debug` / `--trace` / `-X` flag handling                        |
| `cljrs-deps` (workspace)    | `cljrs.edn` parser; `DepsConfig` / `Dependency` types             |
| `cljrs-vcs` (workspace)     | Git subprocess helpers: `fetch_remote`, `cache_path_for_url`      |
| `clap` (workspace)          | CLI argument parsing                                              |
| `miette` (workspace)        | Rich terminal error rendering                                     |
| `tracing` / `tracing-subscriber` | Structured logging output                                    |
| `rustyline` (workspace, optional) | Line-editing REPL when `enable-rustyline` is on              |
