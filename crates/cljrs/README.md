# cljrs (Clojurust CLI)

The `cljrs` binary — command-line interface for running, compiling, and
interactively exploring clojurust programs.

---

## File layout

```
src/
  main.rs   — CLI entry point: Clap structs, miette error hook, subcommand
              dispatch, REPL loop, test harness, GC-stats reporter
```

---

## Subcommands

| Subcommand | Purpose                                                                |
|------------|------------------------------------------------------------------------|
| `run`      | Interpret a `.cljrs` / `.cljc` source file                              |
| `repl`     | Start an interactive REPL                                               |
| `compile`  | AOT-compile a source file (or test directory) to a native binary        |
| `eval`     | Evaluate a single Clojure expression and print the result               |
| `ir-viz`   | Render the optimized IR + source as a self-contained HTML visualizer    |
| `test`     | Run `clojure.test` namespaces (named on the CLI or auto-discovered)     |

### Per-subcommand flags

`run`, `repl`, `compile`, `test` accept:
- `--src-path <DIR>` — repeatable; directories searched by `require`
- `--gc-soft-limit-mb <MB>` — soft GC threshold
- `--gc-hard-limit-mb <MB>` — hard GC threshold

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
```

---

## Build features

| Feature             | Effect                                                                        |
|---------------------|-------------------------------------------------------------------------------|
| `no-gc` (default off) | Propagated to `cljrs-gc`/`cljrs-value`/`cljrs-eval`/`cljrs-compiler`/`cljrs-runtime`/`cljrs-stdlib`.  Disables the tracing GC; only region-allocated and stack values are permitted.  Compiles fail (`AotError::NoGcBlacklist`) if the program contains allocations the optimizer can't lift onto regions. |
| `enable-rustyline`  | Pulls in `rustyline` for a line-editing REPL.  Without it, `cljrs repl` falls back to a plain `BufRead` loop.                                                                                |

Build with e.g. `cargo build --release --features enable-rustyline,no-gc`.

---

## Implementation notes

- Argument parsing uses [Clap](https://docs.rs/clap) derive macros (`Parser`, `Subcommand`).
- The miette error hook is installed at startup so `CljxError` propagated to `main` renders with terminal-linked source snippets.
- A worker thread is spawned with the configured stack size to run the actual command; the main thread only handles signal/exit setup.
- The REPL prints results, paginates errors via `miette`, and persists multi-line input across blank prompts.
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
| `cljrs-logging` (workspace) | `--debug` / `--trace` / `-X` flag handling                        |
| `clap` (workspace)          | CLI argument parsing                                              |
| `miette` (workspace)        | Rich terminal error rendering                                     |
| `tracing` / `tracing-subscriber` | Structured logging output                                    |
| `rustyline` (workspace, optional) | Line-editing REPL when `enable-rustyline` is on              |
