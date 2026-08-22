# cljrs-compiler

Program analysis, optimization, JIT, and AOT compilation for clojurust — one
package for every code-generating path. Provides an intermediate
representation (IR) in A-normal form with SSA, escape analysis, Cranelift-based
native code generation, and a C-ABI runtime bridge.

The JIT (`jit/`) and the AOT backends (`aot.rs`, `wasm/`) are not separate
products: they share `typeinfer`, `codegen`, and the `rt_abi` runtime ABI as
sibling modules, so a change to the calling convention or to representation
inference lands in both at once.

ANF lowering and escape analysis run in pure Rust (`cljrs_ir::lower`, in the
`cljrs-ir` crate); the Cranelift codegen backend here consumes the resulting
`IrFunction` structs directly.

**Phase:** 8.1 (optimization) + 10.0 (backend refactor) + 11 (AOT compilation) + no-gc phases 6–7 — end-to-end AOT working for multi-file programs with variadic functions, protocols, escape analysis optimization, apply, core HOFs, sequence/collection ops, type predicates, atom constructor, and inline expansions.  Under the `no-gc` feature the AOT driver also runs the **blacklist analysis** (`escape.rs`) which rejects programs that cannot be safely compiled without a GC.

**Phase 10.0 (backend refactor):** `Compiler` and `FunctionTranslator` are now generic over `cranelift_module::Module` (`Compiler<M: Module = ObjectModule>`).  The shared CLIF-emitting logic (`compile_function`, `declare_function`) and the full `rt_abi` symbol declaration table (`declare_runtime_funcs`) work with any `Module` backend.  AOT-specific construction (`Compiler::new`) and finalisation (`Compiler::finish`) live in `impl Compiler<ObjectModule>`; the free function `new_compiler_from_module` lets the `jit/` module hand a pre-built `JITModule` to the shared codegen.

**Phase 10.6 (specialization & inline caches):** `typeinfer.rs` infers a machine representation (`Repr::{Boxed, Long, Double, Bool}`) for every IR var; codegen keeps unboxed values in registers (`iadd`/`fadd`/`icmp` instead of `rt_add`/`rt_lt` bridge calls), boxing only at boxed-context uses.  `compile_function_with_specs` compiles a type-specialized entry whose prologue guards each specialized parameter's runtime tag and returns the deopt sentinel on mismatch.  Keyword constants and `Inst::Call` sites compile through per-call-site inline caches (writable module data slots + the `rt_kw_ic_fill` / `rt_call_ic` bridges).

---

## File layout

```
src/
  lib.rs        — module declarations
  rt_abi.rs     — C-ABI runtime bridge: ~40 extern "C" functions called by compiled code
  codegen.rs    — Cranelift code generator: IrFunction → native object code
  typeinfer.rs  — Phase 10.6 scalar representation inference (Repr lattice, fixpoint dataflow)
  aot.rs        — AOT driver: source → parse → expand → lower → codegen → cargo build → binary
  escape.rs     — (no-gc only) blacklist analysis: 4 checks that reject no-gc–unsafe IR patterns
  extensions.rs — Extension / ExtensionSet descriptors + CompileSession: what the *host* supplies
  jit/          — in-process JIT tier (Cranelift `JITModule`) over the same codegen
    mod.rs        — `Jit` (the runtime's `JitBackend`) + `install`; `on_var_rebind` stales superseded code
    jit_compiler.rs — `compile_jit` / `compile_jit_poll`: build a `JITModule`, register rt_abi symbols, call shared codegen
    jit_worker.rs   — background compile thread; maps Tier-1 type profiles to per-parameter specializations
    code_cache.rs   — epoch-tagged module registry; stale/pin/reclaim at stop-the-world safepoints
    async_jit.rs    — `^:async` arity activation: lower → state machine → native poll fn
    osr_integration.rs — (test-only) end-to-end OSR entry compile + call
  wasm/         — (feature `wasm-aot`) AOT Clojure → WebAssembly backend (second backend over the same IR)
    mod.rs      — public API (`compile_function`, `compile_bundle`, `WasmBackend`, `WasmError`); browser tier model
    abi.rs      — ABI/region contract: Value→i32, rt_abi import table, region-handle threading
    reloop.rs   — relooper: IR CFG → structured control flow (`Structured`); wasm-private
    emit.rs     — wasm-encoder emitter: IrFunction(s) → validated wasm module (subset; multi-function)
```

### WebAssembly backend (`wasm/`, feature `wasm-aot`)

Behind the default-on `wasm-aot` feature, together with
`aot::compile_file_to_wasm` and the `AotError::Wasm` variant.  Building with
`--no-default-features` drops the backend and the `wasm-encoder` dependency,
for hosts that only ever produce native binaries.

**Phase 12-wasm (scaffold).** A second code-generation backend over the same
regionalized `cljrs-ir` IR, targeting the browser, where no in-sandbox native
JIT is possible. AOT-wasm is the build-time top tier; the IR interpreter stays
on board as the dynamic-code tier. Everything upstream of codegen — ANF
lowering, escape analysis, region inference, `typeinfer`, the `rt_abi` contract
— is reused unchanged. Because regions are a property of the IR and a region
handle is just an `i32` linear-memory offset, escape-analysis-driven bump
allocation ports for free (a region-parameterised variant takes the handle as a
hidden trailing `i32` param). The only new, wasm-specific work is the
**relooper** (`reloop.rs`, recovering structured control flow — wasm-private,
since Cranelift wants the raw CFG) and the `wasm-encoder` **emitter**
(`emit.rs`).

The **relooper is complete for reducible CFGs** (the universal case for Clojure
source): it implements Ramsey's *"Beyond Relooper"* dominator-tree structuring —
straight-line code, `if`/`cond` diamonds, sequential and nested merges, and
`loop`/`recur` loops with conditional exits. It exploits two facts: back edges
are exactly `Terminator::RecurJump` (so loop headers are the `RecurJump`
targets), and merge nodes (≥2 forward predecessors) are placed at their
immediate dominator in ascending reverse-postorder so every `br` jumps forward.
Irreducible control flow (which Clojure cannot produce) is rejected.

