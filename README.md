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
| [`cljrs-types`](crates/cljrs-types) | Shared foundational types: `Span`, `CljxError`, `CljxResult` | complete |
| [`cljrs-reader`](crates/cljrs-reader) | Lexer + recursive-descent parser; produces `Form` AST with source spans | complete |
| [`cljrs-value`](crates/cljrs-value) | `Value` enum; persistent collections (rpds-backed maps, sets, vectors, sorted variants); Clojure-compatible hashing | complete |
| [`cljrs-gc`](crates/cljrs-gc) | Non-moving mark-and-sweep GC; `GcPtr<T>` smart pointer; `Trace` trait | complete |
| [`cljrs-eval`](crates/cljrs-eval) | Tree-walking interpreter; special forms; macros; namespace/environment; destructuring; dynamic vars | complete |
| [`cljrs-stdlib`](crates/cljrs-stdlib) | Embedded stdlib: clojure.string, clojure.set, clojure.test (native + Clojure source) | complete |
| [`cljrs-runtime`](crates/cljrs-runtime) | Runtime support (Phase 6+) | stub |
| [`cljrs-compiler`](crates/cljrs-compiler) | IR lowering; JIT (Cranelift) and AOT code generation | stub |
| [`cljrs-interop`](crates/cljrs-interop) | Rust ↔ Clojure FFI; type marshalling | stub |
| [`cljrs`](crates/cljrs) | `cljrs` CLI binary: `run`, `repl`, `eval` subcommands (clap-based) | functional |

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
cljrs run <file.cljrs>           # interpret a source file
cljrs run --src-path lib/ <file>  # with additional source paths
cljrs repl                       # start interactive REPL
cljrs eval '(+ 1 2 3)'           # evaluate expression from shell
cljrs test --src-path ...        # run clojure.test test cases from src-path
```

---

## Repository layout

```
Cargo.toml              # workspace manifest (resolver=2)
crates/
  cljrs-types/           # foundational types (Phase 1)
  cljrs-reader/          # lexer + parser (Phase 2)
  cljrs-value/           # Value enum, collections, types (Phase 3)
  cljrs-gc/              # tracing GC (Phase 8)
  cljrs-eval/            # evaluator, builtins, macros (Phase 4-5)
  cljrs-stdlib/          # embedded stdlib (Phase 8-ext-4)
  cljrs-runtime/         # runtime stub (Phase 6+)
  cljrs-compiler/        # JIT/AOT stub (Phase 10/11)
  cljrs-interop/         # FFI stub (Phase 9)
  cljrs/                 # CLI binary (Phase 12)
tests/
  fixtures/             # .cljrs / .cljc source files for integration tests
TODO.md                 # phased implementation roadmap
```
