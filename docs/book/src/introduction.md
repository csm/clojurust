# Introduction

clojurust is a Rust-hosted dialect of the Clojure programming language. It reads
and executes `.cljrs` and `.cljc` source files, provides an interactive REPL, and
can AOT-compile programs to standalone native binaries.

## Goals

- **Interpreter** — read and execute `.cljrs` (native) and `.cljc` (cross-platform) source files.
- **Reader conditionals** — `.cljc` files use `#?(:rust ... :clj ... :default ...)` to branch on platform; the platform key for clojurust is `:rust`.
- **Rust interop** — Clojure code can call Rust functions through a defined set of conventions and type-marshalling primitives.
- **Garbage collector** — a tracing GC manages all Clojure values; an optional region-based allocator is available for allocation-heavy code paths.
- **AOT compilation** — `cljrs compile` produces a standalone native binary via Cranelift, or a WebAssembly module with `--target wasm`. See the [WebAssembly](wasm/index.md) chapter.
- **Async & I/O** — `clojure.core.async` channels and non-blocking file I/O ship as optional crates layered over the interpreter. See the [Async & I/O](async-io/index.md) chapter.

## Source file extensions

| Extension | Meaning |
|---|---|
| `.cljrs` | Native clojurust source. Always evaluated under the `:rust` platform. |
| `.cljc` | Cross-platform source. Reader conditionals select the active branch; clojurust evaluates `:rust` branches. |

## Quick start

```
cljrs run hello.cljrs
cljrs repl
cljrs eval '(+ 1 2)'
cljrs compile app.cljrs -o app
cljrs test --src-path test
```

Detailed documentation for each subcommand is in the [CLI](cli/index.md) chapter.
