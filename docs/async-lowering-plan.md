# Plan: AOT State-Machine Lowering for `^:async` Functions (Phase H)

## Context

`clojurust` AOT-compiles Clojure to native code via Cranelift, but **`^:async`
functions are entirely excluded from native compilation**. Today any form whose
macro-expansion contains `await` is force-kept in the tree-walking interpreter
(`crates/cljrs-compiler/src/aot.rs::expanded_needs_interpreter`, line ~622), and
`codegen.rs` (lines 976–983) returns `CodegenError::UnsupportedInst` for the
four async IR instructions (`Await`/`Spawn`/`ChanTake`/`ChanPut`). Async bodies
run on the cooperative tree-walker `eval_async`
(`crates/cljrs-async/src/eval_async.rs`), which yields via
`tokio::task::yield_now()`.

This is the long-deferred **"Phase H — Async JIT"** item in `TODO.md` (line 448)
and `docs/jit-plan.md` (Layer 5 item 5, line 287). The goal: lower an async
function into an explicit, resumable **state machine** — exactly how the Rust
compiler desugars `async fn` into a `Future::poll` state machine — so async
bodies compile to native code instead of being interpreted.

**Scope (confirmed):**
- **Backend: AOT-first, JIT-ready.** The transform is an IR-on-IR pass feeding
  the *shared* `codegen.rs`, so AOT (`cljrs compile`) emits the state machine
  now and the in-process JIT (Tier 3 `cljrs-jit`) can reuse it later with
  minimal extra dispatch wiring. No JIT dispatch wiring in this plan.
- **Async surface: core `await` + control flow (phases H1–H3).** `await` across
  straight-line code, `if`/`let`, and `loop`/`recur`. Channels/`spawn` (H4) and
  `try`/`catch` spanning a suspend (H5) are documented follow-ups; the existing
  interpreter fallback covers them correctly in the meantime.

**Why this is feasible now** — every foundation already exists and is verified:
- IR already defines the async instructions and `IrFunction.is_async`
  (`crates/cljrs-ir/src/lib.rs:444-473, 575-578`); `Inst::{dst,uses,effect}`
  already handle them.
- `codegen.rs` is generic over `cranelift_module::Module` — AOT and the future
  JIT share one code path.
- The `rt_abi` C-bridge convention (opaque `*const Value` tokens, **no GcPtrs
  embedded in code**) is established (`crates/cljrs-compiler/src/rt_abi.rs`).
- A **hidden-parameter ABI** precedent exists: region-parameterised variants
  thread a `*mut Region` as a hidden trailing arg (`takes_region_param` /
  `abi_param_count`, `lib.rs:621/636`; `Inst::RegionParam`). The state-machine
  pointer threads the same way.
- The executor (`spawn_future`/`settle_future`, `CljxFuture`/`Value::Future`,
  the `LocalSet` GC-service task) is in place and reused unchanged.
- **GC tracing of suspended state is already solved by existing machinery** (see
  the critical decision below).

---

## Critical design decision: GC-tracing suspended state via existing root stacks

A suspended state machine holds live Clojure values (`GcPtr`s) that the
collector must trace while the task is parked. **We do not build a new
"async-task root set."** Instead we reuse the existing thread-local root stacks:
`crates/cljrs-env/src/gc_roots.rs` exposes `VALUE_ROOTS: Vec<(*const Value,
usize)>` and `root_values(&[Value]) -> ValueRootGuard`, and its own comment
(line ~334) states the invariant we depend on: at an `async_gc_collect`, the
thread-local root stacks "fully describe all GcPtrs held." Because the
`LocalSet` is single-threaded, **when `async_gc_collect` runs no task is
mid-poll**, so each suspended task's live values are exactly what it registered
before yielding.

Therefore: the state object holds its live values in a contiguous `Vec<Value>`
(`slots`), and the Rust `Future` adapter registers that slice with a persistent
`VALUE_ROOTS` entry held **across every `.await`**. Allocate the state object on
the **GC heap** (never a region — the task outlives any caller's region scope,
the exact hazard `spawn_future` already guards with `poison_active_regions`,
`eval_async.rs:49`). This reuses proven machinery and eliminates a whole class
of "forgot to trace" bugs. **This is the make-or-break item; GC-stress tests are
mandatory (§Verification).**

---

## Part 1 — IR-on-IR state-machine pass

New file **`crates/cljrs-ir/src/lower/async_lower.rs`** (registered in
`lower/mod.rs`). Entry point:

