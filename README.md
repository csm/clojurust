# clojurust

A Rust-hosted dialect of the Clojure programming language.

---

## What is this?

clojurust is an interpreter (and eventually compiler) for a Clojure dialect
that runs natively on Rust. Source files use the `.cljrs` extension (native) or
`.cljc` (cross-platform, with reader conditionals). The runtime platform key is
`:rust`.

Current capabilities:

- **Interpret** `.cljrs` and `.cljc` source files via `cljx run <file>`
- **REPL** — interactive read-eval-print loop via `cljx repl`
- **Eval** — evaluate expressions from the shell via `cljx eval '<expr>'`
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
- **Standard library** — clojure.string, clojure.set, clojure.test

Planned:

- **Rust interop** — call Rust functions from Clojure with defined type marshalling
- **JIT compilation** — hot code compiled to native via Cranelift
- **AOT compilation** — `cljx compile` produces a standalone binary

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
| 8-ext-4 | stdlib registry (clojure.string, clojure.set) | complete |
| 8-ext-5 | &env/&form macros, resolve, reader-cond in require | complete |
| 8-ext-6 | clojure.template/are, BigDecimal/Ratio, #?@ splicing | complete |
| 9 | Rust interop | not started |
| 10 | JIT compiler | not started |
| 11 | AOT compiler | not started |
| 12 | REPL & tooling | partial (REPL works, tooling not started) |

**288 tests** across the workspace (121 eval, 82 value-collections, 65 value-other, 13 stdlib, 7 GC).

See [`TODO.md`](TODO.md) for the full itemised roadmap.

---

## Crates

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljx-types`](crates/cljx-types) | Shared foundational types: `Span`, `CljxError`, `CljxResult` | complete |
| [`cljx-reader`](crates/cljx-reader) | Lexer + recursive-descent parser; produces `Form` AST with source spans | complete |
| [`cljx-value`](crates/cljx-value) | `Value` enum; persistent collections (rpds-backed maps, sets, vectors, sorted variants); Clojure-compatible hashing | complete |
| [`cljx-gc`](crates/cljx-gc) | Non-moving mark-and-sweep GC; `GcPtr<T>` smart pointer; `Trace` trait | complete |
| [`cljx-eval`](crates/cljx-eval) | Tree-walking interpreter; special forms; macros; namespace/environment; destructuring; dynamic vars | complete |
| [`cljx-stdlib`](crates/cljx-stdlib) | Embedded stdlib: clojure.string, clojure.set, clojure.test (native + Clojure source) | complete |
| [`cljx-runtime`](crates/cljx-runtime) | Runtime support (Phase 6+) | stub |
| [`cljx-compiler`](crates/cljx-compiler) | IR lowering; JIT (Cranelift) and AOT code generation | stub |
| [`cljx-interop`](crates/cljx-interop) | Rust ↔ Clojure FFI; type marshalling | stub |
| [`cljx`](crates/cljx) | `cljx` CLI binary: `run`, `repl`, `eval` subcommands (clap-based) | functional |

Each crate has its own `README.md` with purpose, status, file layout, and public API.

---

## Building

```bash
cargo build               # build all crates
cargo test                 # run all 288 tests
cargo clippy -- -D warnings # lint
cargo fmt --check          # format check
```

## Usage

```bash
cljx run <file.cljrs>           # interpret a source file
cljx run --src-path lib/ <file>  # with additional source paths
cljx repl                       # start interactive REPL
cljx eval '(+ 1 2 3)'           # evaluate expression from shell
```

---

## Repository layout

```
Cargo.toml              # workspace manifest (resolver=2)
crates/
  cljx-types/           # foundational types (Phase 1)
  cljx-reader/          # lexer + parser (Phase 2)
  cljx-value/           # Value enum, collections, types (Phase 3)
  cljx-gc/              # tracing GC (Phase 8)
  cljx-eval/            # evaluator, builtins, macros (Phase 4-5)
  cljx-stdlib/          # embedded stdlib (Phase 8-ext-4)
  cljx-runtime/         # runtime stub (Phase 6+)
  cljx-compiler/        # JIT/AOT stub (Phase 10/11)
  cljx-interop/         # FFI stub (Phase 9)
  cljx/                 # CLI binary (Phase 12)
tests/
  fixtures/             # .cljrs / .cljc source files for integration tests
TODO.md                 # phased implementation roadmap
```
