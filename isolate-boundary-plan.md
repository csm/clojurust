# Isolate Boundary Crossing — Visibility & the Copy Cost

## Why this document exists

`async-worker-pool-plan.md` settles *how* isolates work (per-isolate heaps, `!Send` `GcPtr`, a
structured-clone copy boundary at B2, a shared static arena at B3). What it does **not** pin down is
the **programming model around the boundary**: when a Clojure programmer sends a value to another
isolate, how do they know they just paid a deep copy — and how do they find out when it's expensive
or impossible?

This document records two decisions that came out of reviewing B2:

1. **Land the first version of isolates with deep-copy-on-send** (B2 as written). No `shared-vec`,
   no refcount fast path yet. The copy cost is acceptable for a first cut, *provided the boundary is
   honest* — visible in source, metered in telemetry, and locating its own failures.
2. **Defer the zero-copy optimization** (`shared-vec` et al.) to a later phase, with the design
   sketched here so the boundary we ship now slots it in later without rework.

Read `async-worker-pool-plan.md` first (the ADR and Model B). This doc lives beside it.

---

## The decision: deep-copy-on-send is fine to ship

The `!Send` wall (A1) forces *every* cross-isolate value through one structured-clone seam (B2) —
nothing can leak a `GcPtr` around it. So a value crossing the boundary is deep-copied into the
receiver's heap, full stop. For a first version this is the right amount of machinery:

- It is **correct and simple** — one code path, no second value representation, no promotion rules.
- The cost is **bounded and local** — copying a per-request value once per recipient, on a seam that
  already has to exist.
- It is the **honest default** the rest of the plan is built on: local `atom`/collections stay
  GC-bump-allocated and fast; sharing is something you opt into, not something every value pays for.

The only requirement we put on this first version is that the copy not be *invisible*. The rest of
this document is that requirement.

---

## Boundary visibility

"Is the crossing visible?" is really two questions with different answers.

### Source-visibility — can the programmer tell a crossing happens?

The `!Send` wall makes the boundary a hard fact at the Rust layer, but there is no `Send` bound for
the user to see at the Clojure layer. Source-visibility is therefore whatever the **API surface**
makes it. The lever is one rule:

> **Distinct-at-construction, not per-message.**

Model it on Erlang's `Pid ! Msg`: a send copies, and you know it copies because the thing you are
sending *to* is a process handle — not because you annotated the message. Translate that here:

- In-isolate channel: `(chan)` — free, no copy.
- Cross-isolate channel / isolate handle: a **distinct constructor** (`(isolate-chan)`, sending to a
  spawned-isolate handle, etc.) — this is the boundary.

The programmer does **not** tag every `>!`. The *target they are holding* tells them whether a copy
is coming. "Am I about to copy?" is answered by looking at a value they named, not by reasoning
about the scheduler.

### The trap: do not make `future`/`agent` polymorphic across the boundary

This is where the boundary goes invisible exactly where it is most expensive. Per
`async-worker-pool-plan.md` (the `future`/`agent` behavior-change note), in **Model A** a `future`
is loop-async — same isolate, no copy. In **Model B** the same conceptual operation could be
re-parallelized onto another isolate — a copy, and a possible can't-cross failure. If both modes
share one `future` surface, identical source is sometimes free and sometimes a deep copy, decided by
scheduling the user cannot see.

**Decision:** parallel-across-isolates is a **distinct primitive** (e.g. `pfuture` / `spawn` / an
explicit pool-or-isolate argument), never a silent re-interpretation of the loop-async `future`.
This is consistent with the plan's stated stance ("pure-Clojure compatibility is not a constraint;
we add new primitives on top where they make sense") — introducing a distinct parallel primitive is
in-bounds, not a workaround.

### Cost-visibility — how much, and when does it fail?

Everything funnels through the one clone seam, so both of these are cheap to provide there:

- **Meter it.** Record per-crossing **bytes copied** and **time**, fed into the same
  `GcStats`/coordinator channel the plan already runs for memory-pressure signaling. A fan-out that
  silently deep-copies a 2 MB map to 8 workers then shows up as a number rather than mystery
  latency. This same metric later tells you *which* value is worth making zero-copy.
- **Locate the failure.** Non-shareable values (a closure capturing isolate-local state, an
  isolate-bound `Resource`/FD) cannot cross. With the distinct-target API the error fires **at the
  send site** — "cannot send value holding `<resource>` across isolate boundary" — instead of
  surfacing deep inside scheduling. The explicit API localizes the error, not just the cost.

### Summary of what the first version must guarantee

