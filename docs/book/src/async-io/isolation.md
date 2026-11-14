# Worker isolation

The async tier described in [core.async](async.md) runs on a **single** executor
thread. That keeps the garbage collector simple, but one interpreter thread can
only use one CPU core. To scale Clojure work across cores, clojurust uses an
**isolate** model: each worker is an independent execution context with its own
heap, its own collector, and its own `current_thread` + `LocalSet` executor.
Isolates share no GC pointers. A value moves between them by being **copied**,
never by being aliased.

This chapter explains how that differs from Clojure, why the boundary is an
explicit copy instead of transparent sharing, and how to work with it.

## What an isolate is

An isolate is just "the single-threaded async runtime from the previous chapter,
instantiated N times." Each one:

- owns a **private GC heap** (a thread-local) and collects it **independently** —
  no global stop-the-world, no cross-isolate coordination;
- runs its own Tokio `current_thread` runtime and `LocalSet`, so `go` blocks,
  channels, and `^:async` functions behave exactly as they do in a single-isolate
  program;
- references the shared **static arena** (compiled code, interned
  keywords/symbols) with no copy, so isolates run the same program without
  duplicating it.

Because nothing is shared between heaps, two isolates collect garbage in
parallel and throughput scales with the number of isolates rather than bottlenecking
on one collector.

## Differences from Clojure

JVM Clojure assumes one shared, globally-visible heap. Several of its concurrency
features lean on that assumption; the isolate model deliberately does not
reproduce them bit-for-bit.

