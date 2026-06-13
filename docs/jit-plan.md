# Plan: JIT Compilation (Tiered Execution)

## Overview

`clojurust` runs code through three tiers today, joined at a single seam — the
`GlobalEnv.call_cljrs_fn` function pointer:

- **Tier 0 — tree-walking interpreter** (`crates/cljrs-interp`): the universal
  fallback. Function bodies are stored as unevaluated `Form` ASTs on `CljxFn` /
  `CljxFnArity` (`crates/cljrs-value/src/types.rs`); each arity carries an
  `ir_arity_id: u64` cache key.
- **Tier 1 — IR register interpreter** (`crates/cljrs-eval`): the registered
  hook `call_cljrs_fn` (`crates/cljrs-eval/src/apply.rs`) consults
  `ir_cache::get_cached(arity_id)` and, if a non-async `IrFunction` is cached,
  runs `ir_interp::interpret_ir` over ANF/SSA IR; otherwise it falls back to
  Tier 0. Lowering is opt-in and eager (`CLJRS_EAGER_LOWER=1`), and only for
  non-capturing top-level functions without destructuring or rest params.
- **Tier 2 — AOT Cranelift** (`crates/cljrs-compiler`): `codegen.rs` lowers
  `IrFunction` → CLIF → `.o` via `cranelift-object`, with a ~40-function
  `extern "C"` runtime bridge (`rt_abi.rs`) and a cargo-harness linker
  (`aot.rs`). Constants are materialized via runtime calls, never embedded as
  `GcPtr`s. Intra-module calls are rewritten to `CallDirect`.

**The problem this plan solves:** ad-hoc code — a script run from the CLI or
expressions typed at the REPL — never reaches native speed. AOT requires an
explicit `cljrs compile` and a cargo build; the REPL / `run` / `eval` paths top
out at the Tier-1 interpreter. Worse, many common forms (closures with captured
bindings, destructured params, variadics, `apply` / `swap!` / `atom`, async)
don't even reach Tier 1 and stay in the tree-walker.

**The solution:** a **fourth tier — an in-process JIT** that compiles hot
functions and hot loops to native code on a background thread, reclaims that
code when the REPL redefines functions, shrinks the set of forms that fall back
to the interpreter, and specializes bump-allocation strategy to the *context a
function is actually called from*. Intended outcome: `cljrs run` and the REPL
approach AOT-class throughput on hot code with no explicit compile step, while
staying responsive and memory-stable across a long session.

### Locked design decisions

- **GC-mode roots:** conservative stack scanning first (sound because the
  collector is non-moving); precise Cranelift stack maps deferred.
- **Bump allocation:** extend profile-driven scratch regions to the **default
  GC build**, not only the `no-gc` build.
- **Compilation timing:** background worker thread with atomic code-pointer
  swap; never stall a hot call.

This document is the architecture; `TODO.md` Phase 10 tracks the milestones.

---

## Layer 1 — Backend & Crate Structure

### Backend-agnostic codegen

`codegen.rs` is implicitly tied to `cranelift-object`'s `ObjectModule`. Refactor
the CLIF-emitting core to be **generic over `cranelift_module::Module`** so it is
driven by either `ObjectModule` (AOT) or `JITModule` (JIT). The lowering logic
(`IrFunction` → CLIF) and the `rt_abi` signatures are shared verbatim between
the two backends.

### New crate `cljrs-jit`

A new `cljrs-jit` crate depends on the shared codegen, `cranelift-jit`, and
`cranelift-module`. The existing `cljrs-compiler` Cargo description already reads
*"JIT (Cranelift) and AOT compiler backend"*, so a sibling crate (or a `jit`
module inside `cljrs-compiler`) is the natural home; a separate crate keeps the
JIT's runtime and threading concerns out of the AOT build-tool path. Add
`cranelift-jit` to the workspace dependencies.

### Runtime bridge & constants

- **rt_abi symbol registration:** the ~40 `extern "C"` bridge functions in
  `crates/cljrs-compiler/src/rt_abi.rs` are registered with `JITBuilder` (via
  `symbol` / `symbols`) so JIT-emitted calls resolve in-process. This reuses the
  AOT bridge with zero behavioral change.
