# clojurust TODO

Implementation roadmap for a Rust-hosted Clojure dialect. Native file extension is `.cljrs`; also supports `.cljc` with reader conditional `:rust`.

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

- [x] Lexer: tokenize all Clojure lexical forms (symbols, keywords, numbers, strings, characters, booleans, nil)
- [x] Reader: parse tokens into `Form` AST nodes
  - [x] List `(...)`, vector `[...]`, map `{...}`, set `#{...}`
  - [x] Quote `'`, quasiquote `` ` ``, unquote `~`, unquote-splicing `~@`
  - [x] Metadata `^`, deref `@`, var `#'`
  - [x] Anonymous function `#(...)`, regex literal `#"..."`
  - [x] Symbolic values `##Inf`, `##-Inf`, `##NaN`
  - [x] Tagged literals `#tag value`
- [x] Numeric literals: long, double, ratio (`3/4`), arbitrary-precision `N`/`M` suffixes, radix literals (`2r1010`, `16rFF`)
- [x] String escape sequences and multiline strings
- [x] Character literals (`\a`, `\newline`, `\u0041`, etc.)
- [x] Line/column source-location tracking on all forms
- [x] Reader conditionals (`.cljc` / `.cljrs`)
  - [x] `#?(:rust ... :clj ... :cljs ... :default ...)` splicing and non-splicing forms
  - [x] Platform key `:rust` selects Rust-dialect branch (evaluator filters; reader stores all branches)
- [ ] File extension dispatch: treat `.cljrs` as always-Rust-dialect, `.cljc` as cross-platform with conditionals

---

## Phase 3 — Core Data Types & Persistent Collections

- [x] `Value` enum: Nil, Bool, Long, Double, BigInt, BigDecimal, Ratio, Char, String, Symbol, Keyword, List, Vector, HashMap, HashSet, Fn, Macro, Var, Atom, Ref, Agent, Namespace, NativeFunction, …
- [x] Persistent (immutable, structurally shared) collections
  - [x] PersistentList (linked list with O(1) cons)
  - [x] PersistentVector (32-way trie + tail buffer)
  - [x] PersistentHashMap (32-way HAMT)
  - [x] PersistentHashSet (backed by PersistentHashMap)
  - [x] PersistentArrayMap (small maps, ≤8 entries)
  - [x] PersistentQueue (front-list + rear-vector)
- [ ] Metadata support on collections, symbols, and vars (`with-meta`, `meta`)
- [ ] Seq abstraction over all collections + lazy sequences
- [ ] Transient collections for efficient batch mutations
- [x] Equality and hashing consistent with Clojure semantics

---

## Phase 4 — Evaluator & Special Forms

- [x] Environment: lexical scopes + namespace-level vars
- [x] Namespace system (basic `ns`; full `require`/`use`/`alias`/`refer` deferred to Phase 5)
- [x] Special forms
  - [x] `def`, `defn`, `defmacro`, `defonce`
  - [x] `let`/`let*`, `loop`/`loop*`, `recur`
  - [x] `fn`/`fn*` (multiple arities, rest args, sequential destructuring)
  - [x] `if`, `do`, `and`, `or`, `quote`, `var`
  - [x] `try`/`catch`/`finally`, `throw`
  - [x] `set!` for mutable vars
  - [x] `letfn`
  - [ ] `monitor-enter`/`monitor-exit` — Phase 7
  - [ ] `new` / interop forms — Phase 9
- [x] Sequential destructuring in `let`, `fn`, `loop` (`:as` alias, `& rest`)
- [x] Associative / nested destructuring (`{:keys [a b]}`, `:strs`, `:syms`, `:as`, `:or`)
- [x] Tail-call optimization via `recur` (trampoline in `loop*` and `fn*`)
- [x] Macro expansion pipeline (`macroexpand-1`, `macroexpand`)
- [x] Syntax-quote with symbol resolution and gensyms
- [x] Native built-in functions (arithmetic, comparison, predicates, collections, I/O, atoms)
- [x] Bootstrap HOFs (`map`, `filter`, `reduce`, `comp`, `partial`, `when`, `cond`, `->`…) defined in Clojure

---

## Phase 5 — Core Standard Library (`clojure.core` equivalent)