| Clojure (JVM) | clojurust |
|---|---|
| `future` runs the body on **another OS thread**, sharing the heap | `future` runs in a **fiber** on the same isolate — concurrent, not parallel. Cross-core parallelism is a separate, explicit step. |
| `atom`, `ref`, and var roots are **globally shared** mutable cells any thread can see | `atom` is **isolate-local** and fast. Genuinely shared mutable state is a distinct primitive, [`shared-atom`](#shared-atom-cross-isolate-mutable-state). |
| `(thread ...)` / `(future ...)` capture local variables and run them on another thread | A new isolate is started by **name**: `(spawn-isolate 'my.ns/f arg ...)`. It loads its own copy of the code; nothing is captured. |
| Passing a value to another thread shares a **pointer** — zero copy, but also zero isolation | Passing a value to another isolate **deep-copies** it. The two sides are fully independent afterward. |
| Any value (closures, refs, mutable objects) can be handed to another thread | Only **plain, immutable, acyclic data** can cross an isolate boundary; everything else is rejected at the send site. |

The guiding stance (from `docs/async-worker-pool-plan.md`) is that **pure-Clojure
compatibility is not a hard constraint**: where an honest separation of concerns
conflicts with reproducing JVM semantics exactly, separation wins, and clojurust
adds new primitives on top where they make sense. `shared-atom` and the isolate
channels below are those primitives.

## Why explicit copying instead of transparent sharing

The decision to copy at the boundary — rather than share pointers like the JVM —
falls out of one technical fact and one design value.

**The technical fact: `GcPtr` is `!Send`.** Every Clojure value lives behind a
`GcPtr`, which is a raw heap pointer into one isolate's heap. It is honestly
**not safe to send across threads**, and the Rust type system enforces that. You
*cannot* accidentally leak a pointer from one isolate into another — it is a
compile error, not a runtime hazard. This is the same property that lets
transients and the single-threaded collector stay unsynchronized and fast.

So if a value is going to cross from one heap to another, it has to be
reconstructed on the far side. That reconstruction is the copy. There is no
"share it instead" option to fall back to — the alternative was an `unsafe`
shared-heap discipline, which was rejected because it forced serialized global
GC pauses and a single allocation bottleneck.

**The design value: no invisible surprises.** Given that crossings cost a copy,
the remaining question is whether the programmer can *see* the cost. The plan
(`docs/isolate-boundary-plan.md`) commits to four guarantees so the boundary is
honest:

1. **Crossing only happens through an operation you typed.** You send through a
   *distinct* construct (an isolate channel), so a copy never hides inside an
   ordinary `(chan)` or function call. You know a copy is coming because of the
   *target you are holding*, not because you annotated the message — the same way
   Erlang's `Pid ! Msg` tells you a copy happens because you are sending to a
   process.
2. **The parallel primitive is distinct from `future`.** The same source must not
   be a cheap loop-async task in one place and a silent deep copy onto another
   isolate in another. Parallel-across-isolates is its own primitive, never a
   re-interpretation of `future`.
3. **The copy is metered.** Every accepted crossing records bytes copied and time
   into `GC_STATS`, visible via `--gc-stats`. A fan-out that deep-copies a 2 MB
   map to eight workers shows up as a number, not as mystery latency.
4. **Can't-cross failures are located.** A value that cannot cross raises an error
   **at the send site**, naming the offending type — not deep inside the
   scheduler.

The trap this avoids is the one where identical-looking code is sometimes free
and sometimes an expensive deep copy, decided by scheduling you cannot see. The
explicit boundary trades a little ceremony for the property that cost and failure
are always attached to something you wrote.

Isolates are started from Clojure with
[`spawn-isolate`](#spawning-an-isolate), described after the channels it relies
on.

## Isolate channels — the copy boundary

An **isolate channel** is the sanctioned way to move a value between isolates. It
is a *distinct constructor* from `(chan)`, precisely so the crossing is visible
in source.

```clojure
(require '[clojure.core.async :refer [isolate-chan isolate-put! isolate-poll! isolate-take!]])

(let [[tx rx] (isolate-chan)]
  (isolate-put! tx {:a 1 :b [2 3]})  ; deep-copies the map across the boundary → true
  (isolate-poll! rx))                ; => {:a 1 :b [2 3]}  (an independent copy)
```

| Operation | Meaning |
|---|---|
| `(isolate-chan)` / `(isolate-chan limit-bytes)` | create a channel, returning `[tx rx]`. `tx` is multi-producer (cloneable); `rx` is single-consumer. The channel holds at most `limit-bytes` of queued messages (default: `CLJRS_ISOLATE_QUEUE_LIMIT_MB`, 64 MiB at most). |
| `(isolate-put! tx v)` | deep-copy `v` across the boundary and enqueue it. `true` on success, `false` if the receiver is gone, **throws** if `v` can't cross or the channel is full. |
| `(isolate-poll! rx)` | non-blocking take: the next value (copied into *this* isolate's heap), or `nil` if empty/closed. |
| `(isolate-take! rx)` | a `Future` resolving to the next value, or `nil` once closed and drained. Use with `await` in a `go`/`^:async` body. |

```clojure
;; park until a value arrives, inside an async body:
(go (let [msg (await (isolate-take! rx))]
      (handle msg)))
```

The receiver deserializes into the heap of whichever isolate holds it, so keep
`rx` on the isolate that will consume from it. The sender can be cloned and used
from anywhere.

## Spawning an isolate

`spawn-isolate` starts a new isolate on its own OS thread and returns a `Future`
of its result. It is the parallel primitive, and it is deliberately not
`future` or `thread`.

```clojure
(ns my.app
  (:require [clojure.core.async :refer [spawn-isolate]]))

;; four cores, four isolates
(let [jobs (mapv #(spawn-isolate 'my.work/crunch %) [chunk-a chunk-b chunk-c chunk-d])]
  (mapv deref jobs))
```

The entry point is a **namespace-qualified symbol**, not a function. An isolate
shares no heap with its parent, and that includes loaded code: a closure is a
heap object that points at its captured values, so it cannot be handed over. A
name can. The new isolate builds a fresh runtime, `require`s `my.work`, looks up
`crunch`, and calls it with copies of the arguments. This is the model of
Erlang's `spawn(Module, Function, Args)` and of a Web Worker, not of a JVM
thread.

What follows from that:

- **The namespace must be loadable.** `my.work` has to be on the source path (or
  embedded in the binary). A function defined at the REPL, or in a script file
  that `require` cannot find, exists only in the isolate that defined it and
  cannot be an entry point.
- **Loading runs top-level forms again.** Each isolate evaluates the namespaces
  it requires. Keep side effects out of the top level of a worker namespace, as
  you would for any namespace that is loaded more than once.
- **Nothing is captured.** Everything the worker needs arrives as an argument.
  Arguments and the result are deep-copied with the rules in
  [What can and cannot cross](#what-can-and-cannot-cross), and metered. An
  argument that cannot cross throws at the `spawn-isolate` call.
- **State is per isolate.** A `def`'d atom in `my.work` is a different atom in
  each isolate. Share state with a `shared-atom` passed as an argument.
- **Dynamic bindings are not conveyed.** The worker starts with root bindings.

`deref` (or `await`) on the returned future parks the calling fiber, so the
parent isolate keeps running its other fibers. If the entry function throws, the
future fails; an `ex-info` arrives with its message and data.

### Talking to a running isolate

Pass isolate-channel ends as arguments. A channel end given as a **direct
argument** crosses as itself rather than as a copy: a sender is cloned, and a
receiver *moves* to the new isolate.

```clojure
(ns my.work
  (:require [clojure.core.async :refer [isolate-put! isolate-take!]]))

(defn serve [jobs results]
  (loop []
    (when-let [job @(isolate-take! jobs)]   ; nil when the parent drops its sender
      (isolate-put! results (handle job))
      (recur))))
```

```clojure
(let [[jobs-tx jobs-rx]       (isolate-chan)
      [results-tx results-rx] (isolate-chan)
      worker (spawn-isolate 'my.work/serve jobs-rx results-tx)]
  (isolate-put! jobs-tx {:id 1})
  @(isolate-take! results-rx))
```

After the spawn, `jobs-rx` belongs to the worker. Using it in the parent throws.
A channel end nested inside a collection is not moved; it is rejected like any
other native object.

### Limits

- **No `thread` with captured variables.** There is no form that runs an
  arbitrary closure on another core. That would need closures whose code and
  captures are both shareable, which the runtime does not have.
- **Startup cost.** Each isolate bootstraps `clojure.core` and loads its
  namespaces. Expect tens of milliseconds, and memory for a second copy of the
  loaded code. Spawn a few long-lived workers, not one isolate per small task.
- **Detached.** An isolate ends when its entry function returns. The process
  does not wait for running isolates at exit, and interrupting the parent's
  evaluation does not stop them. Close the channel a worker reads from to ask it
  to finish, and `deref` its future to wait.
- **Gas and interrupts are not inherited.** For that reason a runtime can spawn
  only if its host allows it. The `cljrs` CLI does. An embedded runtime must
  install an isolate factory (`GlobalEnv::set_isolate_factory`); without one
  `spawn-isolate` throws, so a sandboxed guest cannot leave its metered isolate.
- **Not in AOT binaries or on wasm.** A binary from `cljrs compile` and the
  wasm build do not install a factory yet.

### What can and cannot cross

Only **plain, immutable, acyclic data** can be copied across the boundary.
`isolate-put!` (and `shared-atom`) accept:

- all scalars: `nil`, booleans, longs, doubles, chars, bigints, ratios, UUIDs;
- strings, symbols, keywords, and compiled regex patterns (by source);
- all persistent collections — lists, vectors, maps, sets, queues, cons cells —
  and records, recursively;
- primitive and object arrays (snapshotted), and error values (message + data +
  cause chain);
- **realized** lazy sequences (they are forced first, then the result is copied).

The following hold isolate-local state and are **rejected at the send site**:

- mutable cells — `atom`, `volatile`, `var`, `promise`, `future`, `agent`;
- **functions and macros** of every kind — a closure captures `GcPtr`s from its
  home isolate, so it cannot be reconstructed elsewhere;
- native `Resource`s and `NativeObject`s (OS handles, channels — bound to one
  isolate);
- transients and an **unforced** `delay` (deliberately thread-confined).

The error is located and names the type, e.g.:

```clojure
(isolate-put! tx (fn [] 1))
;; => throws: isolate-put!: value of type `fn` cannot cross an isolate boundary;
;;    the value holds isolate-local state and cannot cross an isolate boundary
```

If you need to hand work to another isolate, send it **data** describing the work
(a keyword tag, a vector of arguments) and have the receiving isolate dispatch to
code it already has — not a closure.

## `shared-atom` — cross-isolate mutable state

For genuinely shared mutable state, use `shared-atom`. It is the second tier of a
deliberate two-tier design: `atom` stays local and fast; `shared-atom` is the
explicit, opt-in tool for sharing.

```clojure
(def counter (shared-atom 0))

(swap! counter inc)
(swap! counter + 10)
@counter                      ; => 11

(compare-and-set! counter 11 0)  ; lock-free CAS, like a normal atom
(shared-atom? counter)           ; => true
(atom? counter)                  ; => false — it is a distinct type
```

It supports the full atom surface — `deref`/`@`, `reset!`, `swap!`,
`compare-and-set!` — with **lock-free** atomic updates that are safe across
isolates. Under the hood the cell is an `Arc<ArcSwap<…>>` and its contents are a
`Send + Sync` value representation, so the same reference can be handed to another
isolate (it crosses an isolate channel by a refcount bump, not a deep copy) and
mutated concurrently from both.

The cost is paid on **write**: each value stored is *promoted* into the shared
representation. The same shareability rule applies — you can only publish plain,
immutable, acyclic data. Storing a closure or other isolate-local value into a
`shared-atom` fails, and a failed `swap!` leaves the atom unchanged:

```clojure
(def a (shared-atom 0))
(swap! a (fn [_] (fn [] 1)))   ; throws — the new value is a closure
@a                              ; => 0  (swap did not take effect)
```

Use `shared-atom` only where you actually need cross-isolate sharing; keep the
common case on a local `atom`, which avoids all promotion and refcount traffic.

## The `Send` worker pool

Not every parallel task needs a whole isolate. Byte-level work that touches **no
Clojure values** — socket reads and writes, TLS handshakes and bulk crypto,
compression, hashing — runs on a multi-threaded `Send`-only **worker pool**
instead. The heap thread offloads such work and `await`s the result, which comes
back as plain `Send` data (`Vec<u8>`, a string) that the heap thread turns into a
Clojure value.

This is the seam `cljrs-net` uses: a socket lives in the pool as byte traffic,
and the isolate's interpreter only ever sees byte-arrays it constructed itself.
The pool is a Rust-level facility (`WorkerPool`), used by native crates rather
than called directly from Clojure; it lets I/O-bound servers scale across cores
while Clojure logic stays on its isolate.

## Using isolation effectively

- **Default to local.** Plain `atom`, `future`, channels, and collections are
  bump-allocated and fast. Reach for isolates and `shared-atom` only when you
  need real multicore execution or cross-isolate sharing.
- **Send data, not behavior.** A closure cannot cross. Name the entry point
  (`spawn-isolate 'my.ns/f`), or pass a tag plus arguments and let the receiving
  isolate dispatch to code it has loaded.
- **Keep the receiver pinned.** An isolate-channel `rx` deserializes into its own
  heap; consume from it on the isolate that owns it.
- **Make crossings coarse.** Each crossing is a deep copy. Prefer sending one
  larger message over chattering many small ones, and don't fan a large value out
  to many workers without expecting the copy cost.
- **Watch the meter.** Run with `--gc-stats` to see bytes copied and time spent
  at the boundary. If one value dominates, that is the value to restructure (or,
  later, to make zero-copy).
- **Let failures guide you.** A located "cannot cross" error usually means a
  closure or a stateful object slipped into your message. Replace it with plain
  data.

## Looking ahead

The boundary that ships today is **deep-copy-on-send** with the four visibility
guarantees above. A future phase adds a **zero-copy fast path** — explicitly
constructed `shared-vec`/`shared-map` values that are born in the `Arc`-backed
shared representation and cross by refcount instead of by copy, demoting back to
ordinary GC-backed collections the moment they hold something non-shareable. The
boundary itself does not move; the telemetry from the metered seam is what will
tell you which values are worth promoting to that form. See
`docs/isolate-boundary-plan.md` for the full design.
