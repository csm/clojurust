# clojurust

A Rust-hosted dialect of the Clojure programming language.

> **Early work in progress.** The lexer, parser, and core data types are
> implemented. The evaluator and everything above it is a stub. Expect breaking
> changes at every layer.

---

## What is this?

clojurust is an interpreter and compiler for a Clojure dialect that runs
natively on Rust. Source files use the `.cljx` extension (native) or `.cljc`
(cross-platform, with reader conditionals). The runtime platform key is
`:cljx`.

Planned capabilities:

- **Interpret** `.cljx` and `.cljc` source files
- **Rust interop** — call Rust functions from Clojure with defined type marshalling
- **Tracing GC** — all Clojure values managed by a garbage collector; Rust owns the root
- **JIT compilation** — hot code compiled to native via Cranelift
- **AOT compilation** — `cljx compile` produces a standalone binary

---

## Status

| Phase | Description | Status |
|-------|-------------|--------|
| 1 | Project infrastructure | complete |
| 2 | Lexer + parser (`Form` AST) | complete |
| 3 | Core data types & persistent collections | mostly complete |
| 4 | Evaluator & special forms | not started |
| 5 | Core standard library | not started |
| 6 | Protocols & multimethods | not started |
| 7 | Concurrency primitives | not started |
| 8 | Garbage collector | not started |
| 9 | Rust interop | not started |
| 10 | JIT compiler | not started |
| 11 | AOT compiler | not started |
| 12 | REPL & tooling | not started |

See [`TODO.md`](TODO.md) for the full itemised roadmap.

---

## Crates

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljx-types`](crates/cljx-types) | Shared foundational types: `Span`, `CljxError`, `CljxResult` | complete |
| [`cljx-reader`](crates/cljx-reader) | Lexer + recursive-descent parser; produces `Form` AST with source spans | complete |
| [`cljx-eval`](crates/cljx-eval) | Tree-walking interpreter; special forms; namespace/environment | stub |
| [`cljx-runtime`](crates/cljx-runtime) | Core standard library (`clojure.core` equivalent); concurrency primitives | stub |
| [`cljx-compiler`](crates/cljx-compiler) | IR lowering; JIT (Cranelift) and AOT code generation | stub |
| [`cljx-value`](crates/cljx-value) | `Value` enum; persistent collections (HAMT, list, vector, set, queue); Clojure-compatible hashing | complete |
| [`cljx-gc`](crates/cljx-gc) | Tracing garbage collector; safepoints; write barriers; `GcPtr<T>` (Arc shim until Phase 8) | stub |
| [`cljx-interop`](crates/cljx-interop) | Rust ↔ Clojure FFI; `#[cljx::export]` proc-macro; type marshalling | stub |
| [`cljx`](crates/cljx) | `cljx` CLI binary: `run`, `repl`, `compile`, `eval` subcommands | scaffold |

Each crate has its own `README.md` with purpose, status, file layout, and public API.

---

## Building

```bash
cargo build
cargo test
cargo clippy -- -D warnings
cargo fmt --check
```

The `cljx` binary is not yet functional — subcommand handlers are stubs.

---

## Repository layout

```
Cargo.toml              # workspace manifest
crates/
  cljx-types/           # foundational types (Phase 1)
  cljx-reader/          # lexer + parser (Phase 2)
  cljx-eval/            # evaluator stub (Phase 4)
  cljx-runtime/         # standard library stub (Phase 5)
  cljx-compiler/        # JIT/AOT stub (Phase 10/11)
  cljx-gc/              # GC stub (Phase 8)
  cljx-interop/         # FFI stub (Phase 9)
  cljx/                 # CLI binary (Phase 12)
tests/
  fixtures/             # .cljx / .cljc source files for integration tests
TODO.md                 # phased implementation roadmap
```
