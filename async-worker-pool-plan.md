# Async Worker Pool Plan for clojurust

## Why this document exists

`cljrs-async` today drives all async work on a **single** Tokio `current_thread` + `LocalSet`
executor. One interpreter thread cannot use more than one core, so any "server" built on it is a
demo: throughput is capped at one core no matter how many connections or `go` blocks are in flight.

This plan turns that single executor into a **worker pool** so async work — networking
(`cljrs-net`), file I/O (`cljrs-io`), and ordinary `go`/`^:async` code — can use every core.

It is split out of `networking-plan.md` deliberately: this is a `cljrs-async` (and, at its limits,
`cljrs-gc`) change that affects *everything* async, not just sockets. `networking-plan.md` Phase H
references this document; the networking phases are written against a per-worker `LocalSet` so they
are unaffected by when this lands.

Read `async-plan.md` first — every mechanism here (the `LocalSet` executor, `spawn_future`,
`await_value`, the GC-service task, safepoints) comes from it.

---

## The crucial precondition: the GC already allows this

The single-thread executor is an **async-runtime choice, not a GC limitation.** `current_thread` +
`LocalSet` was chosen so eval futures can hold `GcPtr`s across `await` without being `Send` — a
multi-thread Tokio runtime requires `Send` futures. But the garbage collector underneath is already
a **shared-heap, multi-mutator, stop-the-world** design:

| Capability | Evidence |
|---|---|
| One global, `Mutex`-guarded heap shared by all threads | `static HEAP: GcHeap` (`cljrs-gc/src/lib.rs`); `GcHeapInner` behind a `Mutex`; `unsafe impl Sync for GcHeap` |
| Thread registration for collection | `cljrs_gc::register_mutator()`, already called at `crates/cljrs/src/main.rs:291` and in test harnesses that `thread::spawn` |
| STW that parks **all** mutators and traces **each** thread's roots | `begin_stw()` (`cljrs-env/src/gc_roots.rs`); `GC_CANCELLATION` tracks `registered_threads`/`parked_threads` |
| Per-thread root stacks (no cross-thread root sharing needed) | `ENV_ROOTS` (`cljrs-env/src/gc_roots.rs`), `BINDING_STACK` (`cljrs-env/src/dynamics.rs`), `EVAL_CONTEXT` (`cljrs-env/src/callback.rs`), `ALLOC_ROOTS`/`REGION_STACK` (`cljrs-gc`) |
| Shared, thread-safe global environment | `GlobalEnv` is `Arc`-shared with `RwLock`/`Mutex` namespace tables (`cljrs-env/src/env.rs`) |
| `GcPtr` declared movable across threads | `unsafe impl<T: Trace> Send/Sync for GcPtr<T>` (`cljrs-gc/src/lib.rs:302–303`) |

So multiple OS threads can already run interpreters against the same heap. **No GC rewrite is
required to add a worker pool** — only the async executor and per-thread lifecycle change.

### The safety discipline (what makes the `unsafe impl Send` sound)

`GcPtr: Send + Sync` is sound **only** under a discipline the pool must enforce:

1. **Every thread that holds `GcPtr`s is a registered mutator** (`register_mutator()` held for the
   thread's lifetime) **and reaches safepoints regularly.** STW prevents cross-thread dereference
   races *because* all mutators are parked during mark/sweep — a thread that holds `GcPtr`s but
   never registers / never safepoints is the one real hazard: the collector won't wait for it and
   can free objects under it.
2. **Every live value is reachable from some traced root** — a thread-local root stack, a global,
   or a heap object whose `Trace` walks it. A value handed to another thread must be rooted on the
   receiver (or held in a traced structure, e.g. a channel buffer) before the sender drops it.
3. **No `&mut` aliasing of a `Value` across threads.** Values are immutable after allocation;
   shared mutation goes through `atom`/`ref`/`agent`, which are already synchronized.

This document's design satisfies all three by construction (pinned work + channel handoff with
traced buffers). The discipline is stated here so it is enforced as an invariant, not rediscovered.

---

## The model: shared-heap, pinned workers

`W` worker OS threads (default ≈ `std::thread::available_parallelism()`, configurable), each:

- running **its own** `current_thread` Tokio runtime + `LocalSet` (so per-worker futures stay
  `!Send`/local — no change to `spawn_future`/`eval_async`, which keep using `spawn_local`);
- holding a `register_mutator()` guard for its lifetime;
- sharing the **one** `GcHeap` and the **one** `Arc<GlobalEnv>` with all other workers.

This is the nginx-workers / BEAM-schedulers shape, but with a **shared heap** rather than one heap
per worker (the share-nothing variant is the endgame — see Ceilings).

### Work placement

