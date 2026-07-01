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
- [x] `:extend-via-metadata true` — dispatch consults the first arg's metadata (keyed by the `ProtocolFn`) before falling back to type-tag `impls`
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

## Phase 8.1 — program optimization

### Phase 8.1.1 — IR Foundation

- [x] Convert AST to A-normal form (`cljrs-compiler/src/anf.rs`).
- [x] Convert ANF to single static assignment form (phi nodes at join points).
- [x] Add explicit instruction types: `AllocCons`, `AllocVector`, `AllocMap`, `AllocClosure`, `Call`, `CallKnown`, `LoadLocal`, `LoadGlobal`, `Return`, `Branch`, `Phi`, `Deref`, `DefVar`, `SetBang`, `Throw`, `Recur`.
- [x] Attach effect metadata to instructions: `Pure`, `Alloc`, `HeapRead`, `HeapWrite`, `IO`, `UnknownCall` (`ir.rs`).

### Phase 8.1.2 — Escape Analysis

- [x] Identify allocation instructions (`escape.rs: collect_allocs`).
- [x] Build def-use chains (`escape.rs: build_use_chains`).
- [x] Track value flow through SSA graph (worklist-based, handles phi cycles).
- [x] Mark allocations as escaping if they:
  - are returned
  - stored in heap objects
  - captured by closures
  - passed to unknown functions
  - inserted into global state (def/set!)
- [x] Everything else becomes non-escaping (`EscapeState::NoEscape`).
- [x] Known function argument escape tracking (`known_fn_arg_escapes`).
- [x] Assoc/conj chain detection (`detect_collection_chains`).
- [x] Inter-procedural escape analysis (`EscapeContext`, `compute_fn_summary`).
- [x] `Returns` state for allocations — see `crates/cljrs-ir/ESCAPE_OPT_PLAN.md` stage 2.
- [x] Caller-context propagation for returned allocations — see plan stage 3.

### Phase 8.1.3 — Function-local regions

- [x] Implement `Region` bump allocator (`cljrs-gc/src/region.rs`).
  - Bump-pointer allocation from pre-allocated chunks (4 KiB default).
  - Drop registry: destructors run in reverse (LIFO) order on reset/drop.
  - ~2.6x faster than GC heap allocation (16 ns/alloc vs 40 ns/alloc in release).
  - Region objects are NOT in the GC heap linked list — zero GC pressure.
- [x] Provide operations
  - `Region::alloc<T>(value)` → `GcPtr<T>` (bump allocation, no mutex)
  - `Region::reset()` — drop all objects, reuse first chunk
  - `RegionGuard` — RAII activation of thread-local region
  - `try_alloc_in_region<T>(value)` — allocate in active region if one exists
  - `region_is_active()` — check if a region is on the thread-local stack

Compiler work:

- [x] Add IR nodes:
  - `RegionStart(VarId)` — begin a region scope
  - `RegionEnd(VarId)` — end a region scope (frees all region objects)
  - `RegionAlloc(VarId, VarId, RegionAllocKind, Vec<VarId>)` — allocate in region
- [x] Replace non-escaping allocations with region allocations (`optimize.rs`).
- [x] Inlining pass before escape analysis (`lower/inline.rs`) — enables cross-function region promotion.
- [x] Region parameter passing for non-inlineable callees (`lower/regionalize.rs`) — see `crates/cljrs-ir/ESCAPE_OPT_PLAN.md` stage 4.
- [ ] Fallback to GC heap for escaping objects — requires compiler codegen (Phase 10/11).

### Phase 8.1.4 — Persistent structure virtualization

**Convert persistent chains into mutable construction**

Pattern detection:

```
m1 = {}
m2 = assoc m1
m3 = assoc m2
m4 = assoc m3
```

Lowering:

```
t = transient_map
assoc_mut t
assoc_mut t
persistent t
```

or even:

```
region_map
insert
freeze
```

Checklist:
- [x] Detect assoc/conj chains in `let` bindings (`virtualize.rs: detect_let_chains`).
- [x] Verify intermediate versions do not escape (body/other-binding reference checks).
- [x] Replace with mutable builder (transient operations in `eval_virtualized_chain`).
- [x] IR-level chain detection (`escape.rs: detect_collection_chains`).

Applies to: maps, sets, vectors.

### Phase 8.1.5 — Small collection specialization

**Optimize tiny maps and vectors.**

- [x] Bulk construction for map literals — `MapValue::from_pairs()` builds in one shot, avoiding N intermediate `GcPtr` allocations from repeated `assoc`.
- [x] Bulk construction for set literals — `PersistentHashSet::from_iter()` uses `insert_mut` internally.
- [x] `PersistentArrayMap::from_flat_entries()` for direct construction from evaluated kv vec.
- [ ] Define specialized inline layouts (SmallMap1/2/4, SmallVec4) — deferred to JIT phase.
- [ ] Compiler-detected literal small collections with specialized lookup — deferred to JIT phase.
- [ ] Convert to HAMT if size exceeds limit (already implemented: ArrayMap promotes at 8 entries).

