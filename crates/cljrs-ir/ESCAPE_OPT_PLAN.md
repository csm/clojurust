# Escape-analysis optimization plan

This document records the current state and the remaining implementation stages
for cross-function bump (region) allocation.  It exists so that a new session
can pick up the work cold, without needing the original design conversation.

---

## Background

The optimizer (`lower/optimize.rs`) already promotes **function-local**
non-escaping allocations to region (bump) allocation.  The gap is allocations
that are *returned* from a callee: the current analysis immediately marks any
allocation that reaches a `Return` terminator as `EscapeState::Escapes`
(`escape.rs:541–543`), even if the caller never lets the value escape.

The goal of the remaining stages is to eliminate GC allocations for values
that don't escape the *calling* function, not just the *allocating* function.

---

## Stage 1 — Inlining  ✅ DONE  (`lower/inline.rs`)

**What it does.** For small, non-capturing callees (≤ 20 instructions, no
`LoadLocal`, no subfunctions) resolved via `LoadGlobal → defn_map → registry`,
the pass splices the callee body into the call site before escape analysis
runs.  After inlining the allocation is local to the caller; escape analysis
sees it as `NoEscape` and the region optimizer promotes it.

**Key detail — calling convention.** Every arity function has a leading
self/closure param (`params[0]`), then the user params.  `do_inline` maps
`params[0]` → `callee_var` (the closure object at the call site) and
`params[1..]` → `args`.

**Limitations.**
- Functions exceeding the threshold are not inlined.
- Functions called from many sites (hot polymorphic callees) stay un-inlined
  to avoid code bloat.
- Both limitations are addressed by stages 2–4 below.

---

## Stage 2 — `Returns` state for allocations  ✅ DONE  (`lower/escape.rs`)

**What it does.**  `classify_escape_with_ctx` no longer short-circuits to
`Escapes` when an allocation reaches a `Return` terminator.  Both
`EscapeMode::Alloc` and `EscapeMode::Param` now join `EscapeState::Returns`,
so the caller can decide whether the value truly escapes.

`compute_return_alloc_summary(ir_func, ctx)` was added as a thin wrapper over
`analyze()` that surfaces the `states` map with the finer-grained
classification.  It is annotated `#[allow(dead_code)]` pending stage-3 usage.

The `returned_vector_escapes` regression test was updated to assert `Returns`
(previously `Escapes`).

**Impact on the optimizer.**  `optimize_regions` filters for `NoEscape` only,
so `Returns` allocations are still skipped — no region promotion yet.  That is
correct: without stage 4 we can't promote them.

---

## Stage 3 — Caller-context propagation

**Goal.** At a call site in the caller, if the return value (`call_dst`) is
`NoEscape` in the caller, then any `Returns`-tagged allocation in the callee
is *transitively* `NoEscape` from the caller's perspective.

**Where to implement.**  In `classify_escape_with_ctx`, the `UnknownCallArg`
arm already does inter-procedural lookup and pushes `call_dst` onto the
worklist when the param summary says `Returns` (escape.rs:589–598).  Add the
symmetric handling for the *return value* itself:

After resolving a `Call(call_dst, callee_var, args)` where the callee is
known, look up the callee's return-alloc summary (from stage 2).  For each
allocation in the callee with state `Returns`, check whether `call_dst` is
`NoEscape` in the caller.  If so, those allocations are transitively
`NoEscape` — record them in a `cross_fn_no_escape: HashMap<Arc<str>,
HashSet<VarId>>` map keyed by callee arity-fn-name.

**Implementation sketch.**
```rust
// In analyze() or a new two-pass variant:
// Pass 1: classify everything in the caller (call_dst may be NoEscape).
// Pass 2: for each Call where call_dst is NoEscape and callee is known,
//         fetch callee's return-alloc summary and mark Returns allocs as
//         NoEscape in a side-channel result.
pub struct AnalysisResult {
    pub states: HashMap<VarId, EscapeState>,
    // NEW: callee arity-fn-name → set of alloc VarIds that are NoEscape
    // because the return value is NoEscape at this call site.
    pub cross_fn_no_escape: HashMap<Arc<str>, HashSet<VarId>>,
    pub uses: HashMap<VarId, Vec<UseInfo>>,
    pub alloc_blocks: HashMap<VarId, BlockId>,
}
```