1. Crossing happens only through an operation the user **typed** (a distinct cross-isolate target).
2. The parallel primitive is **distinct** from the loop-async `future`.
3. The clone seam is **metered** (bytes + time) into the existing coordinator.
4. Can't-cross values raise a **located** error at the send site.

These four keep "I just crossed an isolate boundary" honest without `shared-vec` needing to exist.

---

## Deferred: the zero-copy fast path (`shared-vec`)

Recorded here so the boundary we ship now is forward-compatible. **Not** part of the first version.

### The idea

A deep copy is forced only because the value lives in a tracing-GC heap owned by one isolate. An
**`Arc`-backed, `Send + Sync`, acyclic** representation of the same immutable collection crosses by
`Arc::clone` — a refcount bump, zero structural copy — and both isolates read it directly. This is
the `SharedValue` representation the ADR already designs for `shared-atom`/var-roots; the
optimization is to let it also be the **payload form at the B2 boundary**, not just for shared
mutable cells.

The chosen shape is **explicit, opt-in construction**:

- `shared-vec` / `shared-map` (or values placed into a `shared-atom` / cross-isolate channel) are
  **born** in the `Arc` representation, so no promotion pass is needed.
- Adding a non-shareable value (a `GcPtr`-bearing element, a closure capturing isolate-local state,
  an isolate-bound resource) **demotes** the result to the ordinary GC-backed collection — the
  always-safe direction, since `GcPtr` is the more capable representation. Demotion is a
  representation rebuild of the retained spine, not a tag flip, so it is a cost paid at a known,
  opt-in call site.

The boundary itself does not move: a `shared-vec` simply crosses by refcount instead of by copy.
Telemetry from the metered clone seam is what tells you which values to promote to this form.

### Rejected as a *default*: Arc-backed-by-default for all collections

We considered making *every* `conj`/`assoc` produce an `Arc`-backed collection, demoting to
`GcPtr` only on contamination. Rejected as the default, kept available as the explicit `shared-vec`
behavior:

- **Refcount cost, corrected.** Thanks to structural sharing, a `conj`/`assoc` does **not**
  re-increment the whole collection — path-copying allocates O(log n) new nodes, and each shared
  subtree is kept alive by a *single* `Arc` bump on its root (no recursion into the subtree). So the
  per-op refcount traffic is O(B·log n) worst case — same *order* as the pointer-array copying the
  operation already does, and amortized far less for tail-optimized vectors. The cost is a
  **bounded constant factor** (atomic RMW instead of a plain move, inline rather than GC-deferred
  frees), not an asymptotic blowup. This was initially overstated; corrected here.
- **But the constant factor is still non-zero vs `GcPtr`**, which has *no* refcount traffic at all
  (plain moves, deferred dealloc). Rust's `im` defaults to `Rc` and uses `Arc` only when `Send` is
  needed for exactly this reason; clojurust's bump-allocated `GcPtr` default sits even further from
  `Arc` than `Rc` does.
- **Multicore contention.** A genuinely-shared tree whose nodes are concurrently retained/dropped
  from multiple isolates writes the refcount word from multiple cores → cache-line bouncing on the
  refcount field. Per-isolate `GcPtr` avoids this entirely — and avoiding exactly this kind of
  cross-core traffic is *why* the isolate model exists.
- **Demotion-on-contamination** is a latent O(spine) rebuild at an unpredictable call site, and in
  general Clojure (functions-in-data, atoms-in-maps) contamination is common.
- **Representation doubling** across every `Value` match site (or a per-node `Gc | Arc` tag check on
  every deref) is a permanent surface-area cost.

Confining all of this to an explicit `shared-vec` keeps the hot local path at zero refcount traffic
and makes "this value is built to be shared" a choice the programmer makes, with the cost attached
to a name they typed. Whether Arc-by-default is viable for a pure data-plane workload is a
benchmark question, not settled against — but it is not the default we ship.

---

## Phase fit

| Step | Deliverable | Status |
|---|---|---|
| B2 (this doc) | Deep-copy-on-send + the four visibility guarantees (typed target, distinct parallel primitive, metered seam, located errors) | **first version — ship it** |
| later | `shared-vec`/`shared-map` born-`Arc` payloads, demote-on-contamination; same boundary, refcount instead of copy | deferred, design recorded above |

Landing the first version — isolates that run in parallel, each collecting independently, with an
honest and observable copy boundary — is a real step forward on its own. The zero-copy fast path is
an optimization layered on a boundary that already exists, guided by the telemetry the first version
produces.