### Phase 8.1.6 — Map shape system

- [ ] Each map has a shape ID.
- [ ] Shapes store key order and slot offsets.

Example:

```
Shape12:
[:user-id :name]
```

Maps reference:

```
shape_id
values array
```

### Phase 8.1.7 — Inline caches for map lookup

- [ ] Attach cache to each lookup node

Example:

```
LookupNode {
  keyword
  cached_shape
  cached_offset
}
```

Runtime logic:

```
if map.shape == cached_shape
  return values[offset]
else
  slow_lookup()
```

Slow path updates the cache.

Optionally:

- [ ] extend to polymorphic inline caches

### Phase 8.1.8 — Allocation sinking

**Reduce lifetimes further**

- [ ] Move allocations closer to use sites.
- [ ] Shorted object lifetimes.
- [ ] Improve region inference.

E.g.

Before:

```
alloc
if cond
  use
```

After:

```
if cond
  alloc
  use
```

### Phase 8.1.9 — Deforestation / sequence fusion

Eliminate temporary sequences.

Pattern: `(map f (map g xs))`, lower to single loop.

- [ ] Detect common pipelines.
- [ ] Fuse map/filter/reduce.
- [ ] Eliminate sequence allocations.

---

## Phase 9 — Rust Interop

- [x] Define calling conventions: how Clojure code invokes Rust functions
- [x] Macro or annotation to expose a Rust `fn` as a clojurust native function (`#[cljrs_interop::export]` in `cljrs-export-macro`)
- [x] Type marshalling: Clojure `Value` ↔ Rust primitive / struct conversions (`FromValue`/`IntoValue` traits in `cljrs-interop`)
- [x] Error/exception bridging: Rust `Result`/`panic` → Clojure exception (`wrap_result` in `cljrs-interop`)
- [x] Access to Rust structs as opaque objects (`NativeObject` variant in `Value`)
- [x] Calling Rust trait methods on `NativeObject` values via protocol dispatch
- [ ] Safety restrictions: document which Rust APIs are safe to call from GC-managed code
- [ ] `cljx.rust` namespace with intrinsics (`rust/cast`, `rust/ptr`, `rust/unsafe`, etc.)
- [x] Dynamic linking: load compiled Rust `.so`/`.dylib` at runtime — project-local lib via `cljrs build-native` + `load_native_lib`; **pinned native packages** (`:rust/load :dylib`) via `cljrs-dylib` (build the dep's crate at a pinned commit, ABI-fingerprint handshake, register into the `ns@<commit>` namespace). Deferred: statically linking pinned native crates into AOT harnesses (`#[export]` inventory collision between two versions of one crate), and a C-ABI vtable to replace the Rust-ABI `&mut Registry` boundary
- [x] RAII resource management: `with-open` macro + `close` builtin for deterministic cleanup of `Resource` values
- [ ] (Stretch) `#rust` typed sublanguage: functions annotated `#rust` receive Rust-typed arguments with lifetime bounds enforced at the interop boundary, bypassing `Value` boxing entirely for those call sites

---

## Phase 10 — JIT Compiler

A fourth execution tier that compiles hot functions and hot loops to native
code in-process, so ad-hoc code (`cljrs run`, the REPL, `eval`) reaches
AOT-class speed with no explicit compile step. Full architecture and rationale:
[`docs/jit-plan.md`](docs/jit-plan.md). Milestones below map 1:1 to that
document's layers.

Foundations already in place:

- [x] Choose JIT backend (Cranelift)
- [x] Define intermediate representation (IR) for clojurust forms (ANF + SSA + CFG, `cljrs-ir`)
- [x] Emit IR for core special forms and function calls

### Phase 10.0 — Backend refactor

- [x] Make `codegen.rs` generic over `cranelift_module::Module` (drives both `ObjectModule` for AOT and `JITModule` for JIT); AOT behavior unchanged

### Phase 10.1 — Minimal JIT tier (first working JIT)

- [x] New `cljrs-jit` crate (`cranelift-jit` + shared codegen); register `rt_abi` `extern "C"` symbols with `JITBuilder`; materialize constants via runtime calls (no `GcPtr`s in code)
- [x] Per-arity invocation counter + threshold (`CLJRS_JIT_THRESHOLD`, CLI flag) in a `JitState` keyed by `ir_arity_id`
- [x] Background-thread compilation with atomic code-pointer swap (never stall a hot call)
- [x] Dispatch order JIT-native → Tier-1 IR → tree-walk at the `call_cljrs_fn` seam
- [x] Conservative stack scanning of JIT frames for GC roots (sound under the non-moving collector); safepoint polls at loop back-edges and function entry
- [x] Compile the set Tier-1 already handles (non-capturing, no destructuring/rest)

### Phase 10.2 — Code unloading

- [x] Per-version code tagged with `ir_arity_id` + epoch; mark prior arity stale on redefinition (var-rebind hook in `cljrs-value` `Var::bind` → `cljrs-jit` `on_var_rebind` → `code_cache::mark_stale`; dispatch pointer nulled so future calls fall back to the interpreter)
- [x] Reclaim stale epochs at the existing STW safepoint, freeing only epochs with no live JIT frame (resolves the unload-vs-execute race) — per-thread active-frame tracking (`jit_state::push_jit_frame`/`live_epochs`) + `code_cache::reclaim_at_stw` (calls `JITModule::free_memory`), installed via `set_stw_reclaim_hook`

### Phase 10.3 — Shrink the interpreter seam (ROI order)

- [x] Destructured params: expand destructuring to explicit let-bindings in the IR prologue (lowering-only) — `lower_fn_body_destructured` runs the same prologue as inner `fn*` forms (`lower_destructure_binding`), driven by `CljxFnArity.destructure_params`/`destructure_rest` threaded through `lower_arity`; gate in `eager_lower_fn` removed
- [x] Closures with captured bindings: capture lowering + closure-alloc codegen through the JIT — `compile_jit` now recursively declares + compiles closure subfunctions into the same `JITModule` (mirroring `aot.rs`), so `AllocClosure` resolves them. The "closure built by `rt_make_fn` then invoked via `rt_call` returns nil" symptom that blocked this was not a codegen bug — it was the missing-eval-context dispatch-seam bug below. Escaped-closure safety vs. code unloading: a closure value materialized by `rt_make_fn*` captures a raw pointer into the module and lives on the GC heap, invisible to the active-frame scan, so `rt_make_fn*` fires a closure-escape hook that **pins** the executing epoch (`code_cache::pin_epoch`); pinned modules are never freed (a deliberate bounded leak — precise reclamation needs a GC death notification for the closure value). Graceful decline (`lookup_user_func` → clean error) and the panic-resilient worker (`catch_unwind`) remain as the safety net for anything codegen still can't express.
- [x] Variadic / rest params through codegen — codegen already compiles the `(fixed…, rest_list)` signature; the JIT-native dispatch path (`call_jit_native`) now packs the trailing call args into the rest list before invoking native code (mirroring `execute_ir`), fixing silently-dropped rest args (`(mixed 10 20 30 40 50)` → `[10 20 3 30]`, not `[10 20 0 nil]`)
- [~] Promote special-cased ops to first-class `KnownFn`/IR instructions — `apply`, `atom`, `reset!`, `swap!`, `deref` are already `KnownFn`s with rt_abi bridges (mapped in `cljrs-ir/src/lower/known.rs`); `volatile!`/`vswap!`/`vreset!` remain. No longer blocked: the `rt_call`/HOF correctness bugs that made more rt_abi surface net-negative are fixed (below).

  **Resolved: the pre-existing JIT-native correctness bugs.** Both "returns nil under JIT-native dispatch" symptoms — higher-order calls taking a function value (`reduce`/`map`/`filter` with `+`, `inc`, `even?`, or a lambda; any `(f x)` call of a function-valued argument) and in-place calls of a JIT-constructed closure — had a single root cause: **the JIT-native dispatch seam never pushed an eval context.** Every rt_abi bridge that re-enters Clojure (`rt_call`, `rt_load_global`, `call_global_fn`, the HOF bridges) dispatches via `cljrs_env::callback`'s thread-local context; Tier-1 (`execute_ir`) and the AOT preamble push one, `call_jit_native` did not — so `callback::invoke` failed and the bridges swallowed the error into nil. Fixed by installing a guarded eval context (callee's `defining_ns`) around the native call. A second silent-nil hole fixed at the same seam: an *uncaught* `(throw …)` inside native code stashes the value in rt_abi's thread-local and returns the nil sentinel; only an `rt_try` inside compiled code checked it, so the throw vanished (and the stale slot could misfire a later `rt_try`). The seam (and the OSR entry) now takes the pending exception via a hook and re-raises it as `EvalError::Thrown`. Regression-tested end-to-end in `crates/cljrs/tests/jit_seam_correctness.rs`.