```rust
pub fn lower_async(f: &IrFunction) -> Result<AsyncLowering, Unsupported>;
// AsyncLowering { poll_fn: IrFunction, n_state_slots: usize }
```

Producing an ordinary **non-async** "poll function" `IrFunction` that existing
codegen compiles unchanged except for three new instructions. IR-on-IR is
chosen over a codegen-level transform because it keeps `codegen.rs` block-by-
block translation intact, is unit-testable on hand-built IR, serializes through
the existing `IrBundle`, and flows through the existing type-inference/escape
passes.

**1a. Suspend-point splitting.** A suspend point is any `Await` / `ChanTake` /
`ChanPut` / awaited-`Spawn`. Split each block so a suspend is the last operation
before a state transition. Assign each resume point a `state: i32` (entry =
state 0). The poll function = an entry **dispatch block** `switch(state) -> goto
resume_block_k`, followed by the original blocks re-stitched. Branches and loop
back-edges (`RecurJump`) are preserved as ordinary CFG edges.

**1b. Liveness → state slots.** Write a small dedicated backward-liveness pass in
`async_lower.rs` (no reusable cross-block liveness exists today; `escape.rs`/
`optimize.rs` do def-use only). For each suspend `S`, `live_across(S)` = vars
defined before `S` and used after it on any path. SSA makes slot assignment
clean (one def per VarId → one slot = "saved copy of `vN`"). Params get slots
too (saved once in state 0). On suspend: `StateStore(slot_i, vN)` for each live
var, set next state, `Return(Pending)`. On resume: `StateLoad(vN, slot_i)` to
rematerialise.

**1c. recur / loops across suspends.** `RecurJump` is treated as a normal
liveness successor (it already carries `args` mapped to header phis). Loop-
carried vars naturally fall into `live_across` and are saved/restored each
iteration — correct by construction. **Ordering requirement:** run codegen's
existing phi-elimination / SSA-destruction *before* async lowering so we operate
on a phi-stable form (verified in phase H0).

**1d. New `Inst` variants** (in `crates/cljrs-ir/src/lib.rs`; update
`Inst::{dst,uses,effect}` + `Display`):
- `StateStore { slot: u32, val: VarId }` — write a live value into the state
  object (`Effect::HeapWrite`).
- `StateLoad { dst: VarId, slot: u32 }` — read it back (`Effect::HeapRead`).
- `AsyncSuspend { kind: SuspendKind, operand: VarId, next_state: u32, dst:
  VarId }`, `SuspendKind ∈ {Await, ChanTake, ChanPut, Spawn}` — "register the
  awaited thing, set resume state, return Pending; on re-poll check readiness,
  bind `dst` if ready else Pending again."

`StateStore`/`StateLoad`/`AsyncSuspend` all reference the state object via the
poll function's **hidden leading `*mut CljxStateMachine` parameter**, threaded
exactly like the region param. Add an `is_async_poll_fn: bool` field to
`IrFunction` (serde-defaulted) so codegen emits the extra param + `switch(state)`
prologue.

---

## Part 2 — Runtime representation & poll ABI

**Poll-function ABI** (emitted by codegen):
```rust
extern "C" fn <name>__poll(state: *mut CljxStateMachine, out: *mut *const Value) -> i32
// 0 = Pending, 1 = Ready (out written), 2 = Threw (out holds the thrown Value)
```
The `out`-param + `i32` keeps the thrown-vs-resolved distinction unambiguous and
integrates with the existing `PENDING_EXCEPTION` slot in `rt_abi`.

**State object** — new `crates/cljrs-async/src/state_machine.rs`:
```rust
pub struct CljxStateMachine {
    pub state: i32,
    pub slots: Vec<Value>,        // contiguous; GC-traced via VALUE_ROOTS slice
    pub pending: Option<Value>,   // the Future/Promise/Channel currently awaited
    poll_fn: extern "C" fn(*mut CljxStateMachine, *mut *const Value) -> i32,
}
```
- Allocated on the **GC heap** (`GcPtr::new`). `slots`' backing pointer is stable
  after state 0, giving a fixed `(*const Value, len)` to register.
- GC tracing while suspended: the adapter holds a persistent root registration
  over the suspended interval. Add
  `gc_roots::register_state_slots(&[Value]) -> StateRootGuard` (a push/pop of the
  same `(ptr,len)` entry `root_values` uses, but lifetime-managed by the guard
  rather than a stack frame).

