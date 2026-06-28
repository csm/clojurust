# cljrs-ir

Intermediate representation types shared between the clojurust compiler
(`cljrs-compiler`) and interpreter (`cljrs-eval`).

The IR is a control-flow graph of basic blocks in A-normal form (ANF) with SSA
phi nodes at join points.  Every sub-expression is bound to a named temporary
(`VarId`), and control flow is explicit via `Terminator`s.

**Purpose:** Extracted into its own crate so that both `cljrs-eval` (IR
interpreter, Tier 1 execution) and `cljrs-compiler` (Cranelift codegen, Tier 2)
can depend on the same types without a circular dependency.

---

## File layout

```
src/
  lib.rs  — all IR types: IrFunction, Block, Inst, Terminator, VarId, BlockId,
             KnownFn, Effect, Const, ClosureTemplate, RegionAllocKind,
             SuspendKind.  Async-state-machine instructions (StateStore,
             StateLoad, AsyncSuspend, AsyncResume) and the poll-function markers
             on IrFunction (is_async_poll_fn, async_resume_blocks) live here.
  osr.rs  — OSR-entry transform (Phase 10.4): build_osr_function rewrites a
             function so its entry jumps straight to a hot loop header, with
             live-in values (loop φs + pre-loop defs) arriving as parameters
  lower/
    mod.rs      — re-exports: lower_fn_body, lower_fn_body_destructured,
                  lower_fn_body_seeded, analyze, inline, optimize, EscapeContext …
    async_lower.rs — async state-machine lowering (Phase H): rewrites an
                  `^:async` IrFunction into a non-async poll function
                  (is_async_poll_fn) whose control flow is an explicit resumable
                  state machine.  Splits blocks at each `await`, threads values
                  live across a suspend through the CljxStateMachine's slot array
                  (StateStore/StateLoad), and emits AsyncSuspend/AsyncResume.
                  SSA phi-edge liveness keeps loop-init values out of save sets;
                  resume blocks are phi-free so the codegen dispatch jump needs
                  no phi args.  Lowers `await` only; channels/spawn return
                  AsyncLowerError::Unsupported (interpreter fallback kept)
    anf.rs      — ANF lowering: Form AST → IrFunction (pure Rust).  Closures
                  capture only the enclosing locals their (fully macro-expanded)
                  body references (`collect_symbol_names`, a conservative
                  free-variable over-approximation), not every local in scope.
                  `desugar_pre_post_conditions` rewrites `{:pre [...] :post [...]}`
                  maps at the head of a function body into `(assert ...)` forms
                  (binds `%` to the return value in `:post` conditions).
    context.rs  — LowerCtx builder state used by anf.rs
    escape.rs   — worklist-based escape analysis; inter-procedural via EscapeContext
    inline.rs   — inlining pass: splices small callees into call sites
    known.rs    — symbol → KnownFn resolution
    optimize.rs — region-allocation promotion; dominator/post-dominator CFG analysis
    regionalize.rs — stage-4 cross-function region promotion: clones callees
                     whose `Returns` allocs are NoEscape at a call site, wraps
                     the call site in RegionStart/RegionEnd, rewrites Call →
                     CallWithRegion targeting the cloned variant by name.
                     Also co-promotes allocations reachable only through the
                     returned container (e.g. the inner coordinate vectors of
                     `neighbours`), guarded by a caller-side check that the
                     result is never element-extracted (first/nth/get/peek) or
                     passed to an opaque call
tests/
  capture_minimization.rs — closure-capture set regression tests
  escape_regression.rs    — escape-analysis regression tests
```

---

## Public API

### Core types