### Phase 10.4 — OSR (on-stack replacement)

- [x] Loop back-edge counters at loop headers — `interpret_ir_with_osr` counts `RecurJump`s per header within one execution (lazily allocated; straight-line code pays nothing); crossing `osr_threshold()` (override → `CLJRS_OSR_THRESHOLD` → JIT threshold) issues an idempotent `jit_state::osr_request` to the background worker. Per-execution counting is deliberate: hot-within-one-call is exactly the case invocation tiering misses; loops spread over many short calls are already covered by the invocation counter
- [x] OSR-entry compilation (entry block = loop header; live-ins as params) and mid-loop transfer of the interpreter register file into the native frame — `cljrs_ir::osr::build_osr_function` keeps the blocks reachable from the header, rewires header φs to take an extra edge from a fresh entry block (loop variables arrive as fresh params), passes other pre-loop values as params bound to their original `VarId`s, and drops `RegionEnd`s whose `RegionStart` ran in the interpreter (the interpreter frame closes those regions after the transfer). The worker compiles the variant through the ordinary backend, registers it under its own reclamation epoch (rebind staling covers OSR epochs via `take_osr_epochs`), and publishes `(fn_ptr, epoch, live_ins)`; the interpreter polls at loop-header entry (after φ resolution) and, on `Ready`, snapshots the live-in registers and calls native code with the same rooting + frame-epoch protocol as ordinary JIT-native calls. *Milestone holds:* a single-call 2M-iteration `loop`/`recur` script promotes to native mid-run (`crates/cljrs/tests/osr_promotion.rs`); any transform/compile failure marks the slot failed and the loop finishes at Tier 1

