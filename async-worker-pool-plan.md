# Async Concurrency Plan for clojurust — Isolates (A→B)

## Why this document exists

`cljrs-async` today drives all async work on a **single** Tokio `current_thread` + `LocalSet`
executor. One interpreter thread cannot use more than one core, so any "server" built on it is a
demo. This plan makes async work scale across cores **without** an unsafe shared-heap discipline,
by moving to an **isolate** model:

- **Model A (first step):** a single interpreter/heap thread, plus a Rust worker pool that does only
  `Send` work on non-GC data (socket I/O, TLS crypto, compression, hashing) and hands `Send` results
  back. `GcPtr` becomes **honestly `!Send`** — the `unsafe impl Send/Sync` is deleted. Scales
  I/O-bound serving; Clojure logic stays on one core.
- **Model B (target):** independent per-worker **isolates**, each with its own heap, runtime,
  `LocalSet`, and collector, **collected in parallel**. Crossing an isolate boundary is a **copy**,
  not a shared pointer — enforced by `!Send`. A **shared static arena** holds immortal, immutable
  data (code, interned keywords/symbols, large blobs) so isolates don't copy the world.

This supersedes the earlier shared-heap, multi-mutator design. That design was the only option that
(a) kept the `unsafe` `Send` impl, (b) gave *serialized* global STW instead of parallel collection,
and (c) had a single-heap allocation-mutex ceiling. Isolates remove all three.

Read `async-plan.md` first for the existing executor (`spawn_future`, `await_value`, the
GC-service task, safepoints). `networking-plan.md` Phase H references this document.

---

## The shape of the decision

These three facts are one decision, not three:

1. **Parallel collection requires independent heaps.** A single shared heap can only be collected
   stop-the-world (park every mutator, one collector). "Collected in parallel" ⇒ per-isolate heaps.
2. **Independent heaps make `Send`/`Sync` honest.** If a `GcPtr` never crosses a thread, it is
   genuinely `!Send` — no `unsafe impl`, and transients (deliberately thread-confined, unsynchronized)
   stop being a soundness hole. Cross-isolate transfer is a copy.
3. **Cross-thread memory pressure becomes observable and actionable per-isolate** instead of a
   single global cliff (see "Memory pressure").

So the model is: **run today's single-threaded executor, N times, with a copy boundary between
instances and a shared immortal arena underneath.**

---

## What in the code makes this feasible (and what fights it)

**Feasible:**

- **`static_arena.rs` already exists**, is `Send + Sync`, and is program-lifetime / never swept —
  this is the shared immortal arena (Model B) ready-made. `is_static_addr(addr)` already lets a
  collector recognize a pointer into it and **stop tracing** there, so a per-isolate collector can
  safely ignore shared-arena references.
- **Interior mutability is already synchronized** — `LazySeq.state`, `Atom`, `Var`, `Multimethod`,
  `Namespace` are all `Mutex`-guarded. So shareable immutable values placed in the static arena are
  safe to read from any isolate.
- **The executor is already single-threaded per instance** (`spawn_local`, `current_thread`). An
  isolate *is* today's executor; we instantiate it N times instead of changing its internals.

**Fights it (must be migrated):**

- **`GcPtr: Send + Sync` is currently `unsafe impl`'d** (`cljrs-gc/src/lib.rs:302–303`) and several
  paths rely on it: the `future`/`agent` builtins `thread::spawn` + `register_mutator()` and allocate
  off the main thread; `register_mutator` is called at `crates/cljrs/src/main.rs:291`. Deleting the
  impl is a **forcing function** — flip it and the compiler enumerates every cross-thread `GcPtr`
  move to migrate.
- **`future`/`agent` semantics.** In Model A a `future` body running Clojure code cannot run on
  another thread (no off-thread `GcPtr`). Like JS, `future` becomes loop-async; *parallel* execution
  needs an isolate (Model B). This is a behavior change to document.

---

## Guiding principle: pure-Clojure compatibility is not a constraint