- **Constants:** reuse AOT's strategy — materialize constants (`Keyword`,
  `Symbol`, literal collections) through `rt_abi` runtime calls rather than
  embedding `GcPtr`s in the code stream. **This keeps emitted code free of GC
  pointers, which is what makes both conservative stack scanning and code
  unloading tractable: the code itself holds no roots.**

---

## Layer 2 — Tiering & Hot-Path Detection

### Invocation counters

Add an `AtomicU32` invocation counter keyed by `ir_arity_id`, in a new
`JitState` entry parallel to the IR cache (extend
`crates/cljrs-eval/src/ir_cache.rs`, or a sibling `jit_state.rs`) rather than on
`CljxFnArity`, so the hot dispatch path reads one shared structure. The counter
bumps on the Tier-1 entry path (`try_ir_path`, `crates/cljrs-eval/src/apply.rs`)
— a single relaxed atomic increment, negligible cost.

### Threshold & promotion

A tunable invocation threshold (default ~1–2k; env var `CLJRS_JIT_THRESHOLD`,
plus a CLI flag) trips a function from Tier 1 → JIT-queued. The already-reserved
`-X trace:jit` logging feature (`crates/cljrs-logging`) provides observability.

### Background compilation with atomic swap

Crossing the threshold enqueues `(arity_id, Arc<IrFunction>)` onto a JIT worker
thread. The worker compiles via `JITModule`, then atomically publishes the
finalized code pointer into the `JitState` entry (an `AtomicPtr` / epoch-guarded
slot). Until then, calls keep running Tier 1 — **no stall**. On the next
dispatch, `call_cljrs_fn` checks the JIT slot *before* the IR-interp slot and
calls native code. Dispatch order becomes **JIT-native → Tier-1 IR →
tree-walk**, and the seam stays the one integration point — no caller changes.

### Loop back-edge counters + OSR (the critical case for ad-hoc code)

A script or REPL form is often a *single* call containing one very hot
`loop*` / `recur`. Such a function never returns to re-dispatch, so
invocation-count tiering alone never promotes it. Add a back-edge counter at loop
headers (the IR already has explicit `RecurJump` terminators and loop
subfunctions). When a back-edge trips, perform **On-Stack Replacement (OSR)**:

1. Compile an OSR-entry variant of the function whose entry block *is* the loop
   header, with the loop's live-in values (the `recur` bindings, as IR `VarId`s)
   passed as parameters.
2. At its next safepoint poll inside the loop, the Tier-1 interpreter transfers
   its register file into the native OSR frame and jumps.

OSR is the single trickiest piece and is sequenced as its own phase (10.4).

---

## Layer 3 — Code Unloading

**Why it matters:** the REPL re-runs `defn` constantly; each redefinition
produces a fresh `CljxFn` / arity (new `ir_arity_id`) and orphans the old one's
native code. Without reclamation, executable memory grows unbounded across a
session.

- **Per-version code, grouped into epochs.** Each compiled arity gets its own
  `JITModule`-allocated region tagged with the `ir_arity_id` and a monotonically
  increasing *epoch*. Redefining a var marks the prior arity's code *stale*.
- **Epoch-based reclamation leveraging existing STW safepoints.** Because emitted
  code embeds no GC roots and the collector already brings all mutator threads to
  a stop-the-world safepoint (`crates/cljrs-gc/src/cancellation.rs`), the JIT
  piggybacks on that quiescent point: at STW, scan active JIT frames (Layer 4) to
  determine which stale epochs still have a frame executing; epochs with **no**
  live frame are freed. This avoids a separate quiescence protocol and sidesteps
  the unload-vs-execute race.
- **Lifetime tie:** the `JitState` entry is keyed by `ir_arity_id`; when the
  owning `CljxFn` is collected or its var is rebound, the entry moves to the
  stale set for the next reclamation sweep.

---

## Layer 4 — GC Integration for JIT Frames (default GC build)