- **A unit of async work is pinned to the worker it starts on.** Its future, the `GcPtr`s it
  touches, and the byte-arrays it allocates all live on that worker's thread. In steady state **no
  `GcPtr` crosses threads.**
- **New work is placed at spawn time.** `async-spawn`/`go`/`^:async` dispatch picks a worker
  (round-robin or least-loaded) and `spawn_local`s the task onto that worker's `LocalSet` via a
  per-worker handle. Work started *inside* a task stays on the same worker by default.
- **External producers hand off `Send` tokens, not `GcPtr`s.** For `cljrs-net`, an accepted socket
  is handed to a worker as a raw **FD** (an integer — `Send`); the worker builds the
  `TcpStream`/byte-arrays locally. This is the general pattern: cross-thread boundaries carry `Send`
  Rust data (`Vec<u8>`, FDs, primitives), and the `!Send` `GcPtr` is constructed on the destination
  worker.

### Cross-worker communication: channels

When a *value* must move between workers, it goes through a `clojure.core.async` channel. This is
STW-safe provided:

- **`CljChannel::Trace` walks its buffer** so a value parked in a channel between a `put!` on worker
  A and a `take!` on worker B stays rooted regardless of which worker collects. **Audit item.**
- **Cross-runtime wakeups work** — Tokio `Waker`s are `Send`, so a `take!` task parked on worker B
  is woken when worker A `put!`s. `CljChannel` already uses a `Mutex` + condvar internally, so the
  cross-thread case is mechanically fine; the work is verifying it under the pool.

---

## Execution / scheduler design

| Concern | Approach |
|---|---|
| Per-worker driver | Each worker thread: build `current_thread` runtime, enter a `LocalSet`, `register_mutator()`, run the GC-service task, then `block_on` the `LocalSet` until shutdown. |
| Spawn routing | A `Pool` holds `W` `LocalSet` spawn handles (`tokio::task::LocalSet::spawn_local` via a per-worker channel of boxed task-builders, since `spawn_local` must run on the owning thread). `spawn_future` gains a "spawn on worker *i*" path; the default picks a worker by a placement policy. |
| Placement policy | Start with round-robin for top-level spawns; child tasks inherit their parent's worker. Least-loaded (by in-flight task count) is a later refinement. |
| Blocking ops (`<!!`/`>!!`) | Already condvar-parked across threads; unchanged, but documented as "safe from any worker / any non-worker thread, never from inside an `^:async` body." |
| GC-service task | One per worker (each worker services its own safepoint/STW participation), coordinated by the existing global `GC_CANCELLATION`. |
| Shutdown | Drain/cancel per-worker `LocalSet`s, drop mutator guards, join threads. |

The core insight that keeps this tractable: **`spawn_local` stays `spawn_local`.** We are not making
futures `Send`; we are running *N independent single-threaded executors* that happen to share a
heap. Each executor is exactly today's executor.

---

## Phased implementation

### Phase 1 — Pool scaffolding (no behavior change)

- Introduce a `WorkerPool` in `cljrs-async` that can spin up `W` worker threads, each with its own
  `current_thread` runtime + `LocalSet` + `register_mutator()` + GC-service task.
- `W = 1` by default initially, so behavior is identical to today; the CLI/embedder gains a knob
  (`--workers N` / `init_with_workers(n)`).
- `init()` becomes pool-aware but still routes everything to worker 0.

**Done when:** existing async tests pass unchanged with the pool at `W = 1`, and `W > 1` boots `W`
idle workers that all register as mutators and participate in STW.

### Phase 2 — Spawn routing across workers

- `spawn_future` / `async-spawn` / `go` / `^:async` dispatch route new top-level tasks to a worker
  by placement policy (round-robin first); child tasks inherit the parent worker.
- Verify per-worker locality: a task and everything it allocates stay on one thread.

**Done when:** with `W > 1`, independent `go`/`^:async` workloads measurably spread across cores,
and a CPU-bound benchmark scales with `W` (up to the allocation-mutex ceiling).

### Phase 3 — Cross-worker correctness audit

- **Audit `CljChannel::Trace`**: confirm buffered values are traced; add a test that `put!`s on one
  worker, forces STW, then `take!`s on another and checks the value survived.
- Confirm cross-runtime wakeups for `take!`/`put!`/`alts!` parked on a different worker than the
  producer.
- Stress STW under multi-worker load: many workers allocating, one triggering collection, verify
  all park at safepoints and every worker's roots are traced.

**Done when:** a cross-worker channel ping-pong survives aggressive forced GC with no UAF/leak under
the GC's debug provenance assertions.

### Phase 4 — Safepoint coverage for long native calls

- Ensure native/FFI calls reachable from async paths poll safepoints (or are wrapped to do so) so a
  single busy worker cannot stall global STW for the rest.