clojurust does **not** aim to reproduce JVM-Clojure's concurrency semantics bit-for-bit. Where an
honest separation of concerns conflicts with Clojure compatibility, separation wins, and we add
new primitives on top where they make sense. The two-tier atom below is the first concrete
application of this: rather than force one `atom` to satisfy both fast local use *and* cross-isolate
sharing, we keep `atom` local-and-fast and add `shared-atom` as an explicit, separate tool.

---

## ADR: vars/atoms across isolates — two-tier, decided

Clojure's vars and atoms are globally-shared mutable references; the isolate model is share-nothing.
**Resolved (this is no longer open):** a **two-tier** design, with `Arc` providing the shared cell
and a `Send + Sync` value representation providing the shared contents.

### Tier 1 — `atom` (isolate-local, the fast common case)

Unchanged from today: a GC-heap-backed `Mutex<Value>` confined to one isolate. Bump-allocated,
no refcount churn, `!Send` like everything else in an isolate. This is what `(atom …)` returns.

### Tier 2 — `shared-atom` (explicit, cross-isolate)

A new primitive for genuinely shared mutable state:

```
SharedAtom  = Arc<ArcSwap<SharedValue>>          ; Send + Sync; lock-free cross-isolate CAS
SharedValue = immediate                           ; long/double/bool/nil/char — no allocation
            | Arc<…>                               ; refcounted, acyclic, immutable structure
            | ResourceHandle | static-arena ref    ; existing Send+Sync escape hatches
```

- **Cell:** `Arc<ArcSwap<…>>` gives identity, lifetime, and **lock-free atomic `swap!`/`reset!`**
  across isolates — exactly an atom's CAS-retry semantics, off the shelf.
- **Contents:** must be `Send + Sync`. Immutable persistent collections are **acyclic DAGs**, so
  plain `Arc` refcounting is sufficient and leak-free — no tracing GC needed for shared values. This
  reuses the pattern already in the tree: `ResourceHandle` is `Arc<dyn Resource>` and `static_arena`
  is program-lifetime `Send + Sync`; `SharedValue` is the third instance of "an Arc/arena thing
  beside the GC, not inside it."
- **`Value` gains a shared variant** that points at a `SharedValue` (the same way `Value::Resource`
  wraps an `Arc`), so both isolates' interpreters read it directly — no copy on read; `deref` is an
  atomic refcount bump.
- **Promote-on-publish:** `swap!`/`reset!` deep-copy any isolate-local parts of the new value into
  `SharedValue` form before the atomic store (zero-copy if already shared). Reads are cheap.

### Constraints (all the same "plain data only" rule)

1. **Transitive shareability.** Everything reachable from a `shared-atom` must be shareable:
   immediates + Arc/arena data. Values holding a **closure that captured isolate-local state** or a
   **native resource bound to one isolate** cannot be published — same rule as message-copying.
2. **Cycles leak.** Pure `Arc` refcounting leaks cycles, and a `shared-atom` holding another mutable
   ref could build one. **Start by restricting `shared-atom` to hold immutable acyclic data only**
   (no embedded refs/closures); revisit with a small dedicated collector for the shared tier only if
   ref-holding-ref is ever needed.
3. **Promotion cost** is paid once on publish; reads are cheap. Local `atom` avoids all of it.

### Vars

A var's root binding is the same shape as a shared mutable cell, so it uses the **same mechanism**:
`Arc<ArcSwap<SharedValue>>`, promote-on-`def`. `def` is comparatively rare and global by nature, so
the promotion cost is acceptable. Dynamic `binding` is already thread-local and maps directly to
per-isolate — no change.

### Immortal shared state (orthogonal, also decided)

Code (compiled fns, namespaces), interned keywords/symbols, and large `byte-array` blobs live in the
shared **static arena** and are referenced from every isolate with **no copy** (see B3). This is
separate from the atom tiers and not in tension with anything.

### Rejected alternative

**Uniform Arc-backed atoms** (one tier, all atoms cross-isolate-capable). Rejected: every atom would
pay refcounting + promotion and lose GC bump-allocation speed, and the cyclic-leak problem would
become universal rather than confined to an opt-in primitive. The two-tier split keeps the common
case fast and makes sharing an explicit, honest choice.