The book's memory chapter currently states the bump allocator runs in
"AOT-compiled code only" and the interpreter only ever uses the GC. JIT changes
that: native code holds `Value`s in registers and on the stack within the GC
build, so the collector must find those roots.

- **Conservative stack scanning first** (per decision). The collector is
  **non-moving** mark-and-sweep, so it only needs to *find* roots, never rewrite
  them. At STW, conservatively scan the JIT thread's stack range: any word that
  decodes as a valid heap `GcPtr` (passes the existing header/magic check and is
  not a region-tagged pointer — `GcPtr` already tags region pointers in bit 0 so
  the marker skips them) is treated as a root and marked. False positives only
  retain extra garbage for one cycle; they cannot corrupt a non-moving heap. The
  `ALLOC_ROOTS` / `trace_thread_alloc_roots` plumbing in `crates/cljrs-gc` is the
  integration point.
- **Safepoint polling in JIT loops:** emit a safepoint poll (load + branch
  against the STW request flag) at loop back-edges and function entry, mirroring
  the interpreter's `safepoint()` placement, so JIT code yields promptly to a
  collection request and doesn't starve the STW protocol.
- **Deferred:** precise Cranelift stack maps as a later optimization (tighter
  retention; a prerequisite for any future *moving* GC). Noted as a cost, not
  built now.

---

## Layer 5 — Shrinking the Interpreter Seam

Goal: **anything reaching Tier 1 is also JIT-able, and far fewer forms fall back
below Tier 1.** Concrete lowering/codegen work, highest ROI first:

1. **Destructured params** — highest ROI, purely a lowering change. Destructuring
   is sugar over `let`; expand it into explicit binding instructions in the IR
   prologue at lowering time (`crates/cljrs-eval/src/lower.rs`) so the param list
   becomes simple names. Removes a whole fallback class with no codegen changes.
2. **Closures with captured bindings** — IR already has `AllocClosure` with a
   `ClosureTemplate` and capture list (captures minimized per commit `2fd3730`).
   Make capture lowering complete and ensure codegen emits the closure allocation
   plus environment threading. Unlocks the common `fn`-returning-`fn` and
   `map` / `filter` lambda cases.
3. **Variadic / rest params** — extend the existing rest-arg list construction
   (already handled in `try_ir_path`, `crates/cljrs-eval/src/apply.rs`) through
   codegen.
4. **Special-cased ops** (`apply`, `atom`, `swap!` / `reset!`, `volatile!`,
   `vswap!`, etc.) — promote from tree-walk special cases to first-class
   `KnownFn` / IR instructions with effect classes, so they lower and codegen
   uniformly instead of bouncing to Tier 0.
5. **Async (`^:async`)** — deferred to **Phase H**: emit Cranelift state machines
   with explicit resume points integrating with the Tokio executor. Large; out of
   scope for the first JIT.

**Macros** intentionally remain interpreter-expanded: they operate on forms,
expand once, and their *output* is lowered/JIT-able — so they are not a seam to
close.

---

## Layer 6 — Context-Driven Bump Allocation (extended to the GC build)

This is the largest new design surface and the most distinctive goal: specialize
allocation based on *where to-be-compiled code is called from*.

- **Thread the active region as a hidden parameter into JIT'd calls.** When
  escape analysis (`crates/cljrs-ir/src/lower/escape.rs`) plus call-site context
  prove a callee's allocations don't outlive the caller, the JIT passes the
  caller's active region pointer as a hidden argument and the callee
  bump-allocates into it. This generalizes the existing `ScratchGuard` /
  `alloc_ctx` "return value lands in the caller's context" protocol
  (`crates/cljrs-gc/src/alloc_ctx.rs`) and the IR's region-parameterized
  subfunction variants across the JIT call boundary.
- **Call-site monomorphization of allocation strategy.** A hot callee can be
  specialized per calling context into variants that allocate in (a) the static
  arena, (b) the caller's scratch region, or (c) the GC heap — chosen by the call
  site's proven escape behavior. Inline caches at call sites (Phase 10's
  inline-cache goal) supply the concrete-callee info that makes this and
  small-callee inlining possible; inlining then exposes *more* non-escaping
  allocations.
