# clojurust

A Rust-hosted dialect of the Clojure programming language.

---

## What is this?

clojurust is an interpreter (and eventually compiler) for a Clojure dialect
that runs natively on Rust. Source files use the `.cljrs` extension (native) or
`.cljc` (cross-platform, with reader conditionals). The runtime platform key is
`:rust`.

Current capabilities:

- **Interpret** `.cljrs` and `.cljc` source files via `cljrs run <file>`
- **REPL** — interactive read-eval-print loop via `cljrs repl`
- **Eval** — evaluate expressions from the shell via `cljrs eval '<expr>'`
- **Test** — run clojure.test suites via `cljrs test --src-path <dir> <ns>`
- **Reader conditionals** — `.cljc` files use `#?(:rust ... :clj ... :default ...)`
- **Persistent collections** — HAMT-backed maps/sets, RRB vectors, sorted maps/sets (via rpds)
- **Tracing GC** — non-moving mark-and-sweep garbage collector with `GcPtr<T>`
- **Clojure-compatible hashing** — Murmur3-based hashing matching Clojure/JVM semantics
- **Lazy sequences** — lazy-seq, cons cells, and full lazy evaluation
- **Protocols & multimethods** — defprotocol, extend-type, defmulti, defmethod
- **Concurrency** — atom, volatile, delay, promise, future, agent (with send/await)
- **Dynamic variables** — binding, thread-local bindings, future conveyance
- **Namespaces** — require with :as/:refer, load-file, alias resolution
- **Metadata** — with-meta/meta on all collection types, preserved through assoc/conj/dissoc
- **Transducers** — map, filter, take, drop, partition-all, partition-by, distinct, dedupe, etc.
- **Rust interop** — NativeObject trait, FromValue/IntoValue marshalling, protocol dispatch
- **Standard library** — clojure.string, clojure.set, clojure.test, clojure.walk, clojure.edn, clojure.zip, clojure.data, clojure.template
- **IR acceleration** — core namespace functions are pre-lowered to IR at build time for faster dispatch

Planned:

- **JIT compilation** — hot code compiled to native via Cranelift
- **AOT compilation** — `cljrs compile` produces a standalone binary

---

## Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Project infrastructure | complete |
| 2 | Lexer + parser (`Form` AST) | complete |
| 3 | Core data types & persistent collections | complete |
| 4 | Evaluator & special forms | complete |
| 5 | Core standard library | complete |
| 6 | Protocols & multimethods | complete |
| 6-ext | defrecord, reify, built-in protocols | complete |
| 7 | Concurrency primitives | complete |
| 8 | Garbage collector | complete |
| 8-ext | Source-path management & require | complete |
| 8-ext-2 | Dynamic variables & binding | complete |
| 8-ext-3 | *ns*, namespace reflection, clojure.test | complete |
| 8-ext-4 | stdlib registry (clojure.string, clojure.set, ...) | complete |
| 8-ext-5 | &env/&form macros, resolve, reader-cond in require | complete |
| 8-ext-6 | clojure.template/are, BigDecimal/Ratio, #?@ splicing | complete |
| 8-ext-7 | rpds migration (persistent collections) | complete |
| 8-ext-8 | Conformance fixes (172 test files, 4248 assertions) | complete |
| 8-ext-9 | Transducers | complete |
| 9 | Rust interop (NativeObject, FromValue/IntoValue) | in progress |
| 9-ir | IR pipeline & prebuilt IR acceleration | complete |
| 10 | JIT compiler (Cranelift) | not started |
| 11 | AOT compiler | partial (codegen scaffolding) |
| 12 | REPL & tooling | partial (REPL works, tooling not started) |

**336 tests** across the workspace.

See [`TODO.md`](TODO.md) for the full itemised roadmap.

---

## Crates

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-types`](crates/cljrs-types) | Shared foundational types: `Span`, `CljxError`, `CljxResult` | complete |
| [`cljrs-reader`](crates/cljrs-reader) | Lexer + recursive-descent parser; produces `Form` AST with source spans | complete |
| [`cljrs-value`](crates/cljrs-value) | `Value` enum; persistent collections (rpds-backed); Clojure-compatible hashing | complete |
| [`cljrs-gc`](crates/cljrs-gc) | Non-moving mark-and-sweep GC; `GcPtr<T>` smart pointer; `Trace` trait | complete |
| [`cljrs-env`](crates/cljrs-env) | Shared runtime environment: `GlobalEnv`, `Env`, dynamic bindings, namespace loader, GC roots | complete |
| [`cljrs-builtins`](crates/cljrs-builtins) | Native Clojure core functions (~300 builtins), transients, regex, bitops | complete |
| [`cljrs-interp`](crates/cljrs-interp) | Tree-walking Clojure interpreter: eval, special forms, macros, destructuring | complete |
| [`cljrs-ir`](crates/cljrs-ir) | Intermediate representation types with serialization (postcard); `IrBundle` for prebuilt IR | complete |
| [`cljrs-eval`](crates/cljrs-eval) | IR-accelerated evaluation: IR interpreter, IR cache, lowering bridge, prebuilt IR loading | complete |
| [`cljrs-ir-prebuild`](crates/cljrs-ir-prebuild) | CLI tool to pre-lower Clojure namespaces to serialized IR bundles | complete |
| [`cljrs-stdlib`](crates/cljrs-stdlib) | Embedded stdlib: clojure.string, clojure.set, clojure.test, clojure.walk, clojure.edn, clojure.zip, clojure.data | complete |
| [`cljrs-logging`](crates/cljrs-logging) | Feature-gated logging (`-X debug:ir`, `-X trace:gc`, etc.) | complete |
| [`cljrs-compiler`](crates/cljrs-compiler) | IR lowering; JIT (Cranelift) and AOT code generation | partial |
| [`cljrs-interop`](crates/cljrs-interop) | Rust ↔ Clojure FFI; NativeObject, FromValue/IntoValue, type marshalling | in progress |
| [`cljrs-runtime`](crates/cljrs-runtime) | Runtime support | stub |
| [`cljrs`](crates/cljrs) | `cljrs` CLI binary: `run`, `repl`, `eval`, `test` subcommands (clap-based) | functional |

Each crate has its own `README.md` with purpose, status, file layout, and public API.

---

## Building

```bash
cargo build               # build all crates
cargo test                 # run all tests
cargo clippy -- -D warnings # lint
cargo fmt --check          # format check
```

## Usage

```bash
cljrs run <file.cljrs>           # interpret a source file
cljrs run --src-path lib/ <file>  # with additional source paths
cljrs repl                       # start interactive REPL
cljrs eval '(+ 1 2 3)'           # evaluate expression from shell
cljrs test --src-path test/ <ns>  # run clojure.test namespaces
```

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CLJRS_NO_IR` | unset | Disable all IR functionality. When set, the IR cache is not consulted and prebuilt IR is not loaded. All evaluation falls back to the tree-walking interpreter. Useful for debugging semantic differences between the IR interpreter and the tree-walker. |
| `CLJRS_EAGER_LOWER` | unset | Enable eager IR lowering at function definition time. Every `fn*` form triggers the Clojure compiler to lower the function body to IR immediately. This is expensive and primarily useful for testing the IR pipeline. Requires the compiler namespaces to be loaded. Has no effect when `CLJRS_NO_IR` is set. |