---

## Model A — single heap + `Send` worker pool

The safe, small first step. Delete the unsafe impl; parallelize only `Send` work.

### A1 — Make `GcPtr` honestly `!Send`

- Remove `unsafe impl Send/Sync for GcPtr` (and the supporting `GcBoxHeader` impls where they exist
  only to enable cross-thread `GcPtr`). Let the compiler list every violation.
- Migrate `future`/`agent` off `thread::spawn`+`register_mutator`: `future` bodies run as loop-async
  tasks on the heap thread; CPU-parallel work is redirected to the Send pool (A2) or deferred to
  isolates (Model B).
- Keep exactly one `register_mutator()` (the heap thread). STW becomes a single-thread cooperative
  pause again — simpler than today.

**Done when:** the tree builds with `GcPtr: !Send`, all async tests pass, and no code moves a
`GcPtr` across a thread.

### A2 — `Send` worker pool for non-GC work

- A `tokio` multi-thread runtime (or `rayon`) used **only** for `Send` work: socket read/write into
  `Vec<u8>`, TLS handshake/encrypt/decrypt (rustls operates on byte buffers — `Send`), compression,
  hashing, byte-level regex.
- Results return as `Send` data over a oneshot/mpsc to the heap thread, which builds the `GcPtr`
  (`byte-array`, etc.). This is the seam `cljrs-net` uses: the socket lives in the pool as `Vec<u8>`
  traffic; the heap thread only ever sees byte-arrays it constructed.

**Done when:** TLS bulk transfer and compression run on pool threads while the heap thread stays
responsive; an I/O-bound benchmark scales with pool size though Clojure logic stays single-core.

---

## Model B — independent isolates (parallel collection)

An isolate = today's single-threaded executor + its own heap + its own collector. Run N of them.

### B1 — De-globalize the heap into per-isolate heaps

- Replace the `static HEAP: GcHeap` singleton with a **per-isolate** heap (thread-local or
  instance-handle threaded through the allocator). The static arena stays global and shared.
- Each isolate: own `current_thread` runtime + `LocalSet`, own heap, own collector, own root stacks
  (the root stacks are *already* thread-local — `ENV_ROOTS`, `BINDING_STACK`, `ALLOC_ROOTS`,
  `REGION_STACK` — so this part is nearly free).
- Collection is now **fully parallel and independent**: an isolate collects its own heap with no
  global STW and no cross-isolate coordination. A collector that encounters a static-arena pointer
  (`is_static_addr`) stops there; it can never see another isolate's heap because nothing is shared.

**Done when:** two isolates run concurrent allocation-heavy workloads and each GCs independently;
total throughput scales ~linearly with isolate count (no shared alloc lock, no global pause).

### B2 — The copy boundary

- A **structured-clone / serialize step** for moving a value between isolates: deep-copy a shareable
  value from isolate A's heap to isolate B's heap (or to the shared arena). This is the only way a
  value crosses; `!Send` makes "accidentally sharing a pointer" a compile error.
- Reuse / share machinery with a future nippy-like persistent serializer — clojurust wants this
  format anyway for IPC and on-disk values.
- Non-shareable values (holding a native `Resource`/FD) cannot cross; the FD-handoff pattern moves
  the `Send` token instead and rebuilds on the destination isolate (this is exactly the `cljrs-net`
  pinned-connection seam).

**Done when:** a value sent over a cross-isolate channel arrives as an independent copy in the
receiver's heap, verified independently collectable on both sides.

### B3 — Shared immortal arena for code, keywords, blobs

- Interned **keywords/symbols** live in the static arena behind a global `Mutex` intern table, so
  keyword *identity* is consistent across isolates (critical for map lookups). Loaded
  **namespaces/compiled fns** live there too — all isolates run the same code with zero copy.
- Large **`byte-array` blobs** can be arena-allocated and refcounted (the BEAM off-heap-binary
  trick) so big payloads are shared, not copied, across the boundary.
- Implement the two-tier atoms / `shared-atom` / var-root mechanism per the ADR above
  (`Arc<ArcSwap<SharedValue>>`, promote-on-publish; local `atom` stays GC-backed).