- **REPL/script top-level form as an arena scope.** Treat each REPL input (or a
  whole `cljrs run` execution) as a bump-region scope: non-escaping intermediate
  garbage produced while evaluating one form is freed wholesale when the form
  finishes, instead of pressuring the GC. The result value is promoted out
  (static arena / heap) before the region resets — exactly the existing
  `pop_for_return` discipline, lifted to the top level.
- **Extend regions to the default GC build (per decision).** Today the region
  machinery is `no-gc`-only. Introduce **profile-driven scratch regions that
  coexist with the tracing collector**: regions hold only provably-non-escaping
  values; region pointers remain bit-0 tagged so the marker already skips them;
  values that turn out to escape are promoted to the GC heap before region reset.
  A heap-only fallback path means correctness never depends on the analysis being
  perfect.

---

## Risks & Trickiest Parts

- **OSR** — transferring interpreter register state into a native frame.
- **Deoptimization** — when type / inline-cache assumptions are violated (guards
  + bail to Tier 1). Needed once specialization / unboxing lands.
- **Conservative scanning** — false-retention bounds, and getting the
  stack-range and `GcPtr` validity check exactly right.
- **Code-unload vs execute race** — mitigated by reclaiming only at STW with
  frame scanning, but must be argued carefully.
- **Coexisting regions + tracing GC** — escape-analysis soundness; the
  heap-promotion fallback is the safety net.

---

## Phasing & Sequencing

- **Phase 10.0 — Backend refactor.** Make `codegen.rs` generic over
  `cranelift_module::Module`; AOT unchanged. *Milestone:* AOT still builds and
  `cargo test` passes.
- **Phase 10.1 — Minimal JIT tier (first working JIT).** New `cljrs-jit` crate;
  `JITModule` + rt_abi symbol registration; compile non-capturing,
  no-destructure functions (the set Tier-1 already handles); invocation counter +
  threshold; background compile + atomic swap; dispatch JIT → Tier 1 →
  tree-walk; conservative stack scan + safepoint polls. *Milestone:* a hot
  top-level `defn` measurably runs native under `cljrs run`, with `-X trace:jit`
  showing promotion; falls back cleanly otherwise.
- **Phase 10.2 — Code unloading.** Epoch tagging + STW-time reclamation of stale
  arities. *Milestone:* a REPL loop redefining a fn 10⁵ times shows bounded
  executable memory.
- **Phase 10.3 — Seam shrink.** Destructuring → captures → variadics → special
  ops (the Layer 5 order). *Milestone:* the Tier-0 fallback rate on a
  representative script drops sharply (tracked via `-X trace:jit`).
- **Phase 10.4 — OSR.** Loop back-edge counters + OSR entry for hot top-level
  loops. *Milestone:* a single-call million-iteration `loop` / `recur` script
  promotes to native mid-run.
- **Phase 10.5 — Context-driven bump allocation.** Region threading across JIT
  calls + REPL/script arena scope (no-gc first), then profile-driven regions
  coexisting with the GC build, with heap-promotion fallback. *Milestone:* GC
  allocation / collection counts (`GC_STATS`) drop on allocation-heavy hot code.
- **Phase 10.6 — Specialization & inline caches.** Call-site monomorphization,
  numeric unboxing, inline caches for protocol / keyword dispatch, deopt guards.
- **Phase H (deferred) — Async JIT.** Cranelift state machines for `^:async`.

---

## Files (for the implementation phases — not edited by this planning pass)

Reused / modified: `crates/cljrs-compiler/src/{codegen.rs,rt_abi.rs,aot.rs}`;
`crates/cljrs-eval/src/{apply.rs,ir_cache.rs,lower.rs,ir_interp.rs}`;
`crates/cljrs-ir/src/lower/escape.rs`;
`crates/cljrs-gc/src/{alloc_ctx.rs,region.rs,cancellation.rs}`;
`crates/cljrs-value/src/types.rs`; `crates/cljrs-logging` (`jit` feature);
`crates/cljrs/src/main.rs` (CLI flags).

Created: `crates/cljrs-jit/` (with its own `README.md` per the CLAUDE.md
crate-README rule).
