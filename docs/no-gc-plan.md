# Plan: `no-gc` Compilation Mode (Strict Memory)

## Overview

A `no-gc` Cargo feature that replaces all GC machinery with two deterministic
allocation strategies:

| Class | Default? | Lifetime | Allocator |
|---|---|---|---|
| **Static** | Yes — top-level context | program lifetime | `StaticArena` (never frees) |
| **Region** | Opt-in — `with-region` / `loop` | RAII scope | Existing `Region` bump allocator |

**The fundamental rule**: functions never own an allocation context. A function
allocates its return value into whatever context the *caller* has active. This
is what makes return values work correctly across region boundaries — the same
mechanism that lets a region-mode function return a value into a GC context in
mixed mode.

Because the top-level evaluation context is static, most code allocates
statically without any annotation. `with-region` is an explicit opt-in for
bounded, high-frequency scratch work where bump allocation pays off.

No `GcHeap`, no stop-the-world pauses, no `Trace` impls.

---

## Layer 1 — Cargo Feature

Add to `Cargo.toml` (workspace root):

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

All dependent crates propagate via `dep?/no-gc`. The default build keeps the
current GC; `--features no-gc` opts into strict mode. These are mutually
exclusive compilation paths — not a runtime toggle.

---

## Layer 2 — `cljrs-gc` Changes

### `GcBox<T>` becomes zero-overhead under `no-gc`

```rust
// cfg(not(feature = "no-gc")) — current layout with GC header
pub struct GcBox<T: ?Sized> {
    header: GcBoxHeader,  // magic, lives, trace_fn, drop_fn, next
    pub value: T,
}

// cfg(feature = "no-gc") — no header at all
pub struct GcBox<T: ?Sized> {
    pub value: T,
}
```

`GcPtr<T>` still points to `GcBox<T>` everywhere — this keeps `#[cfg]` surface
minimal in the rest of the codebase.

### `GcPtr::new()` routing

```rust
#[cfg(feature = "no-gc")]
pub fn new(value: T) -> GcPtr<T> {
    // Allocates into whatever context is currently active on the thread-local
    // context stack (Static or a Region). Panics if the stack is empty.
    ALLOC_CONTEXT.with(|ctx| ctx.borrow().alloc(value))
}
```

### New `StaticArena` (`cljrs-gc/src/static_arena.rs`)

- Global singleton (`OnceLock<StaticArena>`)
- Thread-safe bump allocator backed by leaked `Box<[u8]>` chunks
- `alloc<T>(value: T) -> GcPtr<T>` — allocates; destructor never runs
- The default active context at program startup

### Compiled out under `no-gc`

- `GcHeap`, `GcBoxHeader`, `MarkVisitor`, `Trace` trait and all impls
- Alloc-root thread-local, `AllocRootGuard`
- STW protocol (`cancellation.rs`), safepoints, parking
- `GcConfig` (no limits to configure)

The existing `Region` and `RegionGuard` are **kept as-is**.

---

## Layer 3 — Allocation Context Stack

The evaluator maintains a thread-local **allocation context stack**. Each entry
is either `Static` (the `StaticArena`) or `Region(r)` (a bump allocator).
`GcPtr::new()` always allocates into the entry at the top of the stack.

### Functions do not push a context

When a function is called, no new entry is pushed. The function allocates
directly into the caller's active context. Return values therefore land in the
caller's context by construction — no promotion or copying needed.

This is the property that makes cross-boundary returns work: a function called
from a `with-region` allocates its result into that region; the same function
called from a static context (top-level `def`) allocates its result statically.

### What does push a context

| Construct | What is pushed | When popped |
|---|---|---|
| Program start | `Static` | never |
| Top-level `def` / `defn` value expr | `Static` | after value is evaluated |
| `atom` / `agent` / `Var` init expr | `Static` | after init is evaluated |
| `reset!` / `vreset!` new-value expr | `Static` | after new value is evaluated |
| fn passed to `swap!` / `vswap!` | `Static` | after fn returns |
| `with-region` block | `Region(fresh)` | on block exit (region reset) |
| `loop` / `recur` each iteration | `Region(fresh)` | on iteration end (region reset) |

`let`, `fn*` bodies, `do`, `if`, `when`, `cond`, etc. do **not** push a
context — they inherit whatever is currently active.

### Illustrative traces

```clojure
; Everything is static — no with-region anywhere
(defn make-map [k v] {k v})
(def config (make-map :host "localhost"))
; Stack at make-map call site: [Static(def)]
; {k v} allocated in StaticArena ✓

; Return value lands in caller's region
(defn helper [x] (* x 2))
(with-region
  (let [n (helper 21)]
    (println n)))
; Stack at helper call site: [Static(prog) | Region(with-region)]
; (* x 2) allocated in the Region ✓
; Region reset after with-region exits ✓

; loop: each iteration gets a fresh region
(loop [i 0]
  (when (< i 1000)
    (let [tmp (build-thing i)]  ; tmp in iteration Region
      (record! results tmp))    ; record! must be conforming (see Layer 4)
    (recur (inc i))))
; Each iteration's Region is reset before the next recur ✓
```

### Metadata follows its value's provenance

A value's metadata map is allocated in the same active context as the value
itself. `with-meta` and `vary-meta` do not push a new context.

---

## Layer 4 — Region Conformance

A `with-region` block (or `loop` iteration) is only correct if no value
allocated within its `Region` escapes to a container with a longer lifetime.
This is the **region conformance** invariant.

