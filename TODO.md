# clojurust TODO

Implementation roadmap for a Rust-hosted Clojure dialect. Native file extension is `.cljx`; also supports `.cljc` with reader conditional `:cljx`.

---

## Phase 1 — Project Infrastructure

- [x] Define crate structure: separate crates under `crates/` for `cljx-types`, `cljx-gc`, `cljx-reader`, `cljx-eval`, `cljx-compiler`, `cljx-runtime`, `cljx-interop`, `cljx` (bin)
- [x] Set up binary crate (`crates/cljx/src/main.rs`) for the `cljx` CLI (run, repl, compile, eval)
- [x] Configure workspace `Cargo.toml` with `[workspace.dependencies]` for shared dep versions
- [x] Add CI (GitHub Actions) — `.github/workflows/ci.yml` — fmt, clippy -D warnings, test
- [x] Test harness skeleton: `crates/cljx/tests/integration.rs` with `#[ignore]`d fixture runner; fixtures dir at `tests/fixtures/`
- [x] Establish error-reporting infrastructure: `cljx-types` with `Span` (miette `SourceSpan` conversion) and `CljxError` (miette + thiserror diagnostics)

---

## Phase 2 — Reader

- [ ] Lexer: tokenize all Clojure lexical forms (symbols, keywords, numbers, strings, characters, booleans, nil)
- [ ] Reader: parse tokens into `Form` AST nodes
  - [ ] List `(...)`, vector `[...]`, map `{...}`, set `#{...}`
  - [ ] Quote `'`, quasiquote `` ` ``, unquote `~`, unquote-splicing `~@`
  - [ ] Metadata `^`, deref `@`, var `#'`
  - [ ] Anonymous function `#(...)`, regex literal `#"..."`
  - [ ] Symbolic values `##Inf`, `##-Inf`, `##NaN`
  - [ ] Tagged literals `#tag value`
- [ ] Numeric literals: long, double, ratio (`3/4`), arbitrary-precision `N`/`M` suffixes, radix literals (`2r1010`, `16rFF`)
- [ ] String escape sequences and multiline strings
- [ ] Character literals (`\a`, `\newline`, `\u0041`, etc.)
- [ ] Line/column source-location tracking on all forms
- [ ] Reader conditionals (`.cljc` / `.cljx`)
  - [ ] `#?(:cljx ... :clj ... :cljs ... :default ...)` splicing and non-splicing forms
  - [ ] Platform key `:cljx` selects Rust-dialect branch
- [ ] File extension dispatch: treat `.cljx` as always-Rust-dialect, `.cljc` as cross-platform with conditionals

---

## Phase 3 — Core Data Types & Persistent Collections

- [ ] `Value` enum: Nil, Bool, Long, Double, BigInt, BigDecimal, Ratio, Char, String, Symbol, Keyword, List, Vector, HashMap, HashSet, Fn, Macro, Var, Atom, Ref, Agent, Namespace, NativeFunction, …
- [ ] Persistent (immutable, structurally shared) collections
  - [ ] PersistentList (linked list with O(1) cons)
  - [ ] PersistentVector (HAMT/RRB-tree)
  - [ ] PersistentHashMap (HAMT)
  - [ ] PersistentHashSet (backed by PersistentHashMap)
  - [ ] PersistentArrayMap (small maps, ≤8 entries)
  - [ ] PersistentQueue
- [ ] Metadata support on collections, symbols, and vars (`with-meta`, `meta`)
- [ ] Seq abstraction over all collections + lazy sequences
- [ ] Transient collections for efficient batch mutations
- [ ] Equality and hashing consistent with Clojure semantics

---

## Phase 4 — Evaluator & Special Forms

- [ ] Environment: lexical scopes + namespace-level vars
- [ ] Namespace system (`ns`, `in-ns`, `require`, `use`, `alias`, `refer`)
- [ ] Special forms
  - [ ] `def`, `defn`, `defmacro`
  - [ ] `let`, `letfn`, `loop`/`recur`
  - [ ] `fn` (multiple arities, rest args, destructuring)
  - [ ] `if`, `do`, `quote`, `var`
  - [ ] `try`/`catch`/`finally`, `throw`
  - [ ] `set!` for mutable vars / atoms
  - [ ] `monitor-enter`/`monitor-exit` (or Rust mutex equivalent)
  - [ ] `new` / interop forms (see Rust Interop phase)
