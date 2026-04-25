# Plan: `no-gc` Compilation Mode (Strict Memory)

## Overview

A `no-gc` Cargo feature that replaces all GC machinery with region-based
allocation and a small blacklist of forbidden operations.

**Core model**: every function call and every `loop` iteration pushes a fresh
bump-allocator scratch region. All internal allocations land there. When the
scope exits, the scratch region is reset — freeing all intermediates. The
**return value** is the sole exception: the return expression is evaluated with
the scratch region temporarily removed from the active context, so the return
value lands directly in the *caller's* active region.

A function is therefore allocation-context-agnostic: called from a top-level
`def` its return value goes to the `StaticArena`; called from a loop iteration
its return value lands in that iteration's scratch region. No annotations
required. The return value is always in the right place by construction.

Other block forms (`let`, `do`, `if`, `when`, `cond`) may optionally create
their own scratch sub-regions; the same "return expression in caller's context"
rule applies recursively. In the initial implementation they inherit the
enclosing function's scratch region.

No `GcHeap`, no stop-the-world pauses, no `Trace` impls, no programmer-visible
region constructs.

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

`GcPtr<T>` still points to `GcBox<T>` everywhere — minimal `#[cfg]` surface.

### `GcPtr::new()` dispatches to the active context

```rust
#[cfg(feature = "no-gc")]
pub fn new(value: T) -> GcPtr<T> {
    ALLOC_CTX.with(|ctx| ctx.borrow().top().alloc(value))
}
```

The thread-local `ALLOC_CTX` stack holds `Static` or `Region(r)` entries. The
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

## Layer 3 — Allocation Context Stack and Scope Rules

The evaluator maintains a thread-local allocation context stack. `GcPtr::new()`
always allocates into the top entry.

### Scope entry: push a scratch region

When entering a function body or a `loop` iteration, a fresh `Region` is pushed
onto the stack. All allocations made during the body go into that region.

### Return expression: pop scratch, evaluate in caller's context

The final (return) expression of each scope is treated specially:

1. The scratch region is **popped** from the stack (not yet reset — its memory
   is still live and readable).
2. The return expression is evaluated. Any new allocations land in whatever is
   now at the top of the stack — the **caller's active context**.
3. The scratch region is **reset**, freeing all intermediates allocated during
   the body.

The return value was therefore never in the scratch region. It was allocated
directly in the caller's context.

### What this means for different call sites

```clojure
(defn make-pair [a b] [a b])

;; Called from top-level def → StaticArena is active when return expr evaluates
(def p (make-pair :x 1))   ; [a b] allocated in StaticArena ✓

;; Called from a loop iteration → iteration's Region is active
(loop [i 0]
  (let [pair (make-pair i (inc i))]
    (record! results pair))        ; pair in loop's Region, freed at recur ✓
  (recur (inc i)))
```

The same function works correctly in both contexts.

### Loop accumulators

`recur` argument expressions are the return expressions of the current
iteration — they are evaluated with the iteration's scratch popped, landing in
the caller's context (the scope enclosing the `loop`). The accumulated value
therefore always lives one level above the current iteration.

```clojure
(loop [acc [] i 0]
  (if (= i n)
    acc                         ; return expr: evaluated in caller's context ✓
    (recur (conj acc i) ...)))  ; recur arg: also in caller's context ✓
```

Old (replaced) `acc` values are in the enclosing scope's region and become
unreachable after the next iteration. They are freed when the enclosing scope
exits — not during the loop. This is expected; the programmer should not use
unbounded loops with persistent accumulation in long-lived static contexts.

### `let`, `do`, `if`, `when` (initial implementation)

These inherit the enclosing function's scratch region — no separate sub-region.
The "return expression in caller's context" rule is applied by the enclosing
function, not by these forms individually.

Future optimisation: each of these forms may push its own sub-scratch region,
enabling earlier reclamation of binding values.

### Static sinks push `Static`

The following forms push `Static` onto the context stack for their value
expression, making the computed value go to `StaticArena` regardless of the
enclosing scope:

| Form | Reason |
|---|---|
| Top-level `def` / `defn` value expr | interned into a `Namespace` for program lifetime |
| `atom` / `agent` / `Var` init expr | container outlives all regions |
| `reset!` / `vreset!` new-value expr | written into a static container |
| fn passed to `swap!` / `vswap!` | return value written into a static container |

### Metadata follows provenance

Metadata maps are allocated in the same active context as the value they
annotate. `with-meta` and `vary-meta` do not push a new context.

---

## Layer 4 — The Interior-Pointer Constraint

Because the scratch region is reset after the return expression is evaluated,
the return value must not contain any pointer into the scratch region's memory.
If it did, the pointer would dangle immediately.

**The return expression must produce a fresh value** (a new allocation in the
caller's context) or a primitive (`Long`, `Double`, `Bool`, `Nil`, `Char` —
stored inline in `Value`, never a pointer). It must not directly return a
pointer to an object allocated earlier in the scratch region.