**Done when:** isolates share code and keyword identity through the arena and only per-request
mutable state is copied at the boundary.

---

## Memory pressure signaling

- **Per-isolate accounting** plus a global `AtomicUsize` of total live bytes (each isolate adjusts on
  collect). A coordinator watches per-isolate `GcStats` (live set, alloc rate).
- The coordinator drives a `tokio::sync::watch<PressureLevel>` (Green/Yellow/Red) that every isolate
  reads at safepoints. Responses are **local and graduated**: Yellow → collect more eagerly / lower
  the young-gen threshold; Red → **stop taking from `:conns`** (the accept backpressure already in
  `networking-plan.md`) and shed load. Memory pressure reuses the same channel-backpressure
  mechanism as the rest of the system.
- Optional real signals into the coordinator on Linux: cgroup v2 **PSI** memory pressure, RSS
  watermarks.

This is strictly better than the shared-heap cliff: pressure is per-isolate observable and per-isolate
actionable.

---

## Phase summary

| Phase | Deliverable | Crate(s) |
|---|---|---|
| A1 | Delete `unsafe impl Send/Sync for GcPtr`; migrate `future`/`agent`; single registered mutator | `cljrs-gc`, `cljrs-builtins`, `cljrs-async` |
| A2 | `Send`-only worker pool (I/O, TLS crypto, compression, hashing) with `Send`-result handoff | `cljrs-async` |
| B1 | Per-isolate heaps; independent parallel collection; arena pointers skipped via `is_static_addr` | `cljrs-gc`, `cljrs-async` |
| B2 | Copy/structured-clone boundary; `Send`-token handoff for resources (the `cljrs-net` seam) | `cljrs-async`, `cljrs-value` |
| B3 | Shared static arena for code, interned keywords/symbols, refcounted blobs | `cljrs-gc`, `cljrs-value`, `cljrs-env` |
| — | **ADR (decided):** two-tier atoms — local `atom` (GC-backed) + `shared-atom`/var-root (`Arc<ArcSwap<SharedValue>>`, promote-on-publish) | design |

A1+A2 (Model A) ship a safe, faster, `unsafe`-free runtime on their own. B1–B3 (Model B) add
parallel collection and multicore Clojure execution.

---

## Risks & open questions

- **Vars/atoms vs share-nothing** — *decided* (two-tier ADR above). Residual risks are narrow:
  cycle leaks in the shared tier (mitigated by restricting `shared-atom` to acyclic immutable data)
  and promotion cost on publish (mitigated by the shared static arena + work pinning).
- **`future`/`agent` behavior change** — parallel→loop-async in Model A; re-parallelized via isolates
  in Model B. Document clearly; it diverges from JVM Clojure's thread-per-future.
- **De-globalizing `HEAP`** — touches every allocation site; the thread-local root stacks already
  being per-thread limits the blast radius, but the allocator entry points (`GcPtr::new`,
  `alloc_ctx`, regions) must learn "which isolate's heap."
- **Copy cost at the boundary** — mitigated by the shared arena (code/keywords/blobs never copy) and
  by pinning work so most values never cross. Measure; promote hot shared data to the arena.
- **Determinism in tests** — default to one isolate for deterministic runs; gate multi-isolate tests.
- **Keyword interning contention** — the global intern table is a shared lock; keywords are created
  often. May need a sharded or lock-free intern table.

---

## Relationship to other crates

- **`cljrs-gc`** — owns `!Send` `GcPtr` (A1), per-isolate heaps + the shared arena (B1/B3).
- **`cljrs-async`** — owns the `Send` worker pool (A2) and isolate runtimes (B1) and the copy
  boundary (B2).
- **`cljrs-value`/`cljrs-env`** — structured-clone (B2), arena-resident keywords/code/vars (B3).
- **`cljrs-net`** — consumes the `Send`-token (FD) handoff to pin a connection to an isolate; see
  `networking-plan.md` Phase H.
- **`cljrs-io`** — file I/O becomes pool/isolate-friendly for free; no change required.