The **emitter produces real, `wasmparser`-validated modules** for a growing
subset of the IR. Each `VarId` is a boxed `i32` local (the universal repr,
always correct, mirroring the Cranelift boxed fallback); `rt_abi` symbols are
imported from the `"rt"` module. The relooper's structured tree maps directly to
wasm control flow (`Labeled`→`block`, `Loop`→`loop`, `If`→`if`/`else`,
`Br`→`br N` with depths resolved from a label stack), and SSA φs are resolved as
parallel moves on the operand stack at each edge — so `loop`/`recur` with a
swapping `recur` is correct. Currently lowered: scalar constants,
string/keyword/symbol constants (UTF-8 bytes interned into a deduplicated
read-only data pool emitted as one active data segment at `RODATA_BASE`, then
`(ptr, len)` passed to `rt_const_string`/`_keyword`/`_symbol`), `LoadLocal`,
folded boxed arithmetic (`+ - * / rem`), binary comparison (`= < > <= >=`),
collection allocation (`AllocVector`/`AllocMap`/`AllocSet`/`AllocList`/
`AllocCons` — element arrays marshalled through an imported linear memory and the
`rt_scratch_ptr` buffer), region operations (`RegionStart`/`RegionAlloc`/
`RegionEnd` → the `rt_region_*` bridges with the handle as a leading `i32`, and
`RegionParam` → the hidden trailing-`i32` param, sizing the signature from
`IrFunction::abi_param_count`), calls, and all control flow.

