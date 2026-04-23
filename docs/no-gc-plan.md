# Plan: `no-gc` Compilation Mode (Strict Memory)

## Overview

A `no-gc` Cargo feature that replaces all GC machinery with two deterministic
allocation strategies and a small blacklist of forbidden operations.

| Context | Allocator | When active |
|---|---|---|
| **Static** | `StaticArena` (never frees) | Top-level evaluation, static sinks |
| **Region** | Bump allocator (reset on scope exit) | `loop` iterations |

**The rule in one sentence**: every `loop` iteration owns a bump-allocator
region; everything else (function calls, `let`, `do`, `if`, etc.) is
transparent and inherits whatever context the caller has active.

Because the program starts in static context, all top-level code and any
function called from it allocates statically — no annotation needed. Regions
only come into play inside `loop` bodies, which is also where tight allocation
pressure exists. A small **blacklist** (see Layer 4) prevents the handful of
operations that would require GC to be safe.

No `GcHeap`, no stop-the-world pauses, no `Trace` impls, no explicit region
management by the programmer.

---

## Layer 1 — Cargo Feature

```toml
[features]
no-gc = [
  "cljrs-gc/no-gc",
  "cljrs-value/no-gc",
  "cljrs-eval/no-gc",
  "cljrs-interp/no-gc",
  "cljrs-env/no-gc",
]
```

Propagates via `dep?/no-gc`. Default build keeps the current GC. Mutually
exclusive compilation paths — not a runtime toggle.

---

## Layer 2 — `cljrs-gc` Changes

### `GcBox<T>` becomes zero-overhead under `no-gc`

```rust
// cfg(not(feature = "no-gc")) — current header-bearing layout
pub struct GcBox<T: ?Sized> {
    header: GcBoxHeader,  // magic, lives, trace_fn, drop_fn, next
    pub value: T,
}

// cfg(feature = "no-gc") — bare value, no header
pub struct GcBox<T: ?Sized> {
    pub value: T,
}
```

`GcPtr<T>` still points to `GcBox<T>` everywhere — minimal `#[cfg]` surface in
the rest of the codebase.

### `GcPtr::new()` dispatches to the active context

```rust
#[cfg(feature = "no-gc")]
pub fn new(value: T) -> GcPtr<T> {
    ALLOC_CTX.with(|ctx| ctx.borrow().alloc(value))
}
```

The thread-local `ALLOC_CTX` stack holds either `Static` or `Region(r)`. The
top entry determines where the allocation lands.

### New `StaticArena` (`cljrs-gc/src/static_arena.rs`)

- Global singleton (`OnceLock<StaticArena>`)
- Thread-safe bump allocator backed by leaked `Box<[u8]>` chunks
- `alloc<T>(value: T) -> GcPtr<T>` — allocates; destructor never runs
- Active context at program startup

### Compiled out under `no-gc`

- `GcHeap`, `GcBoxHeader`, `MarkVisitor`, `Trace` trait and all impls
- Alloc-root thread-local, `AllocRootGuard`
- STW protocol (`cancellation.rs`), safepoints, parking
- `GcConfig`

Existing `Region` and `RegionGuard` are kept as-is.

---

## Layer 3 — Allocation Context Stack

The evaluator maintains a thread-local allocation context stack. `GcPtr::new()`
always allocates into the top entry.

### What pushes a new context

| Construct | Pushed context | Popped / reset when |
|---|---|---|
| Program start | `Static` | never |
| Top-level `def` / `defn` value expr | `Static` | value evaluated |
| `atom` / `agent` / `Var` init expr | `Static` | init evaluated |
| `reset!` / `vreset!` new-value expr | `Static` | new value evaluated |
| fn passed to `swap!` / `vswap!` | `Static` | fn returns |
| `loop` body — each `recur` iteration | `Region(fresh)` | next `recur` or loop exit |

### What does NOT push a context

Function calls, `let`, `do`, `if`, `when`, `cond`, `fn*` bodies, `doseq`,
`for` — all inherit the currently active context. A function always allocates
its return value into the caller's active context, so return values land in the
right place by construction with no promotion or copying.

### Example traces

```clojure
;; Top-level: everything static
(defn make-map [k v] {k v})
(def config (make-map :host "localhost"))
;; Stack at call: [Static]  →  {:host "localhost"} in StaticArena ✓

;; loop: each iteration gets a fresh region
(loop [i 0 acc []]
  (if (= i n)
    acc                        ; acc is in the current region — see blacklist
    (recur (inc i) (conj acc (compute i)))))
;; Stack at compute call: [Static | Region(iter-k)]
;; (compute i) result in Region(iter-k); freed at next recur ✓

;; Nested loops: each loop pushes its own region
(loop [i 0]
  (loop [j 0]
    (process i j)              ; in Region(inner-iter)
    (recur (inc j)))           ; Region(inner-iter) reset here
  (recur (inc i)))             ; Region(outer-iter) reset here
```

### Metadata follows provenance

A value's metadata map is allocated in the same active context as the value.
`with-meta` and `vary-meta` do not push a new context.