```clojure
;; OK — assoc creates a fresh map in the caller's context;
;;      scratch-map is read but not returned by pointer
(defn update-count [scratch-map]
  (assoc scratch-map :count (inc (:count scratch-map))))

;; COMPILER ERROR — direct return of a scratch-region pointer
(defn bad [x]
  (let [tmp {:val x}]
    tmp))              ; tmp is in scratch; returning raw pointer → dangling
```

The second example is caught by the escape analysis: `tmp` was allocated in the
scratch region and is directly returned without going through an expression that
creates a fresh allocation in the caller's context.

In practice most functions naturally satisfy this constraint: they transform
their inputs into a new value and return it. The problematic case is "store
something in a local binding and return the binding unchanged" — which is caught
at compile time.

---

## Layer 5 — Blacklist

Only three operation patterns require GC to be safe. These are **forbidden in
`no-gc` mode** and produce compile errors.

### 1. Unrealized lazy seqs at a region boundary

A lazy seq's thunk captures references into the region where it was created. If
the region is reset before the seq is forced, the thunk accesses freed memory.

**Rule**: a lazy seq must be fully realized (via `doall`, `reduce`, `count`,
etc.) before the enclosing function returns — unless it is the return
expression and is therefore allocated in the caller's context where it will be
forced.

```clojure
;; OK — realized within the function before any region resets
(defn evens-up-to [n]
  (doall (filter even? (range n))))

;; OK — unrealized lazy seq IS the return expression; lands in caller's context
;;      (the caller must ensure it is realized or itself returns it upward)
(defn lazy-evens [n]
  (filter even? (range n)))    ; caller takes responsibility

;; ERROR — lazy seq created, stored in scratch binding, scratch-ptr returned
(defn bad []
  (let [xs (map inc [1 2 3])]
    xs))                       ; xs is a scratch-region pointer, not return-expr
```

### 2. Region-local values stored in static containers

Top-level `Atom`, `Var`, `Agent`, and namespace interns outlive any scratch
region. Storing a region pointer into them leaves a dangling reference after
the region resets.

The static-sink context push (Layer 3) ensures that values computed within
`reset!`/`swap!` expressions are allocated in `StaticArena`. The error fires
when a pre-existing region pointer is passed directly to a sink rather than
being computed freshly inside it.

```clojure
;; OK — conj is evaluated in Static context (inside swap!); fresh static value
(loop [i 0]
  (swap! log conj (build-entry i))
  (recur (inc i)))

;; ERROR — build-entry result is in the iteration's scratch region;
;;         passing it directly to reset! stores a region pointer in a static atom
(loop [i 0]
  (let [entry (build-entry i)]
    (reset! log entry))          ; region pointer → static atom
  (recur (inc i)))
```

### 3. Closures that capture region-local values and escape

A `fn` literal that closes over a region-local binding and escapes the region
(e.g., is stored in a static container or is the direct return of a function
where the capture is a scratch pointer) would reference freed memory when
called.

In most cases this is already caught by rules 1 and 2: the common escape
mechanisms are via lazy seqs (rule 1) or mutable state (rule 2). Any remaining
case — a closure stored as a static value that closes over a scratch binding —
is a compile error.

---

## Layer 6 — Pointer Provenance (Debug)

Tag each `GcPtr` with the low bit of the pointer (alignment gives ≥1 free bit):

- `0` = region-local
- `1` = static

In debug builds, write sites for static containers assert `is_static()` on the
incoming pointer and panic with a clear message if not. Zero memory overhead in
release builds (tag still set, assertions compiled out).

---

## Phased Implementation

| Phase | Work | Key files |
|---|---|---|
| 1 | Feature flag plumbing; conditional `GcBox`; compile-out GC machinery | `Cargo.toml`, `cljrs-gc/src/lib.rs` |
| 2 | `StaticArena` implementation | `cljrs-gc/src/static_arena.rs` |
| 3 | Thread-local context stack; `GcPtr::new()` dispatch; tag-bit provenance | `cljrs-gc/src/lib.rs` |
| 4 | Function/loop scope regions; return-expression-in-caller mechanism | `cljrs-interp/src/eval.rs`, `cljrs-eval/src/apply.rs` |
| 5 | Static-sink context pushes (`def`, `atom`, `swap!`, `reset!`, `Var`) | `cljrs-interp/src/eval.rs` |
| 6 | Blacklist checks: lazy seq escape, region→static store, escaping closure, interior-pointer return | `cljrs-compiler/src/escape.rs` |
| 7 | Debug provenance assertions at static-sink write sites | `cljrs-value/src/types.rs` |
| 8 | Integration tests, docs | `tests/` |

---

## Benefits

- **Zero GC pauses** — no STW, no safepoints, no thread parking
- **Intermediate values freed eagerly** — scratch region resets on every
  function return and loop iteration; only the return-value path persists
- **No programmer annotations** — regions are fully implicit; context is
  inferred from call site
- **Same function everywhere** — return value lands in the caller's active
  context whether that is `StaticArena`, a loop region, or another function's
  scratch
- **Simple blacklist** — three forbidden patterns; everything else is
  unrestricted
- **Backward compatible** — default build keeps GC; `no-gc` is opt-in