**Executor integration — thin Rust `Future` adapter** (recommended over a custom
executor) in `state_machine.rs`:
```rust
struct CompiledAsyncTask { sm: GcPtr<CljxStateMachine>, _roots: StateRootGuard }
impl Future for CompiledAsyncTask {
    type Output = EvalResult;
    fn poll(self, cx) -> Poll<EvalResult> {
        match (sm.poll_fn)(sm_ptr, &mut out) {
            0 => { cx.waker().wake_by_ref(); Poll::Pending }  // cooperative re-poll
            1 => Poll::Ready(Ok(out_value)),
            2 => Poll::Ready(Err(EvalError::Thrown(pending_exception_value()))),
            _ => unreachable!(),
        }
    }
}
```
Spawn it through the **existing `spawn_future`** (`eval_async.rs:40`), reusing
`poison_active_regions`, result-future rooting, and `settle_future` verbatim.
`wake_by_ref` + `Poll::Pending` mirrors the tree-walker's `yield_now()` so the
GC-service cadence and cancellation are unchanged. State and slots are `!Send`;
using `spawn_local`/`spawn_future` (never `spawn`) preserves that — do **not**
make `CljxStateMachine: Send`.

---

## Part 3 — rt_abi additions (`crates/cljrs-compiler/src/rt_abi.rs`)

Registered with the AOT object module (and trivially with `JITBuilder` later).
Core (H1–H3):
- `rt_state_store(sm, slot, val)` — `slots[slot] = (*val).clone()`.
- `rt_state_load(sm, slot) -> *const Value` — return `&slots[slot] as *const
  Value` (zero-copy; the Vec is not reallocated after state 0).
- `rt_async_register_await(sm, fut) -> i32` — store `fut` in `sm.pending`; return
  `1` if already `Done`/`Failed` (codegen can skip the suspend), else `0`.
- `rt_async_poll_ready(sm) -> i32` / `rt_async_take_result(sm, out) -> i32` — on
  resume, inspect `sm.pending`'s `FutureState`: Running→Pending(0), Done→write
  value + Ready(1), Failed→stash exception + Threw(2). Reuses `CljxFuture`
  exactly.

Codegen maps `AsyncSuspend{Await}` → `rt_async_register_await`; if `0`, store
`next_state` + `return Pending`; the resume block calls
`rt_async_poll_ready`/`rt_async_take_result`.

Deferred to H4 (listed for completeness, not built in this plan):
`rt_async_chan_take`/`rt_async_chan_put` (+ `try_take`/`try_put` on
`CljChannel`), `rt_async_spawn`.

---

## Part 4 — Integration & dispatch (AOT)