---

## Layer 4 — Blacklist

A pure functional Clojure program is largely RAII-safe already. Only a small
set of operations require GC to be correct. These are **forbidden in `no-gc`
mode** and produce compile errors.

### Blacklisted: unrealized lazy seqs at a region boundary

A lazy seq's thunk captures references to the region it was created in. If the
region resets before the seq is forced, the thunk references freed memory.

**Rule**: a lazy seq created inside a `loop` iteration must be fully realized
(via `doall`, `reduce`, `count`, etc.) before the iteration exits. A lazy seq
created in a static context (top-level) is fine — it lives in StaticArena.

```clojure
;; ERROR — map returns unrealized lazy seq; iteration region resets at recur
(loop [i 0]
  (let [xs (map inc data)]
    (recur (inc i))))

;; OK — doall forces before recur
(loop [i 0]
  (let [xs (doall (map inc data))]
    (process xs)
    (recur (inc i))))

;; OK — top-level, static context, thunk lives in StaticArena
(def evens (filter even? (range)))
```

### Blacklisted: region-local values stored in static containers

Top-level `Atom`, `Var`, `Agent`, and namespace interns outlive any `loop`
region. Storing a region-local pointer into them would leave a dangling
reference after the region resets.

**Rule**: values passed to `reset!`, `swap!`, `vreset!`, `vswap!`, or used to
initialise an `atom`/`agent`/`Var` must be static (allocated in StaticArena or
a primitive). Values computed entirely within `reset!`/`swap!`'s own static
context (see Layer 3) satisfy this automatically.

```clojure
;; ERROR — record was allocated in the loop's region
(loop [i 0]
  (let [record (build-record i)]
    (reset! global-log record)
    (recur (inc i))))

;; OK — reset! pushes Static; build-record called in static context
(loop [i 0]
  (reset! global-log (build-record i))   ; build-record runs in Static ctx
  (recur (inc i)))

;; OK — local atom created and used entirely within one iteration
(loop [i 0]
  (let [a (atom (build-record i))]       ; atom and its value in Region
    (swap! a update :count inc)
    (emit! @a))
  (recur (inc i)))
```

### Blacklisted: region-local values captured by escaping closures

A closure that closes over a region-local value and outlives the iteration
would reference freed memory. In practice this is already caught by the lazy
seq rule (the most common escape path is an unrealized lazy seq returned from
`map`/`filter` with a closure over a loop variable). Any remaining case — a
`fn` literal that captures a region-local value and is stored somewhere that
outlives the iteration — is also a compile error.

### What is NOT blacklisted

- Returning heap values from functions — fine, they land in the caller's
  active context
- `let`, nested `fn`, `if`, etc. — fully transparent, no restriction
- Eager seq operations (`mapv`, `filterv`, `into`, `vec`, `reduce`) — produce
  results in the active context immediately, no dangling thunks
- Primitives (`Long`, `Double`, `Bool`, `Nil`, `Char`) — stored inline in
  `Value`, no heap allocation at all
- Statically-created closures (`def`-bound functions, `defn`) — always static

---

## Layer 5 — Pointer Provenance (Debug)

Tag each `GcPtr` with the low bit of the pointer (alignment gives ≥1 free bit):

- `0` = region-local
- `1` = static

In debug builds, write sites for static containers (`reset!`, `swap!`, `atom`
init, namespace intern) assert `is_static()` on the incoming pointer and panic
with a clear message if not. This is a safety net for any escape the
compile-time blacklist missed — not the primary enforcement mechanism. Zero
memory overhead in release builds (tag still set, assertions compiled out).

---

## Phased Implementation

| Phase | Work | Key files |
|---|---|---|
| 1 | Feature flag plumbing; conditional `GcBox`; compile-out GC machinery | `Cargo.toml`, `cljrs-gc/src/lib.rs` |
| 2 | `StaticArena` implementation | `cljrs-gc/src/static_arena.rs` |
| 3 | Thread-local context stack; `GcPtr::new()` dispatch; tag-bit provenance | `cljrs-gc/src/lib.rs` |
| 4 | Context pushes: `loop` iteration regions; `def`/static-sink Static pushes | `cljrs-interp/src/eval.rs`, `cljrs-eval/src/apply.rs` |
| 5 | Blacklist checks: lazy seq escape, region→static store, escaping closure | `cljrs-compiler/src/escape.rs` |
| 6 | Debug provenance assertions at static-sink write sites | `cljrs-value/src/types.rs` |
| 7 | Integration tests, docs | `tests/` |

---

## Benefits

- **Zero GC pauses** — no STW, no safepoints, no thread parking
- **Fast loop allocation** — bump pointer per iteration; reset is a pointer
  store
- **No programmer annotations** — regions are fully implicit; static context
  is the default
- **Simple mental model** — loop iterations own memory; everything else is
  transparent; three blacklisted patterns to avoid
- **Same function works everywhere** — a function called from a loop region or
  from a static top-level context behaves correctly in both (allocates into
  caller's context)
- **Backward compatible** — default build keeps GC; `no-gc` is opt-in