```rust
pub struct VarId(pub u32);
pub struct BlockId(pub u32);

/// Machine representation of an IR var (lives here so IrFunction can carry
/// static seeds from `^long`/`^double` type hints; re-exported by
/// cljrs_compiler::typeinfer).  `LongArray`/`DoubleArray` are boxed array
/// pointers with a known element type (from `^longs`/`^doubles`), enabling
/// unboxed `aget`/`aset`.
pub enum Repr { Boxed, Long, Double, Bool, LongArray, DoubleArray }

pub struct IrFunction {
    pub name: Option<Arc<str>>,
    pub params: Vec<(Arc<str>, VarId)>,
    pub blocks: Vec<Block>,
    pub next_var: u32,
    pub next_block: u32,
    pub span: Option<Span>,
    pub subfunctions: Vec<IrFunction>,
    /// Whether this function was declared `^:async`.
    /// Async IR functions fall back to tree-walking `eval_async`; Phase H JIT
    /// will emit Cranelift state machines with explicit resume points.
    pub is_async: bool,
    /// Static per-parameter repr seeds from `^long`/`^double` hints (positional
    /// with `params`).  Empty ⇒ no hints.  Preferred over profiled specs.
    pub seed_reprs: Vec<Repr>,
    /// Static repr seeds for `let`/`loop`-bound locals, keyed by VarId.
    pub local_seed_reprs: Vec<(VarId, Repr)>,
}

impl IrFunction {
    /// True for region-parameterised variants (entry block binds RegionParam):
    /// compiled code receives the caller's region as a hidden trailing param.
    pub fn takes_region_param(&self) -> bool;
    /// Compiled-signature param count: params.len() + the hidden region param.
    pub fn abi_param_count(&self) -> usize;
}

pub struct Block {
    pub id: BlockId,
    pub phis: Vec<Inst>,
    pub insts: Vec<Inst>,
    pub terminator: Terminator,
}
```

### Instructions (`Inst`)

Versioned symbols need no dedicated instruction: the `@<sha>` suffix rides in
the `LoadGlobal` name string, and lowering inside a versioned namespace
(`"base@sha"`) rewrites base-qualified self-references (`base/x`) to the
versioned namespace (see `split_sym` in `lower/anf.rs`).

`Const`, `LoadLocal`, `LoadGlobal`, `LoadVar`, `AllocVector`, `AllocMap`,
`AllocSet`, `AllocList`, `AllocCons`, `AllocClosure`, `CallKnown`, `Call`,
`CallDirect`, `Deref`, `DefVar`, `SetBang`, `Throw`, `Phi`, `Recur`,
`SourceLoc`, `RegionStart`, `RegionAlloc`, `RegionEnd`, `RegionParam`,
`CallWithRegion`

**Async instructions** (Phase H):

- `Await { src, dst }` — yield point inside `^:async` fn; `dst` receives the
  resolved `Future`/`Promise` value.  IR interpreter uses blocking deref;
  `eval_async` yields to the Tokio executor.
- `Spawn { fn_reg, args, dst }` — spawn an `^:async` call as a LocalSet task;
  `dst` receives a `Value::Future` immediately.
- `ChanTake { chan, dst }` — async take from a channel; parks until a value is
  available.
- `ChanPut { chan, val }` — async put into a channel; parks if the buffer is
  full (no result value).

### Terminators

`Jump`, `Branch`, `Return`, `RecurJump`, `Unreachable`

### Known functions (`KnownFn`)

160+ built-in function identifiers with effect classification (`Effect`):
`Pure`, `Alloc`, `HeapRead`, `HeapWrite`, `IO`, `UnknownCall`.

The checked integer arithmetic `Add`/`Sub`/`Mul` throw on overflow at the IR
and compiled tiers (Clojure primitive-long semantics); `UncheckedAdd`/
`UncheckedSub`/`UncheckedMul` are the wrapping counterparts (the `unchecked-*`
family, plus `unchecked-inc`/`-dec`/`-negate` which lower to them).  `inc`/`dec`
lower to checked `Add`/`Sub`.

`CaseEq` is type-strict equality used by the `case` macro (`case=` builtin).
Like Clojure's JVM semantics, `Long` and `BigInt` are interchangeable but mixed
numeric types (`Long` vs `Double`) are never equal.  Always infers `Repr::Bool`.

`Aget`/`Aset`/`Alength` are primitive array access.  On a `^longs`/`^doubles`
operand (`Repr::LongArray`/`DoubleArray`) with an unboxed index, codegen loads/
stores unboxed `i64`/`f64` elements; otherwise it uses a boxed bridge.  All
paths bounds-check and throw on out-of-range access.

`Nth` and `NthLenient` differ only in out-of-bounds behaviour: `Nth` is
user-level `(nth coll idx)` and throws, while `NthLenient` is the
sequential-destructuring nth (`(nth coll idx nil)`) emitted by the lowerer
for `[a b c]` patterns — a short collection binds the missing positions to
`nil` rather than throwing.  Both compile to the `rt_nth` bridge (already
nil-on-OOB); the IR interpreter appends the nil default for `NthLenient`.