- **`codegen.rs:976-983`** — replace the blanket `UnsupportedInst` for the four
  async insts with codegen for `StateStore`/`StateLoad`/`AsyncSuspend` → the
  rt_abi calls above. Keep a defensive `UnsupportedInst` fallback if a raw async
  inst ever reaches codegen (it shouldn't — the pass ran first).
- **Poll-fn signature** — codegen reads `is_async_poll_fn` and emits the hidden
  leading `*mut CljxStateMachine` param (threaded like `RegionParam`) plus the
  `switch(state)` prologue.
- **`aot.rs:622-640`** — `expanded_needs_interpreter` stops excluding `await`
  *when the enclosing fn was successfully async-lowered*: attempt `lower_async`;
  on success register the compiled poll fn; on **any** unsupported construct
  (e.g. `try` across a suspend, channels in H1–H3) it returns `Err(Unsupported)`
  and the current interpreter exclusion stands. This gives the correctness
  fallback for free.
- **Dispatch (`cljrs-env/src/apply.rs:113` `dispatch_if_async`)** — when the
  callee arity has a registered poll fn, build a `CljxStateMachine` (materialise
  args/captures into `slots`), wrap in `CompiledAsyncTask`, and hand to
  `spawn_future` instead of `run_async_fn`. The `Value::Future` return and the
  `call_cljrs_fn` seam are otherwise unchanged. A registry (arity-id → poll-fn
  pointer) drives the choice; absence → `run_async_fn` tree-walk fallback.

---

## Phasing (each milestone independently testable)

- **H0 — Scaffolding & ordering.** Add the three `Inst` variants + `is_async_
  poll_fn` field, `CljxStateMachine` + `CompiledAsyncTask` + `register_state_
  slots`, and the core rt_abi bridges. Confirm phi-elimination runs before async
  lowering. Tree-walk fallback still default. *Milestone:* builds; zero behavior
  change.
- **H1 — Straight-line single-await.** `(defn ^:async f [x] (inc (await x)))`.
  One suspend, linear liveness. *Milestone:* native poll fn returns Ready/Pending
  with parity to `eval_async`.
- **H2 — if / let across awaits.** Branch + join liveness; multiple suspends on
  distinct paths; let-bound value live across an await; `(+ (await a) (await b))`.
- **H3 — loops / recur across awaits.** Loop-carried slots, back-edge resume:
  `(loop [n 10 acc 0] (if (zero? n) acc (recur (dec n) (await (step acc)))))`.

*Follow-ups (out of scope here; interpreter fallback covers them):* **H4** —
channels (`ChanTake`/`ChanPut` + `try_take`/`try_put`) and `spawn`. **H5** —
`try`/`catch` spanning a suspend (catch frames modeled as extra state slots +
per-state active-handler field, mirroring `eval_try_async`,
`eval_async.rs:279`).

---

## Verification

- **Parity harness (primary gate).** For each `^:async` sample, run twice —
  forced tree-walk (`run_async_fn`) vs. compiled state machine — asserting equal
  results and equal thrown errors. Cover the H1–H3 examples above.
- **IR unit tests** in `async_lower.rs`: feed hand-built async `IrFunction`s;
  assert resume-state count, slot assignment, and that no async insts remain in
  the poll fn.
- **GC stress (mandatory for the §Critical decision).** Run the parity tests
  under an aggressive-GC config (force a collection at every yield) to exercise
  slot rooting directly. Reuse the existing GC-stress toggle.
- **Cancellation.** Verify `FutureState::Cancelled` / the cancellation flag
  (`gc_roots.rs`) still aborts a compiled task (adapter checks at poll entry).
- **End-to-end.** `cljrs compile` a small async program and run the native
  binary; confirm it produces the same output as `cljrs run` on the same source.
- Standard gates: `cargo test`, `cargo clippy`, `cargo fmt`.

---

## Trickiest correctness risks (ranked)

1. **GC-tracing suspended slots** — mitigated by reusing `VALUE_ROOTS`-style
   registration held across every `.await` (sound under the `LocalSet` single-
   thread invariant); state on the GC heap, never a region. GC-stress tests are
   the proof.
2. **`!Send` interaction** — state + slots must never cross threads; reuse
   `spawn_future`/`spawn_local`, never make the state `Send`.
3. **recur / loop-carried liveness** — `RecurJump` as a normal liveness
   successor; regression test mutating a value per iteration (H3).
4. **Nested awaits in expression position** — ANF already names sub-expressions,
   so `(f (await a) (await b))` is two sequential suspends; falls out of liveness.
   Explicit H2 test.
5. **try/catch across suspends** — deferred (H5) behind the interpreter fallback.
6. **Thrown-vs-resolved ambiguity** — solved by the `out`-param + `i32`
   (Ready/Pending/Threw) plus the existing `PENDING_EXCEPTION` slot.

---

## Files

**New:**
- `crates/cljrs-ir/src/lower/async_lower.rs` — IR-on-IR state-machine pass +
  liveness.
- `crates/cljrs-async/src/state_machine.rs` — `CljxStateMachine`, poll ABI,
  `CompiledAsyncTask`.

**Modified:**
- `crates/cljrs-ir/src/lib.rs` — new `Inst` variants, `SuspendKind`,
  `is_async_poll_fn`; update `Inst::{dst,uses,effect}`/`Display`; `lower/mod.rs`
  registration.
- `crates/cljrs-compiler/src/codegen.rs` — replace `UnsupportedInst`
  (lines 976–983); poll-fn prologue + hidden state param + new-inst codegen.
- `crates/cljrs-compiler/src/rt_abi.rs` — async/state bridges (Part 3).
- `crates/cljrs-compiler/src/aot.rs` — `expanded_needs_interpreter` (line ~622)
  gated on async-lowering success.
- `crates/cljrs-env/src/apply.rs` — `dispatch_if_async` (line 113) routes to the
  compiled poll fn when registered.
- `crates/cljrs-env/src/gc_roots.rs` — `register_state_slots`/`StateRootGuard`.
- `crates/cljrs-async/src/eval_async.rs` — reuse `spawn_future`/`settle_future`
  (likely no change beyond making them reachable).

**Docs / READMEs (CLAUDE.md mandate — same commit as the code):**
- `crates/cljrs-ir/README.md`, `crates/cljrs-async/README.md`,
  `crates/cljrs-compiler/README.md`, `crates/cljrs-env/README.md`.
- `docs/jit-plan.md` (promote "Phase H" to active with H0–H5) and `TODO.md`
  (line 448).