**Note:** stages 2 and 3 together improve escape information but don't yet
enable region promotion for non-inlined callees.  The allocation still happens
in the callee's stack frame; a region created there would be freed on return,
before the caller can use the value.  Stage 4 solves this.

---

## Stage 4 — Region parameter passing  (hardest)

**Goal.** For call sites where the callee's returned allocation is
`NoEscape` in the caller, pass a region handle as a hidden argument.  The
callee allocates into the caller's region instead of the GC heap.  When the
caller's `RegionEnd` fires, the allocation is freed.

This is the MLKit "region polymorphism" approach.

### 4a — New IR instructions

Add to `Inst` in `lib.rs`:
```rust
/// Receive a region handle passed by the caller (hidden first argument).
/// Emitted in the callee to replace RegionStart when using a caller-provided region.
RegionParam(VarId),

/// Call a function, passing a region handle as a hidden extra argument.
/// Generated by the optimizer at call sites where the return value is NoEscape.
CallWithRegion(VarId, VarId, Vec<VarId>),  // dst, callee, args
                                             // hidden region is implicit
```

Alternatively, encode it as a convention: region-parameterized functions have
an extra `VarId` param appended to their param list, and call sites pass the
region VarId as an extra arg.  This avoids a new `CallWithRegion` instruction
at the cost of convention complexity.

### 4b — Specialization

A callee may be called from sites with different escape behaviour (some
`NoEscape`, some `Escapes`).  Options:

1. **Clone the function** — emit two versions: one with a region param, one
   without.  Rewrite call sites individually.  Simpler but increases code size.

2. **Nullable region param** — single function version; if region is null,
   fall back to GC allocation.  Adds a branch per allocation in the callee.

Start with option 1 (clone) as it is simpler to implement and avoids runtime
branches on the hot path.

### 4c — Optimizer pass (`optimize.rs`)

New pass after `optimize_regions`:
```
fn promote_cross_fn_allocs(ir_func, analysis_result_with_cross_fn_data) -> IrFunction
```

For each `Call(dst, callee, args)` where `dst` is `NoEscape` and the callee
has `Returns`-tagged allocations:
1. Look up or create the region-parameterized variant of the callee.
2. Replace `Call(dst, callee, args)` with `CallWithRegion(dst, callee, args)`
   (or append the region VarId to args).
3. In the callee variant, replace `AllocVector/Map/Set/...` for `Returns`
   allocs with `RegionAlloc(dst, region_param, kind, operands)`.

### 4d — Code generation / interpreter

`cljrs-eval/src/ir_interp.rs`: add dispatch for `RegionParam` (bind the
passed-in region handle) and `CallWithRegion` (push the region onto
`ALLOC_CTX` before the call, pop after).

`cljrs-compiler/src/codegen.rs`: emit the hidden region argument in the
Cranelift call signature.

---

## Sequencing recommendation for a new session

1. **Start with stage 2** — it is a small, self-contained change in
   `escape.rs` and the test update in `escape_regression.rs`.  No new types,
   no new passes.

2. **Then stage 3** — adds a second analysis field; no codegen changes.
   Validates the analysis logic with tests before touching the code-emitting
   path.

3. **Stage 4** is a significant undertaking; approach it incrementally:
   - 4a first (new IR nodes, update `Display`, `Inst::dst()`, `Inst::uses()`
     in `lib.rs`).
   - 4b: implement the clone-based specialization.
   - 4c: the optimizer rewrite pass.
   - 4d: interpreter + codegen.

---

## File map

| File | Relevance |
|------|-----------|
| `crates/cljrs-ir/src/lower/escape.rs` | Core analysis; stages 2 and 3 live here |
| `crates/cljrs-ir/src/lower/optimize.rs` | Region promotion pass; stage 4c goes here |
| `crates/cljrs-ir/src/lower/inline.rs` | Stage 1 (done) |
| `crates/cljrs-ir/src/lib.rs` | IR types; stage 4a adds new `Inst` variants |
| `crates/cljrs-ir/tests/escape_regression.rs` | Regression tests; update for stage 2, add for 3+4 |
| `crates/cljrs-eval/src/ir_interp.rs` | IR interpreter; stage 4d |
| `crates/cljrs-compiler/src/codegen.rs` | Cranelift codegen; stage 4d |
| `crates/cljrs-gc/src/alloc_ctx.rs` | `ALLOC_CTX` thread-local; consulted by stage 4d |