- [ ] Destructuring in `let`, `fn`, `loop` (sequential, associative, nested)
- [ ] Tail-call optimization via `recur`
- [ ] Macro expansion pipeline (`macroexpand-1`, `macroexpand`, `macroexpand-all`)
- [ ] Syntax-quote with symbol resolution and gensyms

---

## Phase 5 — Core Standard Library (`clojure.core` equivalent)

- [ ] Arithmetic: `+`, `-`, `*`, `/`, `mod`, `rem`, `quot`, `inc`, `dec`, `max`, `min`, `abs`
- [ ] Comparison: `=`, `not=`, `<`, `>`, `<=`, `>=`, `identical?`, `nil?`, `zero?`, `pos?`, `neg?`
- [ ] Type predicates: `number?`, `string?`, `keyword?`, `symbol?`, `fn?`, `seq?`, `map?`, `vector?`, `set?`, `coll?`
- [ ] Collection ops: `conj`, `assoc`, `dissoc`, `get`, `get-in`, `assoc-in`, `update`, `update-in`, `merge`, `into`, `empty`
- [ ] Seq ops: `first`, `rest`, `next`, `cons`, `seq`, `count`, `nth`, `last`, `butlast`, `reverse`, `concat`
- [ ] Higher-order: `map`, `filter`, `reduce`, `keep`, `remove`, `mapcat`, `take`, `drop`, `take-while`, `drop-while`, `partition`, `partition-all`, `group-by`, `sort`, `sort-by`
- [ ] Lazy sequences: `lazy-seq`, `range`, `repeat`, `iterate`, `cycle`, `take`, `drop`
- [ ] String functions (`clojure.string`): `join`, `split`, `trim`, `upper-case`, `lower-case`, `replace`, `starts-with?`, `ends-with?`
- [ ] I/O: `print`, `println`, `prn`, `pr`, `pr-str`, `str`, `read-string`, `slurp`, `spit`
- [ ] Math: `Math/abs`, `Math/pow`, `Math/sqrt`, `Math/floor`, `Math/ceil`, `Math/round`, trig functions
- [ ] Miscellaneous: `apply`, `comp`, `partial`, `juxt`, `memoize`, `constantly`, `identity`, `not`, `complement`, `gensym`, `type`, `class`, `hash`
- [ ] Core macros: `when`, `when-not`, `when-let`, `if-let`, `if-not`, `cond`, `condp`, `case`, `and`, `or`, `->`, `->>`, `as->`, `doto`, `dotimes`, `doseq`, `for`, `with-meta`, `vary-meta`

---

## Phase 6 — Protocols & Multimethods

- [ ] `defprotocol` — define named protocol with method signatures
- [ ] `extend-type` / `extend-protocol` — implement protocols on types
- [ ] Protocol dispatch (inline cache / vtable)
- [ ] `defmulti` / `defmethod` — arbitrary dispatch multimethods
- [ ] `prefer-method`, `remove-method`
- [ ] Built-in protocols: `ISeq`, `ICollection`, `ICounted`, `IIndexed`, `ILookup`, `IFn`, `IPrintable`, etc.

---

## Phase 7 — Concurrency Primitives

- [ ] `atom` — compare-and-swap with `swap!`, `reset!`, `compare-and-set!`
- [ ] `ref` + software transactional memory (`dosync`, `alter`, `ref-set`, `commute`, `ensure`)
- [ ] `agent` — async update queue (`send`, `send-off`, `await`)
- [ ] `future` — thread-pool backed async computation
- [ ] `promise` — `deliver` / `deref` with blocking
- [ ] `delay` — lazy one-time computation
- [ ] `volatile!` — non-atomic mutable cell for single-thread perf
- [ ] `locking` macro over Rust `Mutex`

---

## Phase 8 — Garbage Collector

- [ ] Evaluate GC strategies (tracing vs. reference counting vs. generational); document decision
- [ ] Implement chosen GC (likely a simple mark-and-sweep first, then generational)
- [ ] Safe-point mechanism: yield points in eval loop and compiled code
- [ ] Write barriers for generational / incremental GC
- [ ] Weak references (for caches, intern tables)
- [ ] GC integration with Rust's ownership: `GcPtr<T>` smart pointer that is opaque to Rust's borrow checker
- [ ] Finalization hooks (for resource cleanup)
- [ ] Tuning knobs: heap size, GC trigger threshold

