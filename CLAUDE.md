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

## Memory Management

The runtime uses a tracing GC as its primary allocator, with an optional bump allocator that can bypass the GC for values proven not to escape their call frame.

### Tracing GC

All Clojure values are heap-allocated behind `GcPtr<T>`. The GC is a stop-the-world tracing collector (`crates/cljrs-gc/src/lib.rs`). `GcPtr::clone` is O(1) and `GcPtr::drop` is a no-op — the collector reclaims unreachable objects during a collection pause. Collection triggers are threshold-based: a soft limit (75% of the hard limit) and a hard limit (defaulting to ¼ of available RAM, minimum 256 MB).

### Bump allocator

The bump allocator is implemented in `crates/cljrs-gc/src/region.rs` as the `Region` struct. It is available alongside the GC — allocations that are proven non-escaping skip the GC heap entirely and land in a scratch region instead. It works by maintaining a pointer into a contiguous memory chunk and advancing ("bumping") that pointer on each allocation — no per-object bookkeeping, no mutex contention. When the current chunk is exhausted a new chunk is appended; the default initial chunk size is 4 KiB. Resetting a region (e.g. at the end of a function call) runs destructors in LIFO order and then reclaims all chunks in one shot.

There are two region flavours:

| Type | Location | Lifetime | Use |
|---|---|---|---|
| `Region` / `ScratchGuard` | `crates/cljrs-gc/src/region.rs` + `alloc_ctx.rs` | Scoped to a call frame | Intermediate values that don't escape |
| `StaticArena` | `crates/cljrs-gc/src/static_arena.rs` | Program lifetime | Interned symbols, compiled code, constants |

The **allocation context stack** (`crates/cljrs-gc/src/alloc_ctx.rs`) routes each allocation to the currently-active context. The compiler inserts `RegionStart`/`RegionEnd` IR nodes (see `crates/cljrs-ir/src/lower/regionalize.rs`) around call sites whose return values are proven non-escaping, so intermediate allocations land in the scratch region rather than the GC heap. In AOT mode (`no-gc` feature, activated by `cljrs compile`) the GC is disabled entirely and all allocations go through regions or the static arena.

The **return-expression protocol** ensures the tail value of a function body is allocated in the *caller's* context, not the callee's scratch region:

1. `ScratchGuard` is pushed — body allocations enter the scratch region.
2. Non-tail sub-expressions are evaluated; intermediates land in scratch.
3. `pop_for_return()` removes scratch from the active context stack.
4. The tail expression is evaluated — its result lands in the caller's context.
5. `ScratchGuard` drops — scratch memory is reset and all intermediates are freed.

This protocol is verified by the integration tests in `crates/cljrs-gc/tests/no_gc_alloc.rs`.