- [x] Arithmetic: `+`, `-`, `*`, `/`, `mod`, `rem`, `quot`, `inc`, `dec`, `max`, `min`, `abs`
- [x] Comparison: `=`, `not=`, `<`, `>`, `<=`, `>=`, `identical?`, `nil?`, `zero?`, `pos?`, `neg?`
- [x] Type predicates: `number?`, `string?`, `keyword?`, `symbol?`, `fn?`, `seq?`, `map?`, `vector?`, `set?`, `coll?`
- [x] Collection ops: `conj`, `assoc`, `dissoc`, `get`, `get-in`, `assoc-in`, `update`, `update-in`, `merge`, `into`, `empty`
- [x] Seq ops: `first`, `rest`, `next`, `cons`, `seq`, `count`, `nth`, `last`, `butlast`, `reverse`, `concat`
- [x] Higher-order: `map`, `filter`, `reduce`, `keep`, `remove`, `mapcat`, `take`, `drop`, `take-while`, `drop-while`, `partition`, `partition-all`, `group-by`, `sort`, `sort-by`
- [x] Lazy sequences: `lazy-seq`, `range`, `repeat`, `iterate`, `cycle`, `repeatedly` (via `Thunk`/`LazySeq` + `Value::Cons`)
- [x] String functions: `join`, `split`, `trim`, `upper-case`, `lower-case`, `replace`, `starts-with?`, `ends-with?`
- [x] I/O: `print`, `println`, `prn`, `pr`, `pr-str`, `str`, `read-string`, `slurp`, `spit`
- [x] Math: `Math/abs`, `Math/pow`, `Math/sqrt`, `Math/floor`, `Math/ceil`, `Math/round`, `Math/log`, `Math/log10`, `Math/exp`, `Math/sin`, `Math/cos`, `Math/tan`, `Math/asin`, `Math/acos`, `Math/atan`, `Math/atan2`, `Math/sinh`, `Math/cosh`, `Math/tanh`, `Math/hypot`, `Math/PI`, `Math/E`
- [x] Miscellaneous: `apply`, `comp`, `partial`, `juxt`, `memoize`, `constantly`, `identity`, `not`, `complement`, `gensym`, `type`, `class`, `hash`
- [x] Core macros: `when`, `when-not`, `if-let`, `when-let`, `if-not`, `cond`, `condp`, `case`, `and`, `or`, `->`, `->>`, `as->`, `doto`, `dotimes`, `doseq`, `for`
- [x] Namespace ops: `in-ns`, `alias`, `refer` (basic); `ns` with `:require`/`:refer-clojure`
- [x] `require` — file-based namespace loading with `:as` alias and `:refer [...]`/`:refer :all`
- [x] `load-file` — evaluate a source file by absolute path
- [x] Source-path management — `--src-path DIR` CLI flag; `standard_env_with_paths`
- [x] Alias resolution in qualified symbol lookup (`fb/foo` resolves via ns aliases)

---

## Phase 6 — Protocols & Multimethods

- [x] `defprotocol` — define named protocol with method signatures
- [x] `extend-type` / `extend-protocol` — implement protocols on types
- [x] Protocol dispatch on first-arg type tag (`type_tag_of`)
- [x] `defmulti` / `defmethod` — arbitrary dispatch multimethods
- [x] `prefer-method`, `remove-method`, `methods`, `satisfies?`, `extends?`, `isa?` (equality stub), `type`
- [ ] Inline protocol dispatch cache — Phase 10 JIT optimization
- [x] Built-in protocols (`ICounted`, `ILookup`, `ISeqable`) — defined in bootstrap.cljrs; extended for List, Vector, Map, Set, String
- [ ] `derive` / full `isa?` hierarchy
- [x] `defrecord` — backed by `TypeInstance` (type_tag + MapValue fields); generates `->Name` and `map->Name` constructors; supports inline protocol impls
- [x] `reify` — creates a unique-tagged `TypeInstance`; supports inline protocol impls
- [ ] `deftype` — blocked by `.` interop (field access via `(.field obj)` not yet implemented; Phase 9); mutable fields require `set!`-on-field semantic that needs interop dot special form; low priority until Phase 9

---

## Phase 7 — Concurrency Primitives ✓

- [x] `atom` — compare-and-swap with `swap!`, `reset!`, `compare-and-set!`
- [ ] `ref` + software transactional memory (`dosync`, `alter`, `ref-set`, `commute`, `ensure`) — **deferred**
- [x] `agent` — async update queue (`send`, `send-off`, `await`, `agent-error`, `restart-agent`)
- [x] `future` — thread-pool backed async computation (`future-done?`, `future-cancelled?`, `future-cancel`)
- [x] `promise` — `deliver` / `deref` with blocking and timeout
- [x] `delay` — lazy one-time computation (`force`, `realized?`)
- [x] `volatile!` — non-atomic mutable cell (`vreset!`, `vswap!`, `volatile?`)
- [x] `compare-and-set!` — CAS on Atom
- [ ] `locking` macro over Rust `Mutex` — **deferred**

---

## Phase 8 — Garbage Collector