### Debug logging

Feature-level debug logging is available via the `-X` CLI flag:

```bash
cljrs -X debug:ir eval '(+ 1 2)'    # show IR loading/dispatch diagnostics
cljrs -X debug:gc eval '(range 100)' # show GC collection diagnostics
cljrs -X trace:reader eval '(+ 1 2)' # trace-level reader output
```

Format: `-X <level>:<feature1>,<feature2>,...` where level is `debug` or `trace`.

---

## Architecture

### Evaluation pipeline

```
Source code
    |
    v
  Reader (cljrs-reader)        lexer + parser -> Form AST
    |
    v
  Macroexpansion (cljrs-interp) expand macros, syntax-quote
    |
    v
  Interpreter (cljrs-interp)    tree-walking eval of special forms
    |
    v
  IR dispatch (cljrs-eval)      if IR is cached for a function arity,
    |                            execute via the IR interpreter instead
    v                            of tree-walking the body
  Result (Value)
```

### Prebuilt IR pipeline

At build time (`cljrs-stdlib/build.rs`), the tree-walking interpreter boots,
loads the Clojure compiler namespaces, and lowers all `clojure.core` function
arities to IR. The resulting `IrBundle` is serialized with postcard and embedded
via `include_bytes!`. At runtime, `standard_env()` deserializes the bundle and
populates the IR cache so core functions use the IR interpreter from the first
call — no compiler loading required.

```
Build time:                          Runtime:

build.rs                             standard_env()
  |                                    |
  v                                    v
tree-walk bootstrap         deserialize prebuilt IR bundle
  |                           (include_bytes!)
  v                                    |
load compiler namespaces               v
  |                          populate IR cache with
  v                          core fn arities
lower all core fn arities              |
  |                                    v
  v                          user code: IR cache hits
serialize to core_ir.bin     for core fns, tree-walk
                             for user fns
```

### Dependency graph

```
cljrs-types
    |
cljrs-gc -----------> cljrs-types
    |
cljrs-reader -------> cljrs-types
    |
cljrs-value ---------> cljrs-gc, cljrs-reader, cljrs-types
    |
cljrs-ir ------------> cljrs-types
    |
cljrs-env -----------> cljrs-value, cljrs-gc, cljrs-reader
    |
cljrs-builtins ------> cljrs-env, cljrs-value, cljrs-gc
    |
cljrs-interp --------> cljrs-builtins, cljrs-env, cljrs-value, cljrs-gc
    |
cljrs-eval ----------> cljrs-interp, cljrs-env, cljrs-ir, cljrs-value
    |
cljrs-stdlib --------> cljrs-eval, cljrs-interp, cljrs-ir
    |
cljrs-compiler ------> cljrs-eval, cljrs-ir, cljrs-stdlib (Cranelift)
    |
cljrs (binary) ------> cljrs-stdlib, cljrs-compiler
```

---

## Repository layout

```
Cargo.toml              # workspace manifest (resolver=2)
crates/
  cljrs-types/           # foundational types
  cljrs-reader/          # lexer + parser
  cljrs-value/           # Value enum, collections, types
  cljrs-gc/              # tracing GC
  cljrs-env/             # runtime environment, dynamic bindings, loader
  cljrs-builtins/        # native Clojure core functions
  cljrs-interp/          # tree-walking interpreter
  cljrs-ir/              # intermediate representation + serialization
  cljrs-eval/            # IR-accelerated evaluation
  cljrs-ir-prebuild/     # CLI tool for pre-lowering IR
  cljrs-stdlib/          # embedded standard library namespaces
  cljrs-logging/         # feature-gated debug/trace logging
  cljrs-compiler/        # JIT/AOT code generation (Cranelift)
  cljrs-interop/         # Rust <-> Clojure FFI
  cljrs-runtime/         # runtime support (stub)
  cljrs/                 # CLI binary
tests/
  fixtures/             # .cljrs / .cljc source files for integration tests
TODO.md                 # phased implementation roadmap
```
