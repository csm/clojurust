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

## Stage 4 — Region parameter passing  ✅ DONE  (`lower/regionalize.rs`)

**What it does.** For each `Call(dst, callee, args)` site where `dst` is
`NoEscape` in the caller and the callee has at least one region-promotable
`Returns`-tagged allocation, the pass:

1. **Specialises** the callee (clones it under name `<orig>__rgN`,
   rewrites the targeted `AllocVector/Map/Set/List/Cons` instructions into
   `RegionAlloc(dst, region_var, kind, operands)`, and inserts a
   `RegionParam(region_var)` instruction at the entry block's prologue).

2. **Rewrites the call site**: `Call(dst, callee, args)` becomes
   `CallWithRegion(dst, target_name, args)`, bracketed by `RegionStart(rv)`
   at the LCA-block (in the dominator relation of `dst`'s use-blocks plus
   the call's defining block) and `RegionEnd(rv)` at the LCA-block in the
   post-dominator relation.  The dom/postdom logic is shared with the
   local region-promotion pass (`optimize.rs`) — same back-edge / throw
   guards apply.

3. **Attaches the clone** as a subfunction of the *calling* function (not
   root) so that both the IR interpreter (which resolves `CallWithRegion`
   targets via `ir_func.subfunctions`) and the AOT codegen (which
   recursively walks the IR tree and registers every subfunction in
   `user_funcs`) can find the variant by name without any global registry
   plumbing.

**Calling convention.** The "hidden region argument" envisioned by the
original plan is implicit via the thread-local region stack already used
by `RegionAlloc`.  The caller's `RegionStart` pushes a region; the
callee's `RegionAlloc`s see it via `try_alloc_in_region` (in the
interpreter) or `rt_region_alloc_*` (in compiled code).  No new
parameter is added to the callee's signature, so call ABI for
`CallWithRegion` is identical to `CallDirect`.

`RegionParam(VarId)` is therefore mostly a structural marker: the bound
VarId is referenced as the (ignored) `region` operand of subsequent
`RegionAlloc` instructions in the callee body, while the runtime simply
binds it to nil.

**Specialisation memoisation.** Variants are keyed by `(caller_path,
callee_fn_name, sorted_returns_alloc_VarIds)` so that two call sites in
the same caller calling the same callee with the same alloc-set share a
single clone.

**Limitations / non-goals (deferred to later work):**
- Only call sites whose callee resolves via `LoadGlobal → defn_map →
  registry` are considered.  Closure-typed callees with non-empty
  captures aren't rewritten because the cloned variant needs the same
  capture binding behaviour as the original.
- Callees with multi-arity dispatch are handled at the granularity of a
  single arity (the one matching the call's arg count).
- Variadic arities are skipped — the rewrite needs a fixed parameter
  shape to satisfy `pick_arity`'s non-variadic filter.
- If the post-dominator analysis can't find a single block that
  dominates all of `dst`'s use-blocks (e.g. across an exception edge),
  the rewrite is silently skipped, leaving the original `Call` in place.

### File map (delta vs. earlier stages)

| File | New behaviour for stage 4 |
|------|--------------------------|
| `crates/cljrs-ir/src/lib.rs` | `Inst::RegionParam`, `Inst::CallWithRegion` variants; `Display`, `dst()`, `uses()`, `effect()` extended |
| `crates/cljrs-ir/src/lower/regionalize.rs` | The pass itself (specialise + rewrite call site) |
| `crates/cljrs-ir/src/lower/optimize.rs` | Pipeline now runs `promote_cross_fn_allocs` after the local region-promotion pass; several CFG helpers (`collect_use_blocks`, `lca_of`, `lca_of_many`, `blocks_on_path`, `has_back_edge`, `region_contains_throw`) made `pub(crate)` so `regionalize.rs` can reuse them |
| `crates/cljrs-ir/src/lower/inline.rs` | `clone_inst` extended to copy the new variants |
| `crates/cljrs-eval/src/ir_interp.rs` | Dispatch for `RegionParam` (binds nil) and `CallWithRegion` (looks up the named subfunction in `ir_func.subfunctions` and re-enters `interpret_ir`) |
| `crates/cljrs-compiler/src/codegen.rs` | Cranelift translation: `RegionParam` defines the var as `rt_const_nil`; `CallWithRegion` reuses `emit_direct_call` |
| `crates/cljrs-ir/tests/escape_regression.rs` | Three stage-4 regression tests (`stage4_*`) |

---

## File map

| File | Relevance |
|------|-----------|
| `crates/cljrs-ir/src/lower/escape.rs` | Core analysis; stages 2 and 3 live here |
| `crates/cljrs-ir/src/lower/optimize.rs` | Local region promotion pass + pipeline driver |
| `crates/cljrs-ir/src/lower/regionalize.rs` | Stage-4 cross-function region promotion |
| `crates/cljrs-ir/src/lower/inline.rs` | Stage 1 |
| `crates/cljrs-ir/src/lib.rs` | IR types including `RegionParam`/`CallWithRegion` |
| `crates/cljrs-ir/tests/escape_regression.rs` | Regression tests for stages 2–4 |
| `crates/cljrs-eval/src/ir_interp.rs` | IR interpreter; stage 4 dispatch |
| `crates/cljrs-compiler/src/codegen.rs` | Cranelift codegen; stage 4 dispatch |
| `crates/cljrs-gc/src/region.rs` | Thread-local `REGION_STACK` consulted by stage-4 callee allocations |