`compile_bundle` compiles several functions (each top-level function plus its
flattened `subfunctions`) into one module so a direct call can resolve its
callee to a wasm function index. Because imported functions occupy the low
function-index space, the emitter runs **two passes**: pass 1 discovers each
body's `rt_abi` imports; pass 2 re-lowers with `func_base = imports.len()`
settled, so `CallDirect` targets resolve to their final indices. Calls lowered:
`CallDirect`/`CallWithRegion` → a direct `call` to the resolved index (the
region variant threading the caller's handle as the hidden trailing arg), and
`Call` → dynamic dispatch through `rt_call` with arguments marshalled through the
`rt_scratch_ptr` buffer.

**Closures** (`AllocClosure`) lower through `rt_make_fn` / `rt_make_fn_variadic`
/ `rt_make_fn_multi`. An arity function's pointer is a `wasm32` **table index**,
so the module imports the runtime's shared indirect function table (`"rt"
"__indirect_function_table"`, mirroring the imported memory) and installs every
defined function into it with an active `funcref` element segment at
`FUNC_TABLE_BASE`; the closure name, captures, and (multi-arity) the
fn-pointer/param-count/variadic arrays are marshalled contiguously through one
`rt_scratch_ptr` reservation. **Cross-function tail calls** lower a trailing
direct call whose result is returned to `return_call` when `WasmBackend::
tail_calls` is set (else an ordinary `call` + `return`). **Globals / vars**
(`LoadGlobal`/`LoadVar`/`DefVar`/`SetBang`) lower to the `rt_load_global` /
`rt_load_var` / `rt_def_var` / `rt_set_bang` bridges, drawing the `(ns, name)`
byte pairs from the same rodata pool the string constants use (versioned
`name@sha` names are resolved inside `rt_load_global`, uncached — the
per-call-site versioned IC is deferred with `rt_call_ic`). **Exceptions**
(`Throw`, `KnownFn::TryCatchFinally`) lower to the thread-local error path the
native backend uses: `rt_throw` stashes the exception in a thread-local (its nil
result dropped) and `rt_try(body, catch, finally)` invokes the body thunk, routes
a pending exception into the catch thunk, and always runs the finally thunk; the
wasm exception-handling proposal (gated on `WasmBackend::exceptions`) is a
deferred alternative. The `rt_call_ic` inline cache (needs a writable IC data
region) and the async ABI return `Unsupported` — the next lowering increments.

**Unboxed scalars.** `typeinfer::infer` assigns each `VarId` an unboxed `Repr`
(`Long`→`i64`, `Double`→`f64`, `Bool`→`i32` 0/1) where the boxed bridge's exact
semantics survive on the raw representation, so a value's wasm local takes that
machine type and intermediate scalar arithmetic compiles to native `i64`/`f64`
ops instead of the heap-allocating `rt_*` bridges. Values are **boxed only where
they flow into a boxed context** (call arg, collection element, `return`, boxed
φ, var bridge). Promoting long `+`/`-`/`*` stay boxed because overflow can
change the result type to BigInt. A `refine_reprs` pass **demotes back to
boxed**, transitively, any unboxed producer the emitter cannot lower, so the
repr map only ever marks a value unboxed when the emitter can produce it.

**Typed parameter ABI.** A function with `^long`/`^double` parameter hints
(`seed_reprs`, `is_typed`) compiles to **two** wasm functions: a *typed body*
whose hinted params are unboxed `i64`/`f64` (so the body reads them with no
per-use unbox), and a boxed-entry **trampoline** (`emit_trampoline`) with the
all-`i32` signature every dispatcher expects. The trampoline is the function's
primary entry — exported, installed in the shared table, and the target of every
`CallDirect` — so all the always-boxed dispatch paths (dynamic `rt_call`,
indirect closure calls, cross-function direct calls) reach a typed function
unchanged; it coerces each boxed argument (`rt_coerce_long`/`rt_coerce_double`)
and (tail-)calls the body. There is no in-sandbox deopt seam, so a violated
static hint *coerces or throws* rather than re-dispatching at Tier 1. The typed
bodies are appended after the `n` primaries (so primary indices, table slots, and
exports are unchanged); passing unboxed arguments *directly* on a same-bundle
`CallDirect` (the caller-side win) is a further optimization left for later.

```rust
pub fn compile_function(func: &IrFunction, cfg: &WasmBackend) -> Result<Vec<u8>, WasmError>;
pub fn compile_bundle(funcs: &[&IrFunction], cfg: &WasmBackend) -> Result<Vec<u8>, WasmError>;
pub struct WasmBackend { tail_calls: bool, exceptions: bool, layout: abi::WasmLayout }
pub enum WasmError { Reloop(RelooperError), Unsupported(String), Unimplemented(&'static str) }
// abi:    WasmValType{I32,I64,F64}, RtImport, RT_IMPORTS, lookup(name),
//         WasmLayout{rodata_base,func_table_base} (memory/table bases; Default = 0 placeholders)
// reloop: Structured{Simple,Labeled,Loop,If,Br,Return,Unreachable,Nil}, reloop(func)
//         RelooperError{Empty,DanglingTarget,Irreducible}
// emit:   emit_function(func, structured, cfg), emit_bundle(funcs, cfg), function_signature(func)
```

---

## Public API

### IR types (from the `cljrs-ir` crate)

These are **not** defined or re-exported here — `cljrs-compiler` imports them
from `cljrs_ir` directly, as does `cljrs_runtime::tiered`. Listed for reference because
every signature below is stated in terms of them.

```rust
pub struct IrFunction { name, params, blocks, ... }
pub struct Block { id, phis, insts, terminator }
pub enum Inst { Const, LoadLocal, LoadGlobal, AllocVector, AllocMap, AllocSet, AllocList, AllocCons, AllocClosure, CallKnown, Call, Deref, DefVar, SetBang, Throw, Phi, Recur, SourceLoc, RegionStart, RegionAlloc, RegionEnd }
pub enum RegionAllocKind { Vector, Map, Set, List, Cons }
pub enum Terminator { Jump, Branch, Return, RecurJump, Unreachable }
pub enum KnownFn { Vector, HashMap, Assoc, Conj, Get, Count, Add, Sub, Apply, Reduce2, Map, Filter, Mapv, Range1, Take, Drop, Concat, Sort, Keys, Vals, Merge, Update, Atom, ... }
pub enum Effect { Pure, Alloc, HeapRead, HeapWrite, IO, UnknownCall }
```

### Runtime bridge (`rt_abi.rs`)

All functions are `#[unsafe(no_mangle)] pub extern "C"` — called by symbol name from compiled code.

- **Constants:** `rt_const_nil`, `rt_const_true`, `rt_const_false`, `rt_const_long(i64)`, `rt_const_double(f64)`, `rt_const_char(u32)`, `rt_const_string(ptr, len)`, `rt_const_keyword(ptr, len)`, `rt_const_symbol(ptr, len)`.  nil, true/false, and longs in `0..1024` are interned once per process via `cljrs_gc::static_alloc` (program-lifetime, **not** GC-heap allocations — nothing traces the intern caches, so GC-managed entries would be swept after two collections and every compiled use would read freed memory; see `tests/interned_scalars.rs`)
- **Truthiness:** `rt_truthiness(v) -> u8`
- **Arithmetic:** `rt_add`, `rt_sub`, `rt_mul` (promote overflowing longs to BigInt), `rt_div`, `rt_rem`, `rt_unchecked_add`, `rt_unchecked_sub`, `rt_unchecked_mul` (wrapping). `rt_overflow_error` remains available for explicit primitive-long codegen paths; promoting core arithmetic stays boxed.
- **Comparison:** `rt_eq`, `rt_case_eq` (type-strict equality for `case` dispatch — `Long`/`BigInt` interchangeable, mixed numerics never equal), `rt_lt`, `rt_gt`, `rt_lte`, `rt_gte`
- **Primitive arrays:** `rt_alength(arr) -> i64`, `rt_aget_long(arr, i) -> i64`, `rt_aget_double(arr, i) -> f64` (unboxed element loads), `rt_aset_long`/`rt_aset_double` (unboxed stores), `rt_aget`/`rt_aset` (boxed fallback for unknown element types) — all bounds-checked, throwing on out-of-range / type mismatch
- **Collections:** `rt_alloc_vector`, `rt_alloc_map`, `rt_alloc_set`, `rt_alloc_list`, `rt_alloc_cons`, `rt_get`, `rt_count`, `rt_first`, `rt_rest` (both seq any value — vector/list/cons fast paths, plus string/map/set/lazy-seq via `seq`; `rt_first`/`rt_nth`/`rt_peek` return an *interior pointer* into vector storage, so escape analysis treats them as aliasing arg 0), `rt_next` (`seq`-of-`rest`: returns `nil` when exhausted, unlike `rt_rest`), `rt_assoc`, `rt_conj`
- **Scratch:** `rt_scratch_ptr(n_bytes: u32) -> *mut u8` — a thread-local, monotonically growing scratch buffer the wasm backend uses to marshal element-pointer arrays before the slice-taking `rt_alloc_*` bridges (the native backend uses an on-stack slot instead)
- **Region alloc:** `rt_region_start() -> *mut Region` (returns the real region pointer; also pushes it onto the thread-local stack for opportunistic allocation and GC root tracing), `rt_region_end(*mut Region)`, `rt_region_alloc_vector/map/set/list/cons(*mut Region, ...)` — these bump directly into the passed region (the handle threaded through `RegionStart`/`RegionParam`/`CallWithRegion`; a null handle falls back to the thread-local lookup). Region closes route through `cljrs_gc::region::close_region`, honouring the Phase 10.5 poison/retire protocol; `rt_try` saves/unwinds the rt-side and gc-side region-stack depths independently
- **Dispatch:** `rt_call(callee, args, nargs)`, `rt_deref(v)`, `rt_load_global(ns, ns_len, name, name_len)`

#### Eager region-aware fast paths

Several higher-order/collection builtins carry a native Rust fast path that
realizes their result directly (via `box_coll_val` / `alloc_inner_coll`, which
route into the active bump region when one is open) instead of calling back
into the tree-walking interpreter (`call_global_fn`). The interpreted path
allocates every intermediate lazy-seq cons cell on the GC heap and is blind to
the active region, so these fast paths both eliminate allocations and move the
survivors into the region:

- `rt_mapcat(f, coll)` — `f` a `Map`, `coll` a `Vector`: concatenate looked-up
  collections into a fresh `Vector`.
- `rt_into(to, from)` — `Vector` target (any eager `from`), hash-`Set` target
  (eager `from`), or `Map` target (eager `from` of key/value pairs, or a source
  map): build the target directly. The map path realizes via
  `MapValue::from_pairs` (last-wins, size-optimal) so there are no intermediate
  map boxes. Only fires for eager sources — a lazy `for`/`map` source still
  falls back to the interpreter.
- `rt_count_filter` / `rt_into_filter` / `rt_into_mapcat` / `rt_into_map` —
  fused `count`/`into` over `filter`/`mapcat`/`map`, no intermediate seq.
  `rt_into_map` also fuses `(into to (for [x coll] body))` (the minimal `for`
  expands to `map`) and, uniquely, realizes lazy `coll` sources such as
  `range` natively so `(into {} (for [i (range n)] …))` avoids the interpreter
  end to end.
- `rt_repeatedly(n, f)` — `n` a non-negative `Long`: invoke `f` exactly `n`
  times into a `Vector` (finite, so equivalent to the lazy seq for the eager
  consumers it feeds).

Each falls back to `call_global_fn("clojure.core", …)` for inputs it cannot
walk directly, preserving full semantics.
- **Output:** `rt_println(v)`, `rt_pr(v)`, `rt_str(v)`
- **Type checks:** `rt_is_nil`, `rt_is_vector`, `rt_is_map`, `rt_is_seq`, `rt_identical`
- **Linker anchor:** `anchor_rt_symbols()` — call from harness to prevent dead-code elimination
- **Specialization & inline caches (Phase 10.6):**
  `rt_value_tag(v) -> i64` (tag classes `TAG_LONG`/`TAG_DOUBLE`/`TAG_BOOL`/`TAG_NIL`/`TAG_OTHER`,
  `pub const`s) — entry-guard type test; `rt_unbox_long(v) -> i64` / `rt_unbox_double(v) -> f64` —
  payload extraction after a successful guard; `rt_box_bool(u8)` — interned bool boxing for
  unboxed `i8` booleans; `rt_gas_charge(cost)` — charges the active weighted
  basic-block meter, unwinds native bump regions, and tells generated code to
  return early on exhaustion; generated user-call sites perform a zero-cost
  sticky check immediately after return so a failed callee cannot continue in
  its caller with a bogus nil value;
  `rt_deopt()` — counts a guard failure and returns the deopt sentinel
  (a `Box::leak`ed non-GC address; `deopt_sentinel_addr() -> usize` exposes it to the dispatch
  seam via a `cljrs_runtime::tiered::jit_state` hook); `rt_kw_ic_fill(ptr, len, slot)` — keyword-constant
  inline-cache fill: interns the keyword into a permanently rooted global table and stores the
  stable pointer into the call site's data slot (`rt_const_keyword` itself now interns too);
  `rt_call_ic(callee, args, nargs, slot)` — `rt_call` with a per-call-site protocol-dispatch
  inline cache keyed `(ProtocolFn identity, dispatch type-tag, protocol generation)`, falling
  through to `rt_call` for non-protocol callees.  Cached values (interned keywords, impl fns)
  are kept alive by an IC root tracer registered per allocating thread; IC slots in compiled
  modules hold only indices/interned pointers, never GC roots.
- **Slash-named builtins:** when `(ns, name)` resolution fails, `rt_load_global` retries the
  joined `"{ns}/{name}"` in the current namespace (reaching clojure.core through its refers),
  mirroring `eval_symbol` — so `Math/abs`-style builtins, which the lowerer splits into
  `(ns="Math", name="abs")`, resolve in compiled code instead of yielding nil (which turned
  into "not callable: <nil> is not callable" at the first call).
- **Versioned symbols:** `rt_load_global` detects a `name@<sha>` suffix and resolves it through
  the shared `cljrs_runtime::env::versioned` resolver (lazily loading the immutable `ns@sha` namespace;
  resolution failures surface as pending exceptions); lookups into a not-yet-loaded `ns@sha`
  namespace trigger the same lazy load.  `rt_load_global_versioned_ic(ns, ns_len, name,
  name_len, slot)` is the fast path emitted by codegen (`emit_load_global_versioned_ic`):
  versioned bindings are immutable, so the per-call-site slot is filled once with a permanently
  rooted boxed value (the `VERSIONED_IC` table, traced by the same IC root tracer) and never
  invalidated.
  `jit_stats` module — relaxed diagnostic counters (`BOXED_ARITH_CALLS`, `GUARD_DEOPTS`,
  `KW_IC_FILLS`, `PROTO_IC_HITS`, `PROTO_IC_MISSES`) and `jit_stats::snapshot() -> String`
  (written by `cljrs --jit-stats`).
- **JIT seams (safe Rust, not `extern "C"`):**
  `take_pending_exception_value() -> Option<Value>` — take + clear the thread's pending
  exception as an owned `Value`; the JIT dispatch seam calls it right after native code
  returns, so an uncaught `(throw …)` propagates as `EvalError::Thrown` instead of a nil
  return.
  `deopt_sentinel_addr() -> usize` — address of the pointer a failed entry guard returns.
  Closure escape: `rt_make_fn`, `rt_make_fn_variadic`, and `rt_make_fn_multi` call
  `jit::code_cache::pin_epoch` directly (via `notify_closure_escape`) whenever they wrap a
  compiled function pointer into a GC-managed closure value, pinning the executing module's
  reclamation epoch.  A no-op under AOT, where there is no executing JIT frame and code is
  never unloaded.

### Cranelift codegen (`codegen.rs`)

```rust
// Generic over any cranelift_module::Module backend (defaults to ObjectModule for AOT).
pub struct Compiler<M: Module = ObjectModule> { ... }

// Works with any backend:
impl<M: Module> Compiler<M> {
    // param_count must be IrFunction::abi_param_count() — it includes the
    // hidden trailing region parameter of region-parameterised variants.
    pub fn declare_function(&mut self, name: &str, param_count: usize) -> CodegenResult<FuncId>;
    pub fn compile_function(&mut self, ir_func: &IrFunction, func_id: FuncId) -> CodegenResult<()>;
    // Phase 10.6: per-parameter type specializations (entry guards + unboxing);
    // compile_function delegates here with empty specs.
    pub fn compile_function_with_specs(&mut self, ir_func: &IrFunction, func_id: FuncId, specs: &[Repr]) -> CodegenResult<()>;
    pub fn into_inner_module(self) -> M;        // JIT: reclaim the module after compiling
    pub fn last_code_size(&self) -> u32;        // machine-code bytes of the last compiled fn (JIT memory accounting)
}

// AOT-specific (ObjectModule only):
impl Compiler<ObjectModule> {
    pub fn new() -> CodegenResult<Self>;
    pub fn finish(self) -> Vec<u8>;
}

// Entry point for JIT and other backends that supply their own Module:
pub fn new_compiler_from_module<M: Module>(module: M, ptr_type: types::Type) -> CodegenResult<Compiler<M>>;
```

### Type inference (`typeinfer.rs`, Phase 10.6)

```rust
pub use cljrs_ir::Repr; // { Boxed, Long, Double, Bool } — defined in cljrs-ir, re-exported here
pub fn infer(func: &IrFunction, specs: &[Repr]) -> HashMap<VarId, Repr>;
```

`Repr` now lives in `cljrs-ir` so `IrFunction` can carry static representation
seeds from `^long`/`^double` type hints; `typeinfer` re-exports it unchanged.
`infer` seeds parameters from `specs` and `let`/`loop` locals from
`func.local_seed_reprs` (folded through `meet`, so a hint never unsoundly
forces a boxed-producing binding into an unboxed register).
`compile_function_with_specs` merges `func.seed_reprs` (static hints, which win)
with the caller's profiled `specs` before driving both the prologue guards and
inference, so a `^long`-hinted parameter is guarded/unboxed without waiting for
the Tier-1 profiling warmup.

Forward fixpoint dataflow over the CFG (including `RecurJump` back-edges into
loop-header phis).  Parameters are seeded from `specs`; constants and the
arithmetic/comparison `KnownFn`s propagate; phis meet (mixed reprs fall back to
`Boxed`).  A var gets an unboxed repr only where codegen can emit semantics
matching the boxed rt_abi bridge: wrapping `unchecked-*`, f64 arithmetic for
mixed numeric operands, and ordered float compares. Promoting long `+`/`-`/`*`
stay boxed because overflow can produce BigInt.
`Div`/`Rem` and cross-type `Eq` always stay boxed.

### JIT tier (`jit/`)

In-process compilation of hot arities to native code, over the same
`codegen`/`typeinfer`/`rt_abi` as AOT.  A function reaches it through the
tiers: tree-walk → background-lowered IR (Tier 1) → JIT-native (Tier 2).

```rust
pub struct Jit;                                 // the JitBackend a runtime dispatches through
pub fn install(runtime: &Runtime);              // attach a JIT to one runtime
pub fn install_on(globals: &Arc<GlobalEnv>);    // same, for a caller holding the environment

pub mod code_cache {
    pub fn mark_stale(epoch: u64);              // supersede a module (reclaimed at the next STW)
    pub fn reclaim_at_stw() -> usize;           // free stale, unpinned modules with no live frame
    pub fn live_count() -> usize;               // diagnostics
    pub fn stale_count() -> usize;
    pub fn reclaimed_count() -> u64;
    pub fn reclaimed_bytes() -> u64;
}
```

`install` registers a `Jit` on the runtime's `JitState`
(`cljrs_runtime::tiered::backend::JitBackend`) — enqueue, OSR enqueue,
mark-stale, pending-exception, deopt-sentinel, and async-compile in one
object, per runtime.  The first call also creates the process-wide pieces:
the compile queue, the `cljrs-jit-worker` thread, the var-rebind hook, and
the stop-the-world reclaim hook.  All per-arity state (counters, argument
profiles, published pointers, OSR entries) belongs to the runtime, not to
this module; this module owns the compiled machine code.

Everything else here is crate-private: `jit_compiler::compile_jit` (and
`compile_jit_poll` for `^:async` state machines), `jit_worker`, and
`async_jit`.

Environment: `CLJRS_JIT_THRESHOLD` (Tier-1 calls before compiling, default
1000), `CLJRS_OSR_THRESHOLD` (loop back-edges before an OSR-entry compile),
`CLJRS_JIT_NO_SPEC=1` (compile generically), `CLJRS_JIT_DEOPT_LIMIT` (entry-guard
failures tolerated before a specialization is discarded).

### Extensions and the compile session (`extensions.rs`)

The compiler does not decide what a compiled program contains.  A host — the
`cljrs` CLI from its enabled features and project configuration, or an
embedding application — describes each optional extension generically and
hands the set over:

```rust
pub struct Extension {
    pub crate_name: &'static str,        // harness [dependencies] entry, e.g. "cljrs-io"
    pub register: fn(&Arc<GlobalEnv>),   // compile-time registration (macro expansion)
    pub init_path: &'static str,         // same fn, by name, for generated harness source
}

pub struct ExtensionSet { /* ordered */ }
impl ExtensionSet {
    pub fn new() -> Self;
    pub fn with(self, e: Extension) -> Self;      pub fn push(&mut self, e: Extension);
    pub fn register_all(&self, globals: &Arc<GlobalEnv>);
    pub fn crate_names(&self) -> Vec<&'static str>;
    pub fn harness_init_code(&self) -> String;
    pub fn iter(&self) -> Iter<'_, Extension>;    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
}

pub struct CompileSession { /* src_dirs, extensions, rust_config, signatures, opacity */ }
impl CompileSession {
    pub fn new(src_dirs: Vec<PathBuf>, extensions: ExtensionSet) -> Self;
    pub fn rust_config(self, Option<RustConfig>) -> Self;
    pub fn verify_commit_signatures(self, bool) -> Self;
    pub fn opacity(self, OpacityPolicy) -> Self;
    // accessors: src_dirs, extensions, rust_config_ref,
    //            verifies_commit_signatures, opacity_policy, add_src_dir
}
```

`compile_file` and `compile_file_to_wasm` take a `&CompileSession` and call
`register_all` on the bootstrap environment (so `require` resolves during
macro expansion) and `harness_init_code` when generating the harness (so the
produced binary registers the same namespaces).  `crate_names` are merged into
the harness `[dependencies]`, deduplicated against the base set.

Consequently this package does **not** depend on `cljrs-io`, `cljrs-net`,
`cljrs-charset`, or `cljrs-base64` — a compiler build never compiles the
network stack.  `cljrs-async` remains a dependency because `codegen` and
`rt_abi` implement its state-machine poll ABI; registering
`clojure.core.async` is still the host's decision.

### AOT driver (`aot.rs`)

```rust
pub fn compile_file(src_path: &Path, out_path: &Path, src_dirs: &[PathBuf], rust_config: Option<&RustConfig>, verify_commit_signatures: bool, opacity: OpacityPolicy) -> AotResult<()>;
pub fn compile_file_to_wasm(src_path: &Path, out_path: &Path, src_dirs: &[PathBuf]) -> AotResult<()>;
pub fn compile_test_harness(test_dir: &Path, out_path: &Path, src_dirs: &[PathBuf]) -> AotResult<()>;
pub fn lower_via_clojure(name: Option<&str>, ns: &str, params: &[Arc<str>], forms: &[Form], env: &mut Env) -> AotResult<IrFunction>;
pub fn audit_source(interpreted_source: &str, compiled_namespaces: &[CompiledNamespace], versioned_bundled: &[(Arc<str>, String)]) -> SourceAudit;

pub enum AotError { Io, Parse, Codegen, Eval, Link, Wasm(WasmError), SourceEmbedded(SourceAudit), NoGcBlacklist(Vec<BlacklistViolation>) /* no-gc only */ }

pub struct CompiledNamespace { pub ns: Arc<str>, pub init_symbol: Option<String>, pub preamble: String }
pub struct SourceLeak { pub channel: SourceLeakChannel, pub ns: Option<String>, pub bytes: usize }
pub enum SourceLeakChannel { EntryPreamble, NamespacePreamble, BundledNamespace }

pub struct SourceAudit { /* private */ }
impl SourceAudit {
    pub fn leaks(&self) -> &[SourceLeak];
    pub fn is_clean(&self) -> bool;
    pub fn total_bytes(&self) -> usize;
    pub fn verdict(&self, policy: OpacityPolicy) -> OpacityVerdict;
}

pub enum OpacityPolicy { Report /* default */, RequireFullyCompiled }

pub struct BundleAudit;                 // what the wasm module would omit
impl BundleAudit {
    pub fn of(omissions: impl IntoIterator<Item = Omission>) -> Self;
    pub fn omissions(&self) -> &[Omission];
    pub fn is_complete(&self) -> bool;
    pub fn count(&self) -> usize;
    pub fn verdict(&self, policy: OpacityPolicy) -> OpacityVerdict;
}
pub enum OmissionKind { Namespace, EntryForm }
pub enum OpacityVerdict { Clean, Tolerated, Rejected }
```

### Source-embedding audit (`--require-fully-compiled`)

Only plain `defn` bodies reach machine code. Forms that `needs_interpreter`
reports - `ns`, `require`, `defmacro`, `defonce`, `defprotocol`, `defrecord`,
`defmulti`, `defmethod`, `extend-type`, `extend-protocol` - are written to the
harness as `.cljrs` files and pulled in with `include_str!`, method bodies
included, even when the enclosing namespace compiles successfully. A namespace
that fails lowering or codegen falls back to source the same way, and pinned
versioned dependencies always do.

Four steps, each usable on its own:

| step | function | purity |
|---|---|---|
| collect | `embedded_fragments` - the source-carrying channels as raw `(channel, ns, text)` | pure |
| promote | `audit_source` - fragments to a `SourceAudit`, empties dropped | pure |
| decide  | `SourceAudit::verdict(OpacityPolicy)` | pure |
| apply   | `compile_file` - errors, warns, or proceeds | effectful |

A new source-carrying channel is one entry in `embedded_fragments`; a new
policy is one `OpacityPolicy` variant. `OpacityPolicy::Report` is the default
and preserves existing behaviour.

`SourceAudit` covers the native backend. The wasm backend embeds no source at
all, so it falls short the other way: a namespace it cannot lower is skipped
and an entry form needing the interpreter is filtered out of `__cljrs_main`,
both left for an IR-interpreter tier that is not wired up yet. `BundleAudit`
collects those omissions during `lower_file_to_ir_bundle` and
`compile_file_to_wasm` applies the SAME `OpacityPolicy` to them, so
`--require-fully-compiled` means "the artifact fully represents the program"
on both backends and each reports its own way of falling short.

`compile_test_harness` (`--test`) has no audit and cannot have one: it bundles
every test namespace as interpreted source unconditionally, so a strict policy
could never be satisfied. The CLI refuses that combination
(`resolve_opacity_policy` in `crates/cljrs/src/main.rs`).

Property tests: `tests/source_leak_audit.rs`; end-to-end:
`require_fully_compiled_{rejects_embedded_source,accepts_plain_defn}` in
`tests/aot_e2e.rs`.

`compile_file_to_wasm(src, out, src_dirs, opacity)` is the `cljrs compile
--target wasm` entry point: it lowers
the source **and its transitively-required user namespaces** to a bundle of IR
functions (`lower_file_to_ir_bundle`: entry `__cljrs_main` + one
`__cljrs_ns_init_N` per lowerable required namespace, mirroring `compile_file`'s
`discover_bundled_sources`/`lower_namespace`), rewrites same-unit calls to
`CallDirect` (`optimize_direct_calls`), then drives `wasm::compile_bundle` over
every function + its flattened subfunctions and writes the validated module. A
namespace the backend can't lower is skipped (left for the runtime's
IR-interpreter tier), the same graceful degradation the native path uses; the
memory/table bases are `WasmLayout`-configurable (`Default` = the `0`
placeholders). The emitted module's `"rt"` imports are satisfied by the runtime
built for `wasm32-unknown-unknown`; making the runtime *export* that surface,
instantiating the AOT module against it (passing the runtime's reserved bases
through `WasmLayout`), and wiring the IR interpreter in as the dynamic-code tier
is the remaining runtime-side step.

Pipeline: read source → parse → evaluate preamble → macro-expand → pin versioned references → discover required namespaces → **compile each required namespace** (`lower_namespace`: preamble/body partition + ANF lower) → ANF lower entry (Rust, `cljrs_ir::lower`) → optimize (escape analysis + region alloc) → **[no-gc] blacklist check** → Cranelift codegen (entry + per-namespace initializers) → **compile `^:async` poll functions** → generate Cargo harness → `cargo build --release` → copy binary.

**Async activation (Phase H):** `compile_async_poll_fns` introspects the
`^:async` fns the program defined (their `def` forms are evaluated into the
compile-time env first), lowers each arity to a state machine (`is_async`, no
region pass — a region scope can't span a suspend), compiles a poll function
(`declare_poll_function`) into the same object module, and records
`(ns, name, arity, symbol, n_slots)`. The harness declares each symbol `extern
"C"` and calls `cljrs_async::state_machine::register_poll_fn_named` after
`cljrs_async::init`, so `^:async` dispatch runs native. Unsupported bodies
(channels/spawn/`throw`/regions), capturing closures, and fns with inner
closures fall back to the `eval_async` tree-walker.

**Versioned namespaces are snapshotted at compile time.** Versioned requires
execute during expansion (fetching the pinned source from git); a discovery
pass (`pin_versioned_references`) additionally walks the expanded program for
bare versioned symbols (`mylib/foo@<sha>`) and force-loads each pin via
`cljrs_runtime::env::versioned::pin_if_available`.  Every pinned source fetched this
way is embedded in the binary under its versioned namespace name
(`register_builtin_source("mylib@<sha>", …)`), so the produced binary is
self-contained — the generated harness calls
`globals.set_versioned_offline(true)`, and a versioned namespace that was not
embedded fails with a clear error instead of attempting a git fetch.  A bad
pin (missing commit, failed signature check) fails the *compile*.  When
`verify_commit_signatures` is set, native PGP/SSH signature verification (against
the project's `:trusted-signers`) runs at compile time; the binary trusts its
embedded sources.

The generated harness `main()` calls `-main` (via `resolve`) after
`__cljrs_main` returns, forwarding all command-line arguments (skipping the
program name) as individual strings.  If `-main` is not defined the binary
exits normally; if `-main` throws, the binary prints the error and exits 1.

The generated harness `main()` (and the `compile_test_harness` test runner)
calls `cljrs_gc::dump_stats_from_env()` once at exit, so AOT binaries honor
the `CLJRS_GC_STATS` env var (empty/`"-"` → stdout, otherwise a file path).

**Test harness (`cljrs compile --test`).** `compile_test_harness` compiles a
directory of clojure.test namespaces (plus every namespace found on
`src_dirs`) to a standalone test-runner binary.  Each **source** namespace —
the code under test — is loaded into a compile-time environment and
AOT-compiled with the same per-namespace pipeline `compile_file` uses for
required namespaces (`lower_namespace`): top-level forms are partitioned into
an interpreted preamble (`ns`/`require`, `defmacro`, and anything else the
backend can't lower) and a compilable body that is Cranelift-compiled to a
`__cljrs_ns_init_*` initializer.  All initializers share one object module,
linked into the harness binary; the generated `main()` registers each source
namespace's loader via `register_compiled_ns_loader`, so `require` runs the
preamble plus the native initializer.  A source namespace that fails to load,
lower, or codegen at compile time falls back to being bundled as interpreted
source (`register_builtin_source`), and a load failure at runtime is reported
to stderr without aborting the remaining namespaces.

**Test** namespaces are always bundled as interpreted source, never lowered:
`deftest` expands to `alter-meta!`, which the backend can't lower, so every
test would land in the interpreted preamble anyway — and the full
`macroexpand_all` a lowering pass requires is prohibitively slow on
macro-heavy test files (`are`/`is`/`testing` trees; on the 235-namespace
`clojure-test-suite` it takes hours).  Calls from interpreted tests into
compiled source namespaces still dispatch to native code, which is where AOT
pays off.  The runner calls `clojure.test/run-tests` per test namespace
(unloading each afterwards to bound peak memory), prints an aggregate
summary, and exits 1 on any failure or error.  End-to-end coverage lives in
`tests/test_harness_e2e.rs` (gated behind `aot_full_test`).

**Harness dependency resolution.** The harness depends on the runtime crates,
and `resolve_harness_deps()` decides *how*, independently of the current
directory — so `cljrs compile` works on a bare `.cljrs` file with no
surrounding `Cargo.toml`, inside an unrelated Cargo workspace, and from a
`cargo install cljrs` binary with no checkout at all:

- **Local checkout found → path deps** (`path = "<workspace>/crates/cljrs-*"`),
  and the build runs `--offline`. `find_workspace_root()` locates the checkout
  via, in order: (1) the `CLJRS_WORKSPACE_ROOT` env var (explicit override;
  must point at a `Cargo.toml` with `[workspace]`); (2) the compiler crate's
  compile-time `CARGO_MANIFEST_DIR` (`<workspace>/crates/cljrs-compiler`, so the
  root is two levels up); (3) walking up from the current directory.
- **No checkout → published deps** (`cljrs-* = "=<version>"`, pinned to this
  `cljrs`'s own `CARGO_PKG_VERSION`, since the runtime crates share the
  workspace version and publish in lock-step). The build is **not** `--offline`,
  so Cargo may fetch the crates from crates.io. This is what makes
  `cargo install cljrs` + `cljrs compile` self-sufficient (a Rust toolchain and
  network access are still required at compile time).

Setting `CLJRS_WORKSPACE_ROOT` forces path deps against that clone even from an
installed binary.

### No-GC blacklist (`escape.rs`, no-gc only)

```rust
pub enum BlacklistViolation { InteriorPointerReturn { .. }, RegionToStaticStore { .. }, LazySeqEscape { .. }, EscapingClosure { .. } }
pub fn check(func: &IrFunction) -> Vec<BlacklistViolation>;
pub fn check_function(func: &IrFunction) -> Vec<BlacklistViolation>;
```

Detects four classes of no-gc memory-safety violations in IR functions:
1. **InteriorPointerReturn** — return var is (transitively via phi) an allocation from the function's scratch region.
2. **RegionToStaticStore** — allocation result flows into `DefVar` / `SetBang` without the static context.
3. **LazySeqEscape** — lazy-producing call result is bound as an intermediate and returned unrealized.
4. **EscapingClosure** — `AllocClosure` stored in a static container.

Multi-file support: when the source file uses `(ns ... (:require [...]))`, the
required namespaces are loaded during compilation (discovered from `src_dirs`)
and **each is AOT-compiled into the same object module** — not bundled as
source and interpreted at startup. `lower_namespace` parses and macro-expands
each required namespace, partitions its top-level forms into an interpreted
preamble (`ns`/`require`, `defmacro`, protocol/multimethod forms) and a
compilable body, and lowers the body to an `__cljrs_ns_init_<i>` function. The
harness writes each namespace's preamble to `src/ns_<i>_preamble.cljrs`,
declares its initializer `extern "C"`, and registers a `CompiledNsLoader`
(`globals.register_compiled_ns_loader`) so that when `require` resolves the
namespace at runtime, `cljrs_runtime::env::loader::do_load` runs the loader — evaluating
the preamble, then calling the compiled initializer — instead of tree-walking
source. Transitive `require`s resolve naturally: a namespace's preamble
contains its own `ns`/`require` form, which triggers loading of its
dependencies (each via its own compiled loader) before its initializer runs.
Pinned *versioned* sources (`mylib@<sha>`) are the exception — they still embed
as interpreted builtin source, since they resolve through the separate
versioned loader rather than the plain `require` path.

**Structural entry namespace:** the *entry* file's own `ns` form is normally an
interpreted form too, so it ships as readable text and `--require-fully-compiled`
rejects the program. When that `ns` is the **only** interpreted form and carries
no clause richer than `:require` — no `:import`, `:gen-class`, docstring or
`:refer-clojure` — `structural_ns_form` recognises it and the harness establishes
the namespace directly instead: `get_or_create_ns`, `refer_core`, `sync_star_ns`
(so `(resolve '-main)` looks in the entry namespace rather than `user`), then one
`load_ns` call per require. No source text is emitted, so a single-file program
can pass the opacity gate.

Require specs are parsed by `cljrs_runtime::interp::special::parse_require_spec_form`
— the same function `eval_ns` uses — and emitted whole, alias and `:refer` and
`@version` included. This path must not re-derive spec parsing: aliases are
resolved at compile time by `qualify_aliases`, but `:refer` is not. A bare `f`
lowers to `LoadGlobal(<entry-ns>, "f")` and is resolved through the namespace's
refer table at *runtime*, and `rt_call` on a nil callee returns nil rather than
erroring — so a dropped refer produces a program that runs and prints wrong
answers. A dropped `@version` likewise routes a pinned require to the plain
loader instead of the versioned one.

The structural path is taken only when the `ns` is the *first* interpreted form;
otherwise it is restored at the head of the preamble, which would reorder the
program if anything interpreted had preceded it. Any other interpreted form
present at all forces the textual preamble, since that form may depend on the
aliases and refers the `ns` installs. Required namespaces still ship their own
`ns`/`require` preambles as text, so multi-file programs remain rejected by
`--require-fully-compiled` through the `NamespacePreamble` channel.

---

## Features

| Feature | Default | Effect |
|---------|---------|--------|
| `wasm-aot` | on | The WebAssembly backend (`wasm/`), `aot::compile_file_to_wasm`, and the `wasm-encoder` dependency.  `--no-default-features` produces a native-only compiler. |
| `no-gc` | off | Propagated to `cljrs-gc`/`cljrs-value`/`cljrs-runtime`/`cljrs-stdlib`/`cljrs-async`; enables the `escape.rs` blacklist analysis. |
| `aot_full_test` | off | Runs the full (~120 test) AOT end-to-end suite instead of its core subset. |
| `regex-full` | on | Forwards `regex-full` to `cljrs-value`/`cljrs-runtime`/`cljrs-async`/`cljrs-stdlib` — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`; see [cljrs-value's README](../cljrs-value/README.md#features). |
| `deps` | on | Pass-through for `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |

All four workspace dependencies above are taken with default features off (see the
note in the root `Cargo.toml`), so `regex-full` and `deps` are what put back what
their defaults used to provide; both are in `default`, leaving plain builds
unchanged. Selecting `small-regex` means `default-features = false` on *this*
crate — a second direct dependency cannot undo a feature another edge enabled.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `cljrs-ir` (workspace) | IR types: `IrFunction`, `Block`, `Inst`, `KnownFn`, etc. |
| `cljrs-gc` (workspace) | `GcPtr<Value>` — GC interaction |
| `cljrs-value` (workspace) | `Value`, collections, `NativeFn` — value types referenced by IR and rt_abi |
| `cljrs-reader` (workspace) | `Form`, `FormKind` — input AST for lowering |
| `cljrs-runtime` (workspace) | `Runtime::builder` — bootstrap environment for macro expansion + harness; `env::{Env, GlobalEnv}`, `interp` macro expansion, `env::callback::invoke` and `env::apply::{type_tag_of, type_tag_matches}` for rt_call dispatch and protocol IC tag validation; `tiered` lowering |
| `cljrs-stdlib` (workspace) | `install` — stdlib namespaces in that environment |
| `cranelift-*` (workspace) | Cranelift compiler infrastructure (`cranelift-object` for AOT, `cranelift-jit` for the `jit/` tier) |
| `tracing` (workspace) | `tracing::debug!(target: "jit", …)` — JIT tier diagnostics |
| `cljrs-async` (workspace) | `state_machine` — the poll ABI `codegen` and `rt_abi` implement.  An ABI dependency, not a product extension |
| `cljrs-project` (workspace) | `config::RustConfig` — the user's `:rust` crate configuration, carried in `CompileSession` |
| `cljrs-io`/`-net`/`-charset`/`-base64` | **dev-dependencies only** — extensions the end-to-end tests supply the way a host does |
| `target-lexicon` (workspace) | Target triple detection |