---

## Phase 9 — Rust Interop

- [ ] Define calling conventions: how Clojure code invokes Rust functions
- [ ] Macro or annotation to expose a Rust `fn` as a clojurust native function (e.g. `#[cljx::export]`)
- [ ] Type marshalling: Clojure `Value` ↔ Rust primitive / struct conversions
- [ ] Error/exception bridging: Rust `Result`/`panic` → Clojure exception
- [ ] Access to Rust structs as opaque objects (`NativeObject` variant in `Value`)
- [ ] Calling Rust trait methods on `NativeObject` values via protocol dispatch
- [ ] Safety restrictions: document which Rust APIs are safe to call from GC-managed code
- [ ] `cljx.rust` namespace with intrinsics (`rust/cast`, `rust/ptr`, `rust/unsafe`, etc.)
- [ ] Dynamic linking: load compiled Rust `.so`/`.dylib` at runtime

---

## Phase 10 — JIT Compiler

- [ ] Choose JIT backend (Cranelift recommended; LLVM as alternative)
- [ ] Define intermediate representation (IR) for clojurust forms
- [ ] Emit IR for core special forms and function calls
- [ ] Type inference / specialization for numeric code paths
- [ ] Inline caches for protocol dispatch and keyword lookup
- [ ] OSR (on-stack replacement) to transition from interpreter to JIT mid-execution
- [ ] Deoptimization path back to interpreter when assumptions are violated
- [ ] JIT compilation threshold (invocation count trigger)
- [ ] Integration with GC: patch compiled code roots, handle safepoints in native frames

---

## Phase 11 — AOT Compiler

- [ ] AOT compilation command: `cljx compile <source> -o <binary>`
- [ ] Whole-program analysis for direct calls and dead-code elimination
- [ ] Emit machine code (via same backend as JIT)
- [ ] Static linking of runtime + GC + core library into single binary
- [ ] Reflection stubs for dynamic features that survive AOT
- [ ] Cross-compilation support (target triples via `--target`)
- [ ] Source maps / debug info (DWARF) for compiled binaries
- [ ] `ns` `:gen-class` equivalent for emitting native shared libraries

---

## Phase 12 — REPL & Tooling

- [ ] Interactive REPL (`cljx repl`): read–eval–print loop with readline support
- [ ] nREPL-compatible server for editor integration
- [ ] `cljx run <file>` — execute a `.cljx` or `.cljc` source file
- [ ] `cljx eval '<expr>'` — evaluate expression from command line
- [ ] Project / build system (`cljx.edn` project descriptor, dependency resolution)
- [ ] Classpath / source-path management
- [ ] `cljx test` — discover and run test namespaces (`clojure.test` compatible)
- [ ] Source formatting tool (`cljx fmt`)
- [ ] Documentation generator

---

## Phase 13 — Error Handling & Debugging

- [ ] Clojure-style exception hierarchy (`ExceptionInfo`, `ex-info`, `ex-data`, `ex-message`, `ex-cause`)
- [ ] Stack traces that include both Clojure source locations and Rust frames
- [ ] `tap>` / `tap` system for non-intrusive value inspection
- [ ] `clojure.spec.alpha` compatible spec/validation library (stretch goal)
- [ ] Debug build mode: retain all source locations and disable JIT optimizations

---

## Phase 14 — Compatibility & Compliance

- [ ] Define and document clojurust/Clojure compatibility surface (what is intentionally different)
- [ ] Run a representative subset of `clojure.test` suite against clojurust
- [ ] Reader compatibility: verify `.cljc` files with `:cljx` conditionals behave correctly alongside `:clj`/`:cljs`
- [ ] Numeric tower parity with Clojure (promotion, overflow to BigInt, etc.)
- [ ] `*clojure-version*` / `*cljx-version*` vars
- [ ] `*print-dup*`, `*print-readably*`, `*print-length*`, `*print-level*` dynamic vars

---

## Stretch Goals

- [ ] ClojureScript-style source-to-source compiler targeting WebAssembly
- [ ] `clojure.core.async` compatible CSP channels (`go`, `chan`, `<!`, `>!`, `alts!`)
- [ ] Native image / musl-linked static binaries for minimal deployment
- [ ] Language Server Protocol (LSP) implementation for editor support
- [ ] Transducers in core collection ops