Some `KnownFn` variants exist purely for analysis precision — the
codegen and IR interpreter dispatch them through the dynamic builtin
lookup like a regular `Call`, but the analyzer can use them to tighten
escape verdicts.  For example, `Empty?`, `Peek`, `Pop`, `Vec`,
`Mapcat`, `Repeatedly` carry no specialised codegen path; they're
recognised so that the escape analyzer can see through `(empty? coll)`
or `(pop coll)` instead of treating them as opaque `UnknownCall`s.

### Recur and escape analysis

`UseKind::Recur` is *not* treated as an unconditional escape.  When the
analyzer encounters a `Recur` use, it walks to the matching loop-header
`Phi` (positionally aligned with the `RecurJump`'s args) and continues
analysis from the phi's downstream uses.  This is sound because `recur`
is structural control flow — values rebind at the loop header without
leaving the function — and it's what allows a loop-local empty vector
to reach `NoEscape` and get promoted to a region.

### Element extraction aliases its source

`known_fn_arg_escapes` reports that `First`, `Nth`, `NthLenient`, and
`Peek` let arg 0 (the collection) escape into their return value.  This
is because the runtime bridges (`rt_first`/`rt_nth`/`rt_peek`) return an
*interior pointer* into the collection's storage rather than a freshly
boxed element — so the returned value shares the collection's lifetime.
If the extracted element escapes, the collection must be considered
escaping too; otherwise it could be region-promoted and freed while the
interior pointer is still live (use-after-free).  `Get` is exempt: its
bridge clones the value out, so its result does not alias the source.

### Region allocation

`RegionAllocKind`: `Vector`, `Map`, `Set`, `List`, `Cons`

### Closures

`ClosureTemplate`: static description of an `fn*` form (arity info, capture names).

### Optimization pipeline (re-exported from `lower::`)

```rust
/// Inline small, non-capturing callees into their call sites, then promote
/// non-escaping allocations to region (bump) allocation.
pub fn optimize(ir: IrFunction) -> IrFunction;

/// Like `optimize`, but makes previously-lowered defns from other lowering
/// units visible to escape analysis and stage-4 promotion (the script/REPL
/// counterpart of AOT's whole-program tree; supplied by cljrs-eval's defn
/// registry).  Returns the (ns, name) set of externals actually consulted —
/// the caller must invalidate this lowering when any of them is redefined.
pub fn optimize_with_externals(
    ir: IrFunction,
    externals: &[ExternalDefn],
) -> (IrFunction, HashSet<(Arc<str>, Arc<str>)>);

/// One registered cross-unit defn: per-arity registry names (process-unique,
/// never emitted as symbols), callable param counts, variadic flags, and the
/// lowered IR.
pub struct ExternalDefn { pub ns, pub name, pub arity_fn_names,
                          pub param_counts, pub is_variadic, pub arity_irs }

/// Run only the inlining pass (before escape analysis).
pub fn inline(ir: IrFunction) -> IrFunction;
```

Inlining is deliberately *not* externals-aware: splicing another unit's body
into the caller would carry the same redefinition-staleness obligations for a
much larger set of call shapes.

**Pipeline order** inside `optimize`:
1. **Inlining** (`lower::inline`) — resolves `Call` sites whose callee is a
   small, non-capturing, non-variadic `defn` in the same compilation unit and
   splices the callee body into the caller.  Runs up to 8 rounds per function,
   bottom-up.  Threshold: ≤ 20 instructions across all callee blocks.
2. **Escape analysis** (`lower::escape`) — two-pass analysis.  Pass 1
   classifies each allocation as `NoEscape`, `ArgEscape`, `Returns`, or
   `Escapes` (inter-procedural via `EscapeContext`).  Pass 2 (stage-3
   caller-context propagation) identifies callee allocations that are
   transitively `NoEscape` at a specific call site and records them in
   `AnalysisResult::cross_fn_no_escape`, keyed by callee arity-fn-name.
3. **Region promotion** (`lower::optimize`) — rewrites `NoEscape` allocations
   to `RegionStart` / `RegionAlloc` / `RegionEnd` over the minimal CFG
   subgraph that covers the allocation and all its uses.
4. **Cross-function region promotion** (`lower::regionalize`) — for `Call`
   sites whose result is `NoEscape` and whose callee has `Returns`-tagged
   allocations, clones a region-parameterised variant of the callee
   (`<orig>__rgN`) where those allocations become `RegionAlloc` and the entry
   block carries a `RegionParam` binding.  The call site is rewritten to
   `CallWithRegion(dst, target_name, args, region)` — carrying the handle of
   the `RegionStart` the rewrite inserts — and bracketed by
   `RegionStart`/`RegionEnd` over the dom/postdom-LCA scope of `dst`'s uses.
   In compiled code the region travels as a **hidden trailing argument**
   (`IrFunction::abi_param_count` = `params.len() + 1` for such variants;
   `takes_region_param()` detects them), so the callee bump-allocates
   directly into the caller's region without a thread-local lookup; the IR
   interpreter threads the same handle through its per-frame handle map.
   Variants are attached as subfunctions of the calling function so both the
   IR interpreter and codegen can resolve them by name.
   The clone also co-promotes allocations reachable *only* through the
   returned container (e.g. the eight inner `[r c]` vectors stored inside
   `neighbours`' result vector): their lifetime is bounded by the returned
   value, so they live in the same region.  This is gated by a caller-side
   check that the call result is never element-extracted (`first`/`nth`/
   `get`/`peek`) or passed to an opaque call, either of which could keep an
   inner pointer alive past `RegionEnd`.  Note: this sharpens the IR (and
   benefits the tree-walking interpreter, where `AllocVector` is not
   region-aware); the AOT backend already bump-allocates any collection
   built while a region scope is active, so the win there comes from region
   *scope* coverage rather than the per-allocation kind.

### Analysis (re-exported from `lower::`)

```rust
pub fn analyze(ir: &IrFunction, ctx: Option<&EscapeContext>) -> AnalysisResult;
pub fn make_analysis_context(ir: &IrFunction) -> EscapeContext;

pub enum EscapeState { NoEscape, ArgEscape, Returns, Escapes }
pub struct UseInfo { pub block: BlockId, pub kind: UseKind }
pub enum UseKind { Return, DefVar, SetBang, ClosureCapture, Throw,
                   StoredInHeap, Recur, KnownCallArg{..}, UnknownCallArg{..},
                   PhiInput, BranchCond, Deref, CallCallee }
pub struct AnalysisResult {
    pub states:            HashMap<VarId, EscapeState>,
    // Stage-3: callee arity-fn-name → callee alloc VarIds that are
    // transitively NoEscape because the call result is NoEscape here.
    pub cross_fn_no_escape: HashMap<Arc<str>, HashSet<VarId>>,
    pub uses:              HashMap<VarId, Vec<UseInfo>>,
    pub alloc_blocks:      HashMap<VarId, BlockId>,
}
```

These are the same types the optimizer uses internally; they are exposed
publicly so downstream tooling (e.g. `cljrs-ir-viz`) can present
escape-analysis results without re-implementing the use-chain walk.

### OSR-entry construction (`osr` module, Phase 10.4)

```rust
/// Cap on OSR parameters (the JIT dispatch shim supports 8 native args).
pub const MAX_OSR_PARAMS: usize = 8;

pub struct OsrFunction {
    /// Entry block jumps to the loop header; loop state arrives as params.
    pub func: IrFunction,
    /// For each param (in order), the original VarId whose current value the
    /// interpreter must pass when transferring into the native frame.
    pub live_ins: Vec<VarId>,
}

/// Build the OSR-entry variant of `orig` for the loop header `header`
/// (a `RecurJump` target).  Keeps only blocks reachable from the header;
/// header φs get a new incoming edge from the fresh entry block fed by fresh
/// parameters (loop variables), other live-ins become parameters bound to
/// their original VarIds.  `RegionEnd`s whose `RegionStart` executed before
/// the loop (i.e. in the interpreter) are dropped — the interpreter frame
/// closes those regions after the transfer returns.  Errs (caller stays at
/// Tier 1) if the header is unknown or live-ins exceed MAX_OSR_PARAMS.
pub fn build_osr_function(orig: &IrFunction, header: BlockId) -> Result<OsrFunction, String>;
```

### Source mapping

ANF lowering emits `Inst::SourceLoc(span)` markers at the head of each
form's lowering, deduplicated per `(file, line)` within a basic block.
`SourceLoc` has no `dst` and `Effect::Pure`, so it is invisible to the
optimizer and codegen — it exists for downstream tooling only.

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span` type for source locations |