### Phase 10.5 — Context-driven bump allocation

- [x] Thread the active region pointer as a hidden parameter into JIT'd calls — `rt_region_start` returns the real `*mut Region`; `CallWithRegion` carries the caller's handle and codegen passes it as a hidden trailing argument bound to the callee's `RegionParam` (`IrFunction::abi_param_count`); `rt_region_alloc_*` bump directly into the passed region instead of a per-alloc thread-local lookup. The "call-site context" half is the new cross-defn registry (`cljrs-eval/src/defn_registry.rs`): each eagerly-lowered top-level defn is registered, later lowerings consume referenced defns as `ExternalDefn`s, and stage-4 promotion fires in the script/REPL flow (previously whole-program-AOT-only). Redefining a consumed defn invalidates its dependents (cached IR dropped, native code staled via the stale-epoch hook, lazy re-lower on next dispatch) — load-bearing because stage 4 clones the callee body into the caller. End-to-end: `crates/cljrs/tests/region_threading.rs`
- [~] Call-site monomorphization of allocation strategy — the caller-region vs GC-heap choice is made per call site (stage-4 clones a region-parameterised `__rg` variant when the result is `NoEscape`; the heap path is the default `Call`). A distinct static-arena variant is not implemented: in GC builds "static arena" coincides with the GC heap (program-lifetime is the collector's default), and in no-gc builds the interpreter's `StaticCtxGuard` discipline already routes program-lifetime sinks (`def`/atom/`reset!`) to the static arena
- [~] Treat each REPL form / script run as an arena scope; promote the result out before reset — the *promotion* half ships as the publish barriers below, which run while scopes are still live (a form-boundary copy would be too late). The form-wide arena itself is deliberately not adopted: with an always-open region, `box_coll_val`'s opportunistic regioning captures *unproven* allocations, and soundness would then require barriers on every mutation channel (transients, `aset!`, watches, metadata), not just the program-lifetime cells. Revisit if/when allocation sites are classified at codegen time instead of opportunistically
- [x] Extend profile-driven scratch regions to the default GC build — analysis-driven regions now demonstrably run under the tracing collector on the JIT/script path (bit-0-tagged pointers skipped by the marker, live regions traced as roots, `GC_STATS` region counters as the milestone evidence), with a **heap-promotion fallback for escapers**: publish barriers at `Var::bind`, `Atom::new/reset`, `Volatile::new/reset`, `Promise::deliver`, and channel puts (`cljrs-value/src/publish.rs`) scan stored values for region-tagged boxes and deep-copy them to the heap (via the structured-clone machinery); values opaque to the scan (closures, unrealized lazy seqs, native objects) — and task spawns — instead *poison* the active regions, which are then retired (kept alive forever, traced as GC roots) rather than reset (`cljrs_gc::region::{poison_active_regions, close_region}`) — the bounded-leak escape hatch that keeps correctness independent of analysis perfection

*Milestone evidence:* `region_threading.rs` runs an allocation-heavy hot cross-defn call 20k times under the JIT in the default GC build and asserts ≥10k region (bump) allocations in `GC_STATS` — allocations that previously all hit the GC heap.

### Phase 10.6 — Specialization & inline caches

- [x] Type inference / primitive unboxing (`i64`/`f64` in registers, no boxing, no GC alloc) — `cljrs-compiler/src/typeinfer.rs` assigns every IR var a `Repr` (`Boxed`/`Long`/`Double`/`Bool`) by forward dataflow (params seeded from type profiles; constants; arithmetic/comparison closure; phi meets, including recur back-edges); codegen keeps unboxed vars in registers — `iadd`/`fadd`/`icmp`/`fcmp` instead of `rt_add`/`rt_lt` bridge calls (each of which boxed a result on the GC heap) — and boxes only where a value flows into a boxed context (call arg, collection element, return, boxed phi edge). Sound by construction: unboxed ops are emitted only where the bridge semantics are bit-identical (`rt_add` on Long+Long is `wrapping_add` = `iadd`; mixed long/double promotes to f64 exactly as the bridges do; `Div`/`Rem`/cross-type `Eq` stay boxed). Unboxed booleans also skip `rt_truthiness` at branches
- [x] Inline caches for protocol dispatch and keyword lookup — keyword constants compile to a per-call-site writable data slot (fast path: inline load + branch; the first execution interns via `rt_kw_ic_fill` into a permanently rooted global table — previously *every* execution of a keyword literal heap-allocated a fresh keyword, and `rt_const_keyword` now interns too); `Inst::Call` compiles to `rt_call_ic`, which caches `(ProtocolFn identity, dispatch type-tag, generation) → impl fn` per call site, skipping the per-dispatch `Arc<str>` tag allocation + protocol mutex + double hash lookup; the global protocol generation (`cljrs_value::protocol_generation`, bumped on every `extend-type`/`extend-protocol`/inline impl) invalidates every entry on re-extension. IC slots hold only table indices / interned pointers, so compiled modules stay free of GC roots; cached impl fns and interned keywords are kept alive by an IC root tracer registered on each allocating thread's heap
- [x] Call-site monomorphization on stable type profiles — Tier-1 dispatch (`jit_state::record_call`) accumulates a per-parameter type bitmask (Long/Double/other; variadic rest params are never profiled) until the compile is queued; the JIT worker turns a monomorphic profile into per-parameter specs (`specs_from_profile`) and compiles a specialized entry: a prologue that guards each specialized parameter's runtime tag (`rt_value_tag`) and unboxes it into a register — which is what lets the inference above unbox whole loop bodies. Closures/region subfunction variants and OSR entries always compile generic. Disable with `CLJRS_JIT_NO_SPEC=1`; observability via `--jit-stats` (boxed-arith / deopt / IC counters)
- [x] Deoptimization path back to Tier 1 when assumptions are violated — entry guards run before any side effect, so a failure returns a unique sentinel (`rt_deopt`; a `Box::leak`ed non-GC address no real result can alias) and the dispatch seam (`call_jit_native`) re-executes the call at Tier 1 — exact interpreter semantics for the violating call. Failures are counted per arity; crossing `CLJRS_JIT_DEOPT_LIMIT` (default 10) unpublishes the specialized code (module reclaimed through the existing stale-epoch path), bans the arity from re-specialization, and resets its counters so the ordinary hot path recompiles it generically

*Milestone evidence:* `crates/cljrs/tests/jit_specialization.rs` — a hot monomorphic 20k-call loop's boxed-arithmetic bridge calls drop >5× vs the same run under `CLJRS_JIT_NO_SPEC=1`; Double calls against a Long-specialized function deopt with exact Tier-1 results and discard the specialization past the limit; keyword ICs fill once per call site across 20k iterations; protocol dispatch hits the IC and a mid-run `extend-type` is picked up at the same (already compiled) call site.

### Phase 10.7 — Background IR lowering (warm tier) + cold IR eviction

- [x] Warm threshold (Tier 0 → Tier 1) — tree-walked calls bump the per-arity counter (`jit_state::record_interp_call`); crossing `CLJRS_IR_THRESHOLD` (default 50; `--ir-threshold`; 0 disables) macro-expands the fn's arity bodies on the calling thread (macros need the interpreter — the runtime is single-mutator, so expansion cannot move off-thread) and enqueues the expanded `Form`s to the background `cljrs-ir-lower` worker (`cljrs-eval/src/lower_worker.rs`), which runs the Env-free half of lowering (ANF + inlining + escape analysis + region promotion, split out as `lower::lower_expanded_arity`) and publishes via `ir_cache::store_cached`. `cljrs_jit::init` no longer forces eager lowering; `CLJRS_EAGER_LOWER=1` restores it. The Tier-1 counter restarts at IR publish, so `CLJRS_JIT_THRESHOLD` measures pure Tier-1 calls with a full arg-type-profile window
- [x] Scope: **user code only** — background lowering skips macros, async fns, capturing closures, bootstrap-era definitions (arity-id watermark snapshotted by `mark_compiler_ready`), and fns from builtin-source namespaces (clojure.test, clojure.string, …). Shipped namespaces keep their historical behavior (no IR unless opted in): they were never lowered by default before, and several of their patterns trip latent bugs (below)
- [x] Rebind races — `defn_registry::snapshot_externals` records dependent edges atomically with reading the registry (REGISTRY read lock held across the DEPENDENTS write, serializing against `on_redefined`), and the lowering worker is the **only consumer** of relower marks: after `store_cached` it re-peeks `relower_marked` and re-lowers with fresh externals (≤3 attempts) if a rebind landed mid-flight. The dispatch seam only peeks (`relower_marked` + `lower_queued` dedup) — this also fixes the previously-broken relower path, which consumed the mark and then silently skipped re-lowering whenever eager lowering was off
- [x] JIT publish guard — the worker publishes a whole-function compile only if the request's IR is still the arity's current cache entry (`Arc::ptr_eq`); otherwise the module is marked stale (fixes a pre-existing race where a compile from since-invalidated IR could resurrect stale native code)
- [x] Cold IR eviction — `Cached` entries track coarse last-access; `ir_cache::sweep_idle` runs at the stop-the-world reclaim pass (`set_stw_reclaim_hook` is now a multi-hook registry) and evicts entries idle past `CLJRS_IR_CACHE_TTL` (default 600 s) — deliberately *colder* than native code. Arities with published native code or an in-flight compile are skipped (deopt fallback); `Unsupported` markers are kept; eviction drops the `JitEntry` (the fn re-warms from zero) and stales any OSR code
- [x] Pre-existing bug fixed en route: `rt_alloc_vector`/`rt_alloc_map`/`rt_alloc_set`/`rt_alloc_list` formed `slice::from_raw_parts(null, 0)` for empty collection literals (codegen passes null for `n == 0`) — UB that aborts debug builds the moment any JIT-compiled fn builds an empty literal (e.g. a hot `(conj [] x)`)
- [ ] Known pre-existing bugs surfaced (reproduce under `CLJRS_EAGER_LOWER=1` on older trees too; the reason builtin namespaces are excluded above): (1) JIT codegen of a seq-driven `loop*` (e.g. `(loop [s (seq coll) acc []] (if s (recur (next s) …) acc))`) compiles to an infinite loop — Tier-1 executes the same IR correctly; (2) Tier-1 IR closures in clojure.test patterns fail with `Arity { name: "<ir-closure>", expected: 2, got: 0 }` (`cargo test -p cljrs-stdlib --test compiler_clojure_tests` with `CLJRS_EAGER_LOWER=1`)
- [ ] Known limitation: a long-running loop entered at Tier 0 cannot tier up mid-call (the tree-walker has no OSR); `CLJRS_EAGER_LOWER=1` is the escape hatch

*Milestone evidence:* `crates/cljrs/tests/background_lowering.rs` — without `CLJRS_EAGER_LOWER`, a hot fn tiers tree-walk → background-lowered IR → JIT-native mid-run with per-iteration correctness; the debug log shows `background lower published`; rebinding a dependency mid-warm takes effect immediately; the worker runs without `cljrs_jit::init` (`CLJRS_NO_JIT=1`); `--ir-threshold 0` produces no lowering.

### Phase H (deferred) — Async JIT

- [ ] Emit Cranelift state machines with explicit resume points for `^:async` functions, integrating with the Tokio executor

---

## Phase 11 — AOT Compiler

- [x] AOT compilation command: `cljx compile <source> -o <binary>`
- [ ] Whole-program analysis for direct calls and dead-code elimination
- [x] Emit machine code (via same backend as JIT)
- [x] Static linking of runtime + GC + core library into single binary
- [ ] Reflection stubs for dynamic features that survive AOT
- [ ] Cross-compilation support (target triples via `--target`)
- [ ] Source maps / debug info (DWARF) for compiled binaries
- [ ] `ns` `:gen-class` equivalent for emitting native shared libraries
- [ ] Whole-program escape analysis: with full call graph visibility, identify values that never escape their function or thread and lower them to stack allocation or Rust-owned heap allocation outside the GC

---

## Phase 11.5 — AOT Clojure → WebAssembly backend

Handoff + design doc: [`docs/wasm-aot-plan.md`](docs/wasm-aot-plan.md).

Native-fast, sandbox-safe browser deployment. A *second* code-generation
backend over the same regionalized `cljrs-ir` IR (parallel to Cranelift), since
no in-sandbox native JIT is possible in a browser. Build-time AOT-wasm is the
top tier; the IR interpreter stays on board as the dynamic-code tier (tiers
invert vs. native). All upstream passes (ANF lowering, escape analysis, region
inference, `typeinfer`, the `rt_abi` contract) are reused unchanged.

- [x] Scaffold the backend: `cljrs-compiler/src/wasm/` (`mod`, `abi`, `reloop`, `emit`)
- [x] ABI/region contract: `Value`→`i32` linear-memory offset; `rt_abi` import table; region handle as a hidden trailing `i32` param (mirrors `IrFunction::abi_param_count`)
- [x] Relooper data model (`Structured`) + acyclic/diamond structuring (wasm-private; Cranelift keeps the raw CFG)
- [x] Relooper: full dominator-based structuring (Ramsey "Beyond Relooper") — `loop`/`recur` back-edges → `Loop`/`Br`-continue, multi-predecessor joins → dominator-placed labeled blocks in ascending RPO, irreducible CFGs rejected
- [x] `wasm-encoder` emitter (core): boxed `i32` value model, `rt_abi` imports, structured-tree → wasm control flow with label-stack `br` resolution, SSA φ as parallel operand-stack moves, scalar constants, folded arithmetic + binary comparison bridges; emits `wasmparser`-validated modules
- [x] `wasm-encoder` emitter — `Alloc*` (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/`AllocCons`): element-pointer arrays marshalled through an imported `"rt" "memory"` + the `rt_scratch_ptr` buffer, then the slice-taking `rt_alloc_*` bridge (map count = pair count, empty = null/0, cons = two pointer args)
- [x] `wasm-encoder` emitter — region ops (`RegionStart`/`RegionAlloc`/`RegionEnd`/`RegionParam`): `rt_region_*` bridges with the handle threaded as a leading `i32` (reusing the `rt_scratch_ptr` marshalling; map = pair count, cons = two direct pointers), and `RegionParam` bound from the hidden trailing-`i32` param — `emit_function` sizes the signature from `IrFunction::abi_param_count`; the `takes_region_param`→`Unsupported` guard is dropped (`CallWithRegion` still needs multi-function modules)
- [x] `wasm-encoder` emitter — calls + multi-function modules (`emit_bundle`/`compile_bundle`): two-pass index assignment (discover imports, then settle `func_base` = import count) so `CallDirect`/`CallWithRegion` resolve a bundled callee's wasm index (region variant threads the handle as the hidden trailing arg); `Call` → dynamic dispatch via `rt_call` with args marshalled through `rt_scratch_ptr`; bundles flatten each function's `subfunctions`
- [x] `wasm-encoder` emitter — closures + function table: `AllocClosure` via an active `funcref` element segment installing the bundle's functions into the imported `"rt" "__indirect_function_table"` at `FUNC_TABLE_BASE` (wasm function pointers are table indices) + `rt_make_fn`/`rt_make_fn_variadic`/`rt_make_fn_multi` (name/captures/multi-arity arrays marshalled through one `rt_scratch_ptr` reservation); the `rt_call_ic` inline cache (writable per-call-site IC slot) is still deferred with the data-segment work
- [x] `wasm-encoder` emitter — string/keyword/symbol constants (`Const::Str`/`Keyword`/`Symbol`): UTF-8 bytes interned into a deduplicated read-only data pool emitted as one active data segment at `abi::RODATA_BASE` in the imported memory, resolving to the `(ptr, len)` pair passed to `rt_const_string`/`_keyword`/`_symbol` (keywords skip the per-call-site IC, which stays deferred with `rt_call_ic`); the rodata base is the linear-memory analogue of `FUNC_TABLE_BASE`, finalized in the CLI/bundling step
- [x] `wasm-encoder` emitter — globals/vars (`LoadGlobal`/`LoadVar`/`DefVar`/`SetBang`): the `rt_load_global`/`rt_load_var`/`rt_def_var`/`rt_set_bang` bridges with the `(ns, name)` byte pairs drawn from the rodata pool (`push_name_args` → `intern_rodata`); versioned `name@sha` names resolve inside `rt_load_global` uncached (the per-call-site versioned IC stays deferred with `rt_call_ic`); `rt_set_bang`'s `*const Value` result is dropped
- [x] `wasm-encoder` emitter — unboxed scalar values (intermediates): `typeinfer::infer` + a wasm-private `refine_reprs` cleanup type each `VarId`'s local (`Long`→`i64`, `Double`→`f64`, `Bool`→`i32`), so intermediate arithmetic/comparison lower to native `i64`/`f64` ops (checked long `+`/`-` with the signed-overflow branch → `rt_overflow_error`/`rt_throw`/early boxed-nil return; long `*` demoted to boxed `rt_mul` for lack of `i64.mul_hi`) and values box on demand (`get`) only at boxed contexts; `refine_reprs` transitively demotes any unboxed producer the emitter can't lower so reprs and local types stay in lock-step
- [x] `wasm-encoder` emitter — unboxed **parameter** ABI aligned with `function_signature` (`is_typed`): a function with `^long`/`^double` param hints (`seed_reprs`) compiles to a *typed body* (hinted params unboxed `i64`/`f64`) plus a boxed-entry **trampoline** (`emit_trampoline`) that coerces boxed args (`rt_coerce_long`/`rt_coerce_double`) and (tail-)calls the body; the trampoline is the primary entry (export/table-slot/`CallDirect` target) so all always-boxed dispatch reaches a typed fn unchanged, and a violated static hint coerces or throws (no in-sandbox deopt seam). Typed bodies are appended after the `n` primaries; passing unboxed args *directly* on a same-bundle `CallDirect` is a further optimization left for later
- [ ] GC heap in linear memory (reuse the `wasm32-unknown-unknown` GC) + `rt_safepoint` at entry/back-edges
- [x] `recur` → `loop`/`br`; cross-function tail calls via the wasm tail-call proposal (`return_call`) when `WasmBackend::tail_calls` (a trailing returned `CallDirect`/`CallWithRegion` → `return_call`); a trampoline fallback for `tail_calls` off — and `return_call` for dynamic `Call`s — remain deferred (ordinary `call` + `return` is emitted, correct but not constant-stack)
- [x] `throw`/`try`/`catch` via the `rt_abi` thread-local error path: `Inst::Throw` → `rt_throw` (stashes the exception, returns nil which is dropped, block falls into its `unreachable`/return terminator) and `KnownFn::TryCatchFinally` → `rt_try(body, catch, finally)` over the boxed thunks (mirrors the Cranelift backend); the wasm exception-handling proposal (`try`/`catch`/`throw`, gated on `WasmBackend::exceptions`) is a deferred alternative — the thread-local path is always used
- [x] CLI front-end: `cljrs compile <file> --target wasm -o <out>.wasm` → `aot::compile_file_to_wasm` (lower entry ns → `optimize_direct_calls` → `wasm::compile_bundle` over the entry fn + flattened subfunctions → write validated module; `AotError::Wasm`); emits the entry namespace's AOT module (the `"rt"` imports await runtime linking). End-to-end `wasmparser`-validated in `crates/cljrs-compiler/tests/wasm_compile.rs`
- [x] Cross-namespace bundling: `compile_file_to_wasm` lowers the entry ns **and every transitively-required user namespace** the backend can lower (`lower_file_to_ir_bundle` → `discover_bundled_sources`/`lower_namespace`, mirroring `compile_file`), emitting each `__cljrs_ns_init_N` (+ flattened subfunctions) into one module; a non-lowerable ns is skipped for the IR-interpreter tier (graceful degradation). Names are globally unique (`GLOBAL_NAME_CTR`)
- [x] Configurable memory/table layout: `abi::WasmLayout { rodata_base, func_table_base }` on `WasmBackend` (Default = the `0` placeholders) replaces the hardcoded `RODATA_BASE`/`FUNC_TABLE_BASE` throughout the emitter (data/element-segment offsets, table minimum, string-const `(ptr,len)`, closure table slots), so the linking step can *finalize* the bases against the runtime's real layout
- [ ] Runtime linking (runtime-side): make `cljrs-wasm` **export** the `rt_abi` surface + reserve memory/table regions, instantiate the AOT module against those exports (passing the reserved bases through `WasmLayout`), and wire the IR interpreter into the bundle as the dynamic-code tier (drop JIT/OSR hooks in-sandbox)
- [ ] WasmGC (host-managed reference types) — deferred; keep the linear-memory GC for now

---

## Phase 12 — REPL & Tooling

- [x] Interactive REPL (`cljx repl`): read–eval–print loop (basic; readline deferred)
- [x] nREPL-compatible server for editor integration (`cljrs nrepl`, `cljrs-nrepl` crate)
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
- [ ] Full array compatibility with clojure.
  - [x] Regular object arrays (e.g. object-array) as Vec<Value>. Don't cheat and use PersistentVector.
  - [x] Coercion via vec, seq, etc.
  - [x] Equality support against collections.
  - [ ] `amap` and `areduce`.
  - [ ] Multi-dimensional aset variants.
- [ ] Implement `sorted-map-by` and `sorted-set-by`.
- [ ] Implement hierarchies `ancestors`, `descendants`, `derive`, `underive` etc.
- [ ] Implement `ref` and STM.
- [x] Implement `clojure.data` and `clojure.walk` namespaces.
- [ ] Implement `clojure.zip` and `clojure.pprint` namespaces.
- [x] Implement `transduce` and transducer variants of common higher-order functions.

---

## Stretch Goals

- [ ] ClojureScript-style source-to-source compiler targeting WebAssembly
- [ ] `clojure.core.async` compatible CSP channels (`go`, `chan`, `<!`, `>!`, `alts!`)
- [ ] Native image / musl-linked static binaries for minimal deployment
- [~] Language Server Protocol (LSP) implementation for editor support
  - [x] v1 (`cljrs-lsp` crate + `cljrs lsp` subcommand, syntactic/reader-based): parse
        diagnostics (with per-top-level-form error recovery) and document-symbol outline;
        UTF-8/UTF-16 position-encoding negotiation; FULL text sync
  - [ ] v2 (semantic, evaluator-backed via `cljrs-eval` `GlobalEnv`): hover, completion,
        go-to-definition, find-references
  - [ ] INCREMENTAL text sync, semantic tokens
- [ ] Transducers in core collection ops

---

## cljrs-net — QUIC + HTTP/3 (`docs/quic-http3-integration-plan.md`)

Using **quinn 0.11** (rustls-ring backend, no new native build) + `h3`/`h3-quinn`
for HTTP/3.  Each phase ships new source files + tests in one commit.

- [x] **Q1 — QUIC client transport.** `quic_config.rs` (wraps `tls::build_client_config`
      into `quinn::ClientConfig` via `QuicClientConfig::try_from`), `quic.rs`
      (`connect_to`, `open_stream_on`, `connect`/`open-stream`/`close` builtins,
      pool accept/open loops, `QuicConnectionResource`/`QuicStreamResource`),
      `clojure_rust_net_quic.cljrs` (sugar: `with-stream`, `drain-stream`).
      Tests: echo round-trip against a quinn in-test server; connect-failure path.
- [x] **Q2 — QUIC server transport.** `quic.rs` `listen`/`listen-close`,
      `endpoint.accept()` pool loop, `:conns`/`:streams` LocalSet bridges,
      `QuicListenerResource`. Tests: server echo round-trip, listener close.
- [ ] **Q3 — HTTP/3 client.** `h3.rs` client over `h3-quinn`: `h3/get`/`request`,
      response-body streaming to a `:body` channel.  Depends on Q1.
- [ ] **Q4 — HTTP/3 server.** `h3.rs` server: request map + `respond` fn,
      `send_response`/`send_data`.  Depends on Q2+Q3.
- [ ] **Q5 (optional) — QUIC datagrams.** `send_datagram`/`read_datagram`;
      connection-level `:dgram-in`/`:dgram-out` channels.