- Document the rule for `interop` authors: long-running native fns must yield to safepoints.

**Done when:** a deliberately long native call on one worker does not deadlock GC for others
(bounded pause), with a regression test.

### Phase 5 — External `Send` handoff API (enables `cljrs-net`)

- Public API for "build this on worker *i* from `Send` inputs": e.g. `spawn_on(worker, FnOnce ->
  Future)` and an FD/`Vec<u8>` handoff helper.
- This is what `cljrs-net`'s accept path calls to pin a connection to a worker.

**Done when:** `cljrs-net` (or a stand-in test) can accept on one thread and run a pinned
connection's reader/writer on a chosen worker, with byte-arrays built on that worker.

---

## The two ceilings (and the endgame)

The shared-heap pool scales to a point and then hits two walls. Naming them up front so they are
chosen, not stumbled into:

1. **Global STW.** Every worker must reach a safepoint before *any* collection runs (Phase 4
   mitigates the stall risk but not the fundamental coordination). I/O `await` points are
   safepoints, so I/O-bound serving is fine; allocation-heavy CPU-bound work pays coordination cost.
2. **The single global heap `Mutex` is the allocation bottleneck.** Every `GcPtr::new` locks the one
   heap; at high allocation rates across many cores this serializes and becomes the scaling wall.

### Endgame (deferred — `cljrs-gc` changes, separate plan when pursued)

- **TLABs (thread-local allocation buffers).** Each worker gets a private bump region and only takes
  the global lock to refill it. Relieves ceiling #2 with no change to the shared-heap / global-STW
  model. The natural *first* `cljrs-gc` follow-up once Phase 2 shows allocation contention.
- **Share-nothing per-worker heaps** (the BEAM/Ractor model). De-globalize `static HEAP` into a
  per-worker heap: no shared alloc lock (ceiling #2 gone), no global STW (ceiling #1 gone — each
  worker collects independently), cross-worker messages **copied/serialized** rather than shared.
  This is the real linear-scaling design and a substantial `cljrs-gc` refactor. The pinned-work +
  `Send`-handoff + channel design in this plan is already the share-nothing message-passing shape,
  so application code and `cljrs-net` port to it unchanged — only the channel handoff switches from
  "share a `GcPtr`" to "copy across heaps."

---

## Risks & open questions

- **Placement policy.** Round-robin ignores task cost; least-loaded needs cheap load metrics. Start
  simple, measure, refine. Bad placement hurts throughput but not correctness.
- **Work stealing.** Not in scope — pinned tasks can't be stolen (they hold `!Send` `GcPtr`s).
  Imbalance is handled by placement, not migration. Revisit only with share-nothing heaps.
- **`thread_local!` correctness.** `EVAL_CONTEXT`, `BINDING_STACK`, `ALLOC_ROOTS`, `REGION_STACK`
  are per-thread, which is *correct* for per-worker pinning, but anything that implicitly assumed a
  single global thread-local must be audited (dynamic `binding` does not cross workers — by design,
  matching Clojure's thread-local `binding` semantics).
- **Determinism of tests.** Multi-worker scheduling is nondeterministic; keep `W = 1` as the default
  for deterministic test runs and gate multi-worker tests explicitly.
- **Embedder impact.** Embedders calling `cljrs_async::init` get `W = 1` (today's behavior); opting
  into a pool is explicit.

---

## Relationship to other crates

- **`cljrs-async`** — owns the pool (this plan).
- **`cljrs-gc`** — unchanged for Phases 1–5; owns the deferred TLAB / share-nothing endgame.
- **`cljrs-net`** — consumes the Phase 5 `Send`-handoff API to pin connections (see
  `networking-plan.md` Phase H, which points here).
- **`cljrs-io`** — file I/O tasks become poolable for free once spawn routing (Phase 2) lands; no
  `cljrs-io` change required.

---

## Phase summary

| Phase | Deliverable | Crate |
|---|---|---|
| 1 | `WorkerPool` scaffolding; `W=1` default; per-worker runtime + mutator + GC-service | `cljrs-async` |
| 2 | Spawn routing across workers; parent-inherits-worker; per-worker locality | `cljrs-async` |
| 3 | Cross-worker channel audit (`CljChannel::Trace`, wakeups, STW stress) | `cljrs-async` (+ `cljrs-gc` tests) |
| 4 | Safepoint coverage for long native calls | `cljrs-async`, `cljrs-interop` |
| 5 | `Send`-handoff / `spawn_on(worker, …)` API for external producers | `cljrs-async` |
| — | **Deferred:** TLABs, then share-nothing per-worker heaps | `cljrs-gc` (separate plan) |
