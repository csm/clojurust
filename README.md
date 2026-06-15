# clojurust

[![crates.io](https://img.shields.io/crates/v/cljrs.svg)](https://crates.io/crates/cljrs)
[![docs](https://img.shields.io/badge/docs-docs.clj.rs-blue)](https://docs.clj.rs)
[![main](https://img.shields.io/badge/main-clj.rs-yellow)](https://clj.rs)

A Rust-hosted dialect of the Clojure programming language.

---

## What is this?

clojurust runs a Clojure dialect natively on Rust. Source files use the
`.cljrs` extension (native) or `.cljc` (cross-platform, with reader
conditionals). The runtime platform key is `:rust`.

A program flows through a tiered execution pipeline: the reader and
tree-walking interpreter give immediate startup, hot functions are lowered to
an SSA/A-normal-form IR and interpreted faster, and the hottest arities are
**JIT-compiled to native code via Cranelift** while the program runs. The same
backend powers **ahead-of-time compilation** to a standalone binary.

Current capabilities:

- **Interpret** `.cljrs` and `.cljc` source files via `cljrs run <file>`
- **REPL** — interactive read-eval-print loop via `cljrs repl`
- **Eval** — evaluate expressions from the shell via `cljrs eval '<expr>'`
- **Test** — run clojure.test suites via `cljrs test --src-path <dir> <ns>`
- **JIT compilation** — hot function arities are compiled to native code via
  Cranelift while the program runs, with type specialization, inline caches,
  on-stack replacement (OSR) of hot loops, code unloading on redefinition, and
  deoptimization back to the interpreter when type assumptions break
- **AOT compilation** — `cljrs compile <file> -o <bin>` produces a standalone
  native binary; end-to-end for multi-file programs (variadic fns, protocols,
  escape-analysis region allocation, HOFs, sequence/collection ops)
- **Reader conditionals** — `.cljc` files use `#?(:rust ... :clj ... :default ...)`
- **Persistent collections** — HAMT-backed maps/sets, RRB vectors, sorted maps/sets (via rpds)
- **Tracing GC** — non-moving mark-and-sweep garbage collector with `GcPtr<T>`,
  conservative scanning of JIT frames, and escape-analysis scratch regions
- **Clojure-compatible hashing** — Murmur3-based hashing matching Clojure/JVM semantics
- **Lazy sequences** — lazy-seq, cons cells, and full lazy evaluation
- **Protocols & multimethods** — defprotocol, extend-type, defmulti, defmethod, defrecord, reify
- **Concurrency** — atom, volatile, delay, promise, future, agent (with send/await)
- **core.async** — `go`/`chan`/`<!`/`>!`/`alts!`/`timeout` via a Tokio executor (`cljrs-async`)
- **Dynamic variables** — binding, thread-local bindings, future conveyance
- **Namespaces** — require with :as/:refer, load-file, alias resolution
- **Metadata** — with-meta/meta on all collection types, preserved through assoc/conj/dissoc
- **Transducers** — map, filter, take, drop, partition-all, partition-by, distinct, dedupe, etc.
- **Rust interop** — `#[cljrs_interop::export]` proc-macro, NativeObject, FromValue/IntoValue
  marshalling, protocol dispatch, and dynamic loading of native `.so`/`.dylib`
- **Standard library** — clojure.string, clojure.set, clojure.test, clojure.walk, clojure.edn, clojure.zip, clojure.data, clojure.template
- **IR acceleration** — core namespace functions are pre-lowered to IR at build time for faster dispatch

Tooling:

- **LSP server** — `cljrs lsp`: parse diagnostics + document-symbol outline (`cljrs-lsp`)
- **nREPL server** — `cljrs nrepl`: bencode-over-TCP for CIDER/Calva/Conjure (`cljrs-nrepl`)
- **IR visualizer** — `cljrs ir-viz`: HTML view of optimized IR + region allocation (`cljrs-ir-viz`)
- **Dependencies** — `cljrs deps fetch/status`: git-hosted deps from `cljrs.edn` (`cljrs-deps`, `cljrs-vcs`)
- **WASM REPL** — browser REPL compiled to `wasm32-unknown-unknown` (`cljrs-wasm`)

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
| 8.1 | Program optimization (ANF/SSA, escape analysis, regions) | complete |
| 8-ext-* | require, dynamic vars, *ns*, stdlib registry, transducers, … | complete |
| 9 | Rust interop (NativeObject, FromValue/IntoValue, `#[export]`, dylib) | mostly complete |
| 9-ir | IR pipeline & prebuilt IR acceleration | complete |
| 10 | JIT compiler (Cranelift) | working |
| 11 | AOT compiler | working (end-to-end for multi-file programs) |
| 12 | REPL & tooling (REPL, LSP, nREPL) | working |
| async | core.async, async I/O, networking, charset | implemented |

**880+ tests** across the workspace.

See [`TODO.md`](TODO.md) for the full itemised roadmap.

---

## Crates

### Core pipeline

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-types`](crates/cljrs-types) | Shared foundational types: `Span`, `CljxError`, `CljxResult` | complete |
| [`cljrs-reader`](crates/cljrs-reader) | Lexer + recursive-descent parser; produces `Form` AST with source spans | complete |
| [`cljrs-value`](crates/cljrs-value) | `Value` enum; persistent collections (rpds-backed); Clojure-compatible hashing | complete |
| [`cljrs-gc`](crates/cljrs-gc) | Non-moving mark-and-sweep GC; `GcPtr<T>` smart pointer; `Trace` trait; scratch regions | complete |
| [`cljrs-env`](crates/cljrs-env) | Shared runtime environment: `GlobalEnv`, `Env`, dynamic bindings, namespace loader, GC roots | complete |
| [`cljrs-builtins`](crates/cljrs-builtins) | Native Clojure core functions (~300 builtins), transients, regex, bitops | complete |
| [`cljrs-interp`](crates/cljrs-interp) | Tree-walking Clojure interpreter: eval, special forms, macros, destructuring | complete |
| [`cljrs-ir`](crates/cljrs-ir) | IR types (ANF/SSA) with serialization (postcard); ANF lowering, escape analysis, OSR | complete |
| [`cljrs-eval`](crates/cljrs-eval) | IR-accelerated evaluation: IR interpreter, IR cache, tiering/JIT state, lower worker, prebuilt IR loading | complete |
| [`cljrs-ir-prebuild`](crates/cljrs-ir-prebuild) | CLI tool to pre-lower Clojure namespaces to serialized IR bundles | complete |
| [`cljrs-stdlib`](crates/cljrs-stdlib) | Embedded stdlib: clojure.string, clojure.set, clojure.test, clojure.walk, clojure.edn, clojure.zip, clojure.data | complete |
| [`cljrs-logging`](crates/cljrs-logging) | Feature-gated logging (`-X debug:ir`, `-X trace:gc`, etc.) | complete |
| [`cljrs-runtime`](crates/cljrs-runtime) | Runtime support placeholder | stub |

### Compilation

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-compiler`](crates/cljrs-compiler) | Cranelift codegen (generic over `Module`), type inference, AOT object/binary emission, C-ABI runtime bridge | working |
| [`cljrs-jit`](crates/cljrs-jit) | In-process JIT: hot-arity native compilation, type specialization + inline caches, OSR, code unloading, region threading | working |
| [`cljrs-ir-viz`](crates/cljrs-ir-viz) | HTML visualizer for optimized IR + region allocation (`cljrs ir-viz`) | implemented |

### Interop

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-interop`](crates/cljrs-interop) | Rust ↔ Clojure FFI: NativeObject, FromValue/IntoValue, error bridging, Registry | mostly complete |
| [`cljrs-export-macro`](crates/cljrs-export-macro) | Proc-macro backing `#[cljrs_interop::export]` (re-exported by `cljrs-interop`) | complete |
| [`cljrs-dylib`](crates/cljrs-dylib) | Pinned native packages: build a dep's crate at a git commit as a cdylib and load it | experimental |
| [`cljrs-base64`](crates/cljrs-base64) | Base64 encode/decode exposed as Clojure native functions (interop example) | implemented |
| [`cljrs-blake3`](crates/cljrs-blake3) | BLAKE3 hashing exposed as Clojure native functions (interop example) | implemented |

### Async, I/O & networking

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-async`](crates/cljrs-async) | `clojure.core.async` via a Tokio `current_thread` + `LocalSet` executor; per-isolate heaps | implemented |
| [`cljrs-io`](crates/cljrs-io) | Non-blocking file I/O delivered over core.async channels | implemented |
| [`cljrs-net`](crates/cljrs-net) | TCP/UDP/Unix/TLS sockets as core.async channels | implemented |
| [`cljrs-charset`](crates/cljrs-charset) | Charset encode/decode with stream support (`encoding_rs`) | implemented |

### Project & tooling

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs-deps`](crates/cljrs-deps) | Parses `cljrs.edn` project config (`DepsConfig`); git/local/Rust deps | implemented |
| [`cljrs-vcs`](crates/cljrs-vcs) | Thin `git` CLI wrapper for versioned symbol resolution + dep cache | implemented |
| [`cljrs-lsp`](crates/cljrs-lsp) | LSP server (`cljrs lsp`): parse diagnostics + document symbols (`tower-lsp`) | implemented (syntactic) |
| [`cljrs-nrepl`](crates/cljrs-nrepl) | nREPL server (`cljrs nrepl`): bencode over TCP for CIDER/Calva/Conjure | implemented |
| [`cljrs-wasm`](crates/cljrs-wasm) | Browser REPL compiled to `wasm32-unknown-unknown` (wasm-bindgen) | implemented |

### Binary

| Crate | Description | Status |
|-------|-------------|--------|
| [`cljrs`](crates/cljrs) | `cljrs` CLI: `run`, `repl`, `eval`, `test`, `compile`, `ir-viz`, `deps`, `lsp`, `nrepl`, `build-native` (clap-based) | functional |

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
cljrs run <file.cljrs>            # interpret a source file (JIT-accelerated)
cljrs run --src-path lib/ <file>   # with additional source paths
cljrs repl                        # start interactive REPL
cljrs eval '(+ 1 2 3)'            # evaluate expression from shell
cljrs test --src-path test/ <ns>   # run clojure.test namespaces
cljrs compile app.cljrs -o app     # AOT-compile to a standalone binary
cljrs ir-viz <file> -o ir.html     # render optimized IR + source to HTML
cljrs deps fetch                  # fetch git deps declared in cljrs.edn
cljrs lsp                         # start an LSP server (stdio)
cljrs nrepl --port 7888           # start an nREPL server
```

The JIT runs by default. `--jit-threshold N` sets the per-arity invocation
count before native compilation (default 1000; `0` disables the JIT, falling
back to the IR interpreter and tree-walker). It can also be set via
`CLJRS_JIT_THRESHOLD`.

---

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CLJRS_NO_IR` | unset | Disable all IR functionality. The IR cache is not consulted and prebuilt IR is not loaded; all evaluation falls back to the tree-walking interpreter. Useful for debugging semantic differences between the IR interpreter and the tree-walker. |
| `CLJRS_EAGER_LOWER` | unset | Lower every `fn*` body to IR at definition time instead of lazily after warm-up. Expensive; primarily for testing the IR pipeline. No effect when `CLJRS_NO_IR` is set. |
| `CLJRS_IR_THRESHOLD` | 50 | Tier-0 → Tier-1 warm threshold: tree-walked calls per arity before the body is lowered to IR (`0` disables). |
| `CLJRS_JIT_THRESHOLD` | 1000 | Tier-1 → JIT-native threshold: IR-interpreted calls per arity before native compilation (`0` disables the JIT). |
| `CLJRS_OSR_THRESHOLD` | (JIT threshold) | Loop back-edge count within a single call before on-stack replacement promotes the loop to native code. |
| `CLJRS_JIT_NO_SPEC` | unset | Disable type specialization; the JIT compiles generic boxed entries only. |
| `CLJRS_JIT_DEOPT_LIMIT` | 10 | Deopt failures per arity before its specialized code is unpublished and the arity is banned from re-specialization. |
| `CLJRS_IR_CACHE_TTL` | 600 (s) | Idle time before a cold IR cache entry is evicted at the stop-the-world reclaim pass. |

### Debug logging

Feature-level debug logging is available via the `-X` CLI flag:

```bash
cljrs -X debug:ir eval '(+ 1 2)'    # show IR loading/dispatch diagnostics
cljrs -X debug:jit eval '(+ 1 2)'   # show JIT compilation/dispatch diagnostics
cljrs -X debug:gc eval '(range 100)' # show GC collection diagnostics
cljrs -X trace:reader eval '(+ 1 2)' # trace-level reader output
```

Format: `-X <level>:<feature1>,<feature2>,...` where level is `debug` or `trace`.
Use `--jit-stats <path>` to dump JIT specialization / inline-cache / deopt
counters on exit.

---

## Architecture

### Execution pipeline

```
Source code
    |
    v
  Reader (cljrs-reader)         lexer + parser -> Form AST
    |
    v
  Macroexpansion (cljrs-interp) expand macros, syntax-quote
    |
    v
  Tier 0: tree-walk (cljrs-interp)   immediate execution; counts calls per arity
    |
    v
  Tier 1: IR interpreter (cljrs-eval) hot arities lowered to ANF/SSA IR
    |                                  (background lower worker), interpreted
    v                                  faster; OSR counters on hot loops
  Tier 2: JIT native (cljrs-jit)     hottest arities compiled to native code
    |                                  via Cranelift: type specialization,
    v                                  inline caches, OSR, region threading;
  Result (Value)                      deopt back to Tier 1 on guard failure
```

The same Cranelift backend (`cljrs-compiler`, generic over
`cranelift_module::Module`) drives both the in-process `JITModule` and the
AOT `ObjectModule`, so `cljrs compile` reuses the JIT's codegen.

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

### Dependency graph (core path)

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
cljrs-compiler ------> cljrs-eval, cljrs-ir, cljrs-stdlib (Cranelift AOT)
    |
cljrs-jit -----------> cljrs-compiler, cljrs-eval, cljrs-ir (Cranelift JIT)
    |
cljrs (binary) ------> cljrs-stdlib, cljrs-compiler, cljrs-jit, cljrs-lsp,
                       cljrs-nrepl, cljrs-deps, cljrs-vcs, cljrs-ir-viz,
                       cljrs-interop  (+ async/net/charset behind features)
```

---

## Repository layout

```
Cargo.toml              # workspace manifest (resolver=2)
crates/
  # core pipeline
  cljrs-types/           # foundational types
  cljrs-reader/          # lexer + parser
  cljrs-value/           # Value enum, collections, hashing
  cljrs-gc/              # tracing GC + scratch regions
  cljrs-env/             # runtime environment, dynamic bindings, loader
  cljrs-builtins/        # native Clojure core functions
  cljrs-interp/          # tree-walking interpreter
  cljrs-ir/              # IR types + lowering + serialization
  cljrs-eval/            # IR-accelerated evaluation + tiering state
  cljrs-ir-prebuild/     # CLI tool for pre-lowering IR
  cljrs-stdlib/          # embedded standard library namespaces
  cljrs-logging/         # feature-gated debug/trace logging
  cljrs-runtime/         # runtime support (stub)
  # compilation
  cljrs-compiler/        # Cranelift codegen + AOT
  cljrs-jit/             # in-process JIT
  cljrs-ir-viz/          # IR HTML visualizer
  # interop
  cljrs-interop/         # Rust <-> Clojure FFI
  cljrs-export-macro/    # #[export] proc-macro
  cljrs-dylib/           # pinned native packages
  cljrs-base64/          # base64 interop library
  cljrs-blake3/          # BLAKE3 interop library
  # async, I/O & networking
  cljrs-async/           # clojure.core.async
  cljrs-io/              # async file I/O
  cljrs-net/             # TCP/UDP/Unix/TLS sockets
  cljrs-charset/         # charset encode/decode
  # project & tooling
  cljrs-deps/            # cljrs.edn project config
  cljrs-vcs/             # git wrapper for versioned deps
  cljrs-lsp/             # LSP server
  cljrs-nrepl/           # nREPL server
  cljrs-wasm/            # browser REPL (wasm)
  # binary
  cljrs/                 # CLI binary
examples/
  rust-interop/         # Rust interop example
tests/
  fixtures/             # .cljrs / .cljc source files for integration tests
TODO.md                 # phased implementation roadmap
```