### What "escaping" means

A region-local value *escapes* its region when:

1. It is stored into a static container — a top-level `Atom`, `Var`, or
   `Namespace` intern that was created outside the region.
2. It is captured by a closure that outlives the region.
3. It is returned as the final result of the `with-region` block itself (the
   block's return value would dangle immediately after the region resets).
4. A static value is constructed that holds an interior pointer into the
   region — the static value would outlive the pointer.

Storing into a container that was *also* created inside the same region is
fine: both the container and the value die together.

### Whole-call-subtree enforcement

Conformance is not checked locally — it is checked across the entire reachable
call graph rooted at each `with-region` block.

A function F is **region-conforming** if:

- F never performs any of the four escape operations above, AND
- Every function G that F calls is also region-conforming (transitively).

The compiler checks conformance by walking the call graph from each
`with-region` boundary. If any reachable function reaches a static store
targeting a container created outside the region, that is an error reported at
the point of the violating call.

### Why whole-call-subtree matters

A single function may look safe locally but pull in a callee that escapes:

```clojure
(defn save! [x]
  (reset! global-log x))   ; stores into a top-level atom — non-conforming

(with-region
  (let [record (build-record)]
    (save! record)))        ; ERROR: save! is non-conforming
```

`save!` is flagged at its call site inside the `with-region`, not at its
definition. The same function may be called from a static context without
error:

```clojure
(save! (build-record))  ; fine — build-record allocates statically,
                         ; save! stores static value into static atom ✓
```

### `LazySeq` under conformance

A `LazySeq` created inside a `with-region` has its thunk allocated in the
region. Forcing the seq after the region resets would access freed memory.
Therefore a `LazySeq` must be fully realized (via `doall`, `reduce`, etc.)
before the `with-region` exits — or it must not be created inside a
`with-region` at all. The escape analysis treats an unrealized `LazySeq`
returning from a `with-region` as a violation of rule 3 above.

```clojure
; OK — fully realized before region resets
(with-region
  (doall (map inc scratch-data)))

; ERROR — unrealized lazy seq escapes the region
(with-region
  (map inc scratch-data))   ; returns unrealized; thunk is dangling
```

---

## Layer 5 — Mutable State as Static Sinks

Top-level `Atom`, `Var`, `Agent`, and namespace interns are always allocated in
the `StaticArena`. Storing a value into them pushes `Static` onto the context
stack for the new-value expression (Layer 3), so the new value is itself
allocated statically.

The region conformance analysis (Layer 4) ensures that no region-local value
reaches a static sink as a direct pointer. Because the static-sink context push
makes the *new value expression* allocate statically, the value stored is
always a fresh static allocation — never a pointer into a region. The escape
error fires when the code hands an already-allocated region pointer to the sink
rather than letting the sink evaluate its value expression freshly.

```clojure
; Fine — swap! evaluates (assoc @state :k v) in Static context;
; result is a new static map.
(swap! state assoc :k v)

; ERROR — region-local 'record' is passed directly to reset!;
; reset! stores the existing pointer, not a new static value.
(with-region
  (let [record (build-record)]
    (reset! state record)))    ; region pointer → static atom: escape
```

Debug builds retain a runtime provenance tag (low bit of `GcPtr`) and assert
at static-sink write sites that the incoming pointer is tagged static. This
catches any escape the compile-time analysis missed.

---

## Layer 6 — Pointer Provenance Tracking (Debug)

Tag each `GcPtr` under `no-gc` using the low bit of the pointer (alignment
gives at least 1 free bit):

```rust
#[cfg(feature = "no-gc")]
// Low bit: 0 = region, 1 = static. Masked off before deref.
pub struct GcPtr<T: ?Sized> {
    ptr: NonNull<GcBox<T>>,
}
```

- `is_static()` → `ptr.as_ptr() as usize & 1 == 1`
- `is_region()` → `ptr.as_ptr() as usize & 1 == 0`

In release builds the tag is still set (allocation path is the same) but
assertions are compiled out. Zero memory overhead.

---

## Phased Implementation

| Phase | Work | Key files |
|---|---|---|
| 1 | Feature flag plumbing; conditional `GcBox`; compile-out GC machinery | `Cargo.toml`, `cljrs-gc/src/lib.rs` |
| 2 | `StaticArena` implementation | `cljrs-gc/src/static_arena.rs` |
| 3 | Thread-local context stack; `GcPtr::new()` dispatches to top entry; tag-bit provenance | `cljrs-gc/src/lib.rs` |
| 4 | Context pushes for `with-region`, `loop` iteration, top-level `def`, static sinks | `cljrs-interp/src/eval.rs`, `cljrs-eval/src/apply.rs` |
| 5 | Whole-call-subtree region conformance analysis | `cljrs-compiler/src/escape.rs` |
| 6 | Debug-mode provenance assertions at static-sink write sites | `cljrs-value/src/types.rs` |
| 7 | `with-region` macro, docs, integration tests | `cljrs-stdlib`, `tests/` |

---

## Benefits

- **Zero GC pauses** — no STW, no safepoints, no thread parking
- **~10x faster allocation in regions** — bump pointer vs GC heap with header overhead
- **Deterministic memory** — region reset is explicit and immediate
- **No user annotations** — static context is the default; regions are opt-in
- **Correct across mixed mode** — functions allocate in caller's context, so
  the same function works in GC, static, or region calling contexts
- **Backward compatible** — default build keeps GC; `no-gc` is opt-in