- [x] Evaluate GC strategies; document decision (non-moving stop-the-world mark-and-sweep chosen)
- [x] Implement chosen GC: non-moving stop-the-world mark-and-sweep (`GcBox<T>` + intrusive linked list + `GcHeap::collect`)
- [x] `GcVisitor` trait + real `Trace::trace` on all types in `cljx-value` and `cljx-eval` (Phase 8.1)
- [x] Replace `Arc<T>` shim in `GcPtr` with `NonNull<GcBox<T>>` raw pointer; `clone` O(1), `drop` no-op (Phase 8.2)
- [x] `MarkVisitor` with grey stack (handles cycles, avoids recursion stack overflow)
- [x] `HEAP` global singleton; `GcHeap::count`/`total_allocated`/`total_freed` stats
- [ ] Automatic collection trigger (threshold-based) — deferred
- [ ] Safe-point mechanism: yield points in eval loop and compiled code — deferred to Phase 10 (JIT)
- [ ] Write barriers for generational / incremental GC — deferred
- [ ] Weak references (for caches, intern tables) — deferred
- [ ] Finalization hooks (for resource cleanup) — deferred
- [ ] Perceus-style in-place mutation: when ref count == 1, mutate in place — deferred (optimization)

---

## Phase 8-ext-2 — Dynamic Variables (`binding`) ✓

- [x] `Var` gains `meta: Mutex<Option<Value>>` field with `get_meta`/`set_meta`
- [x] `cljx-eval/src/dynamics.rs` — thread-local binding stack (`BINDING_STACK`), RAII `BindingGuard`, `push_frame`/`pop_frame`/`deref_var`/`is_thread_bound`/`set_thread_local`/`capture_current`/`install_frames`/`trace_current`
- [x] `lookup_in_ns` routes through `dynamics::deref_var` (two sites: interns + refers)
- [x] `deref_value` for `Value::Var` routes through `dynamics::deref_var`
- [x] `eval_def` handles `FormKind::Meta` (`^:dynamic`, `^TypeHint`, `^{...}`); stores metadata on var
- [x] `binding` special form: evaluates to RAII-guarded dynamic frame + body
- [x] `set!` checks thread-local binding stack before falling back to root `var.bind`
- [x] Binding conveyance in `future`: `capture_current` + `install_frames` on new thread
- [x] `with-bindings*` intercepted in `eval_call`; `alter-var-root` and `vary-meta` handled similarly
- [x] New builtins: `var-get`, `var-set!`, `bound?`, `thread-bound?`, `meta`, `with-meta`
- [x] Sentinel builtins for `alter-var-root`, `vary-meta`, `with-bindings*` (env-needing, intercepted)
- [x] Bootstrap dynamic vars: `*ns*`, `*out*`, `*err*`, `*assert*`, `*print-dup*`, `*print-readably*`, `*print-length*`, `*print-level*`, `*1`–`*3`, `*e`
- [x] `with-bindings` macro in bootstrap.cljrs
- [x] 10 new tests (259 total)

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
- [ ] RAII resource management: `with-open` and similar resource scopes lower to Rust `Drop` rather than GC finalizers, giving deterministic cleanup with no GC involvement
- [ ] (Stretch) `#rust` typed sublanguage: functions annotated `#rust` receive Rust-typed arguments with lifetime bounds enforced at the interop boundary, bypassing `Value` boxing entirely for those call sites

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
- [ ] Primitive unboxing: where type feedback confirms a value is always `i64` or `f64`, emit raw arithmetic on machine registers — no `Value` boxing, no GC allocation
- [ ] Escape analysis: values that do not escape their defining scope (not returned, not captured, not stored) may be stack-allocated rather than heap-allocated through the GC
- [ ] Call-site monomorphization: generate type-specialized copies of hot functions when call-site type profiles are stable (e.g. `(map inc xs)` where `xs` is always `Vec<i64>`)

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
- [ ] Whole-program escape analysis: with full call graph visibility, identify values that never escape their function or thread and lower them to stack allocation or Rust-owned heap allocation outside the GC

---

## Phase 12 — REPL & Tooling

- [x] Interactive REPL (`cljx repl`): read–eval–print loop (basic; readline deferred)
- [ ] nREPL-compatible server for editor integration
- [x] `cljx run <file>` — execute a `.cljrs` or `.cljc` source file
- [x] `cljx eval '<expr>'` — evaluate expression from command line
- [ ] Project / build system (`cljx.edn` project descriptor, dependency resolution)
- [x] Source-path management (`--src-path DIR` on `run`/`repl`; namespace→file resolution)
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
- [ ] Reader compatibility: verify `.cljc` files with `:rust` conditionals behave correctly alongside `:clj`/`:cljs`
- [ ] Numeric tower parity with Clojure (promotion, overflow to BigInt, etc.)
- [ ] `*clojure-version*` / `*cljx-version*` vars
- [x] `*print-dup*`, `*print-readably*`, `*print-length*`, `*print-level*` dynamic vars (defined in Phase 8-ext-2)
- [ ] Preserve metadata for `assoc`.

---

## Stretch Goals

- [ ] ClojureScript-style source-to-source compiler targeting WebAssembly
- [ ] `clojure.core.async` compatible CSP channels (`go`, `chan`, `<!`, `>!`, `alts!`)
- [ ] Native image / musl-linked static binaries for minimal deployment
- [ ] Language Server Protocol (LSP) implementation for editor support
- [ ] Transducers in core collection ops
