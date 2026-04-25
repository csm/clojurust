# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`clojurust` is a Rust-hosted dialect of the Clojure programming language. Goals:

- **Interpreter**: read and execute `.cljrs` (native extension) and `.cljc` (cross-platform) source files
- **Reader conditionals**: `.cljc` files use `#?(:rust ... :clj ... :cljs ... :default ...)` — the platform key for this runtime is `:rust`
- **Rust interop**: Clojure code can call into Rust functions with defined conventions and type-marshalling
- **Garbage collector**: a tracing GC manages all Clojure values; Rust owns the GC root
- **AOT compilation**: `cljrs compile` produces a standalone native binary

See `TODO.md` for the full phased implementation roadmap.

## Crate READMEs

Every crate in `crates/` must have a `README.md` that documents:

- **Purpose** — one-sentence summary of what the crate does
- **Status** — which phase it belongs to and whether it is implemented or a stub
- **File layout** — every source file listed with a one-line description
- **Public API** — all public types, functions, and trait impls; include signatures for non-obvious items

**Keep READMEs current.** Whenever you add, remove, or rename a public type, function, module, or source file in a crate, update that crate's `README.md` in the same commit. A stale README is worse than no README: it actively misleads readers trying to trace a bug or understand a design decision.

## Commands

```bash
# Build
cargo build

# Run tests
cargo test

# Run a single test by name
cargo test <test_name>

# Check for errors without building
cargo check

# Lint
cargo clippy

# Format
cargo fmt
```

Once the CLI exists:
```bash
cljrs run <file.cljrs>      # interpret a source file
cljrs repl                   # start interactive REPL
cljrs compile <file> -o <bin> # AOT compile to binary
cljrs eval '<expr>'          # evaluate expression from shell
cljrs test --src-path ...  # run clojure.test namespaces
```

## Tooling

Use LSP whenever possible to navigate the code base.

## Architecture

The project is a library crate (`src/lib.rs`) with a binary entry point (`src/main.rs`, to be added). Expected module breakdown:

| Module | Responsibility |
|---|---|
| `reader` | Lexer + parser; produces `Form` AST with source spans; handles reader conditionals |
| `types` | `Value` enum (all Clojure runtime types); persistent collections (HAMT-backed); GC smart pointer `GcPtr<T>` |
| `gc` | Tracing garbage collector; safepoints; write barriers; weak refs |
| `eval` | Tree-walking interpreter; special forms; macro expansion; namespace/environment |
| `compiler` | IR lowering; AOT code-gen; inline caches |
| `runtime` | Core standard library (`clojure.core` equivalent); concurrency primitives (atom, ref/STM, agent, future) |
| `interop` | Rust↔Clojure FFI; `#[cljx::export]` proc-macro; type marshalling; `NativeObject` |
| `cli` | `cljx` command entry point; REPL; file runner; project tooling |

### Key design constraints

- **All Clojure values live behind `GcPtr<Value>`** — never store `Value` directly on the Rust stack across a GC safepoint
- **Persistent collections are the default** — mutability only via `atom`/`ref`/`agent` or transients
- **Rust interop is safe-by-default** — unsafe Rust APIs accessible only through an explicit `cljx.rust/unsafe` boundary
- **Reader is platform-agnostic** — it parses all branches of `#?(...)` and returns them; the evaluator filters by `:rust`
