# Plan: `no-gc` Compilation Mode (Strict Memory)

## Overview

A `no-gc` Cargo feature that replaces all GC machinery with two deterministic
allocation strategies. When enabled, every allocation is either region-local
(RAII, bump-pointer) or explicitly tagged `^:static` (program-lifetime, never
freed). No `GcHeap`, no stop-the-world pauses, no `Trace` impls.

### Memory Classes

| Class | Trigger | Lifetime | Allocator |
|---|---|---|---|
| **Region-local** | default | RAII scope | Existing `Region` bump allocator |
| **Static** | `^:static` metadata | program lifetime | New `StaticArena` (never frees) |

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
    try_alloc_in_region(value)
        .expect("no-gc: allocation outside a region scope; \
                 use ^:static for long-lived values")
}

pub fn static_alloc(value: T) -> GcPtr<T> {
    STATIC_ARENA.alloc(value)  // never freed
}
```

### New `StaticArena` (`cljrs-gc/src/static_arena.rs`)

- Global singleton (`OnceLock<StaticArena>`)
- Thread-safe bump allocator backed by leaked `Box<[u8]>` chunks
- `alloc<T>(value: T) -> GcPtr<T>` — allocates; destructor never runs
- Appropriate for: interned symbols, namespace tables, compile-time constants,
  global config, any top-level `def`-bound value

### Compiled out under `no-gc`

- `GcHeap`, `GcBoxHeader`, `MarkVisitor`, `Trace` trait and all impls
- Alloc-root thread-local, `AllocRootGuard`
- STW protocol (`cancellation.rs`), safepoints, parking
- `GcConfig` (no limits to configure)

The existing `Region` and `RegionGuard` are **kept as-is** — they become the
primary allocator.

---

## Layer 3 — Automatic Region Scoping

Under `no-gc`, the evaluator wraps constructs with implicit `RegionGuard`:

| Construct | Scope boundary |
|---|---|
| Function call (`fn*` body) | entire body |
| `let` block | entire block |
| `loop` / `recur` | each iteration |
| `doseq`, `for`, `dotimes` | each iteration step |

This mirrors Clojure's natural lexical scoping — each scope owns its
allocations, reset on exit.

A `with-region` special form (or macro) for explicit control:

```clojure
(with-region
  (let [x (compute-something)]
    (process x)))  ; region released after this block
```

---

## Layer 4 — Allocation Context Propagation

There is no user-visible annotation. The evaluator maintains a thread-local
**allocation context stack**. Each entry is either a `Region` (bump allocator)
or `Static` (the `StaticArena`). `GcPtr::new()` always allocates into the top
of this stack.

### Static context sources (automatic)

The following forms push `Static` onto the context stack before evaluating
their value expression, then pop it on exit:

| Form | Reason |
|---|---|
| Top-level `def` / `defn` | interned into a `Namespace` for program lifetime |
| `atom` / `agent` initial value | container outlives all regions |
| `reset!` / `vreset!` new value | written into a static container |
| Function passed to `swap!` / `vswap!` | return value written into a static container |
| `Var` root binding (`def` with value) | same as top-level `def` |

Everything else runs under the `Region` pushed by the nearest enclosing scope
(fn body, `let`, `loop` iteration).

### Context propagates inward through calls

When a function is called, it inherits the active allocation context from its
call site. The called function does not push a new context — it allocates into
whatever is currently active. A new `Region` is pushed only at scope boundaries
(fn entry, `let`, `loop` iteration).

This means a function called from a static context (`def`, `atom` init,
`swap!`, etc.) allocates all of its intermediate and return values statically,
without any annotation at the call site or inside the function.

```clojure
(defn make-map [k v] {k v})   ; allocates wherever caller dictates

(def config (make-map :host "localhost"))
; ↑ static context: config and the {:host ...} map both go to StaticArena

(defn process [data]
  (let [m (make-map :key data)]
    (use m)))
; ↑ region context: m is region-local, freed when let exits
```

### Metadata follows its value's provenance

A value's metadata map is allocated in the same context as the value itself —
the allocation context stack is not changed for metadata. `with-meta` and
`vary-meta` inherit the active context of the value being annotated.

---

## Layer 5 — Interprocedural Escape Analysis

Extend the existing escape analysis pass (`cljrs-compiler/src/escape.rs`) with
a `no-gc` mode that tracks allocation context through the full call chain, not
just within a single function.

### Escape signatures

Each function is annotated with an **escape signature** for its return value:

- `Local` — the return value is always consumed within the function's own
  region (e.g., the value is returned only into a `doall`, `doseq`, etc. that
  runs before the region resets).
- `Caller` — the return value's lifetime is polymorphic: it is allocated in
  whatever context is active at the call site, and may legitimately be returned
  up the call chain.

Most functions are `Caller`. `Local` is only inferred when the analysis can
prove the return value never leaves the function's scope.

### Error condition

A genuine escape error occurs only when the analysis conclusively shows that a
value allocated in a specific, finite-lifetime region is stored into a
container with a definitively longer lifetime **and** no static-context capture
point exists anywhere in the call chain above it.

In practice this means:

```clojure
; fine — escapes upward, but caller is a static sink (def)
(defn make-config [] {:host "localhost"})
(def config (make-config))   ; static context captures the escape

; fine — escapes upward through two functions, still reaches a static sink
(defn inner [] {:x 1})
(defn outer [] (inner))
(def result (outer))

; ERROR — escapes into a Var inside a non-static context with no capture path
(defn bad [atom-ref]
  (reset! atom-ref (java.util.Date.))   ; atom-ref is region-local, not static
  nil)
```

The key difference from the previous local-only rule: **returning a
region-local value from a function is never an error by itself**. The error is
only emitted when the full upward chain of call sites shows no valid capture.

### `LazySeq` under interprocedural analysis

`LazySeq` requires no special treatment. A thunk captures bindings from the
active allocation context at creation time. If created in a static context
(because the call chain roots at a `def` or `atom` init), the thunk's closed-
over values are static and the lazy seq may be returned and forced at any time.
If created in a region context, the same escape rules apply as for any other
value — the lazy seq may still be returned upward as long as the chain
eventually reaches a static capture point.

```clojure
; fine — called from a static sink; thunk's captures are static
(def evens (filter even? (range)))

; fine — realized before the region resets
(defn sum-evens [n]
  (let [xs (filter even? (range n))]
    (reduce + xs)))

; ERROR — escapes into a region-local atom with no static capture path
(defn bad []
  (let [a (atom nil)]
    (reset! a (map inc [1 2 3]))))   ; a is not a static sink
```

---

## Layer 6 — Mutable State as Static Sinks

`Atom`, `Var`, `Agent`, and `Namespace` internment are **static sinks**: they
push `Static` onto the allocation context stack before evaluating the value
expression, so there is nothing for the user or the compiler to do specially.
The evaluator handles this automatically as part of Layer 4.

The runtime provenance tag (Layer 7) is retained as a debug-mode assertion:
in debug builds, the mutable-state constructors verify that the value being
stored is tagged static and panic with a clear message if not. This catches any
case where the compiler analysis missed an escape — it is not the primary
enforcement mechanism.

---

## Layer 7 — Pointer Provenance Tracking

Tag each `GcPtr` under `no-gc` using the low bit of the pointer (alignment
guarantee gives at least 1 free bit):

```rust
#[cfg(feature = "no-gc")]
// Low bit: 0 = region-local, 1 = static
// Bit is masked off before dereferencing.
pub struct GcPtr<T: ?Sized> {
    ptr: NonNull<GcBox<T>>,
}
```

- `is_static()` → `ptr.as_ptr() as usize & 1 == 1`
- `is_region()` → `ptr.as_ptr() as usize & 1 == 0`
- Zero memory overhead — just a bit in the existing pointer

This runtime tag acts as a safety net alongside compile-time escape analysis.

---

## Phased Implementation

| Phase | Work | Key files |
|---|---|---|
| 1 | Feature flag plumbing; conditional `GcBox`; compile-out GC machinery | `Cargo.toml`, `cljrs-gc/src/lib.rs` |
| 2 | `StaticArena` implementation | `cljrs-gc/src/static_arena.rs` |
| 3 | `GcPtr::new()` → context-stack dispatch; tag-bit provenance (debug) | `cljrs-gc/src/lib.rs` |
| 4 | Allocation context stack; implicit `RegionGuard` (fn, `let`, `loop`) | `cljrs-interp/src/eval.rs` |
| 5 | Static context push for `def`, `atom`, `swap!`, `reset!`, `Var` | `cljrs-interp/src/eval.rs`, `cljrs-eval/src/apply.rs` |
| 6 | Interprocedural escape analysis with escape signatures | `cljrs-compiler/src/escape.rs` |
| 7 | Debug-mode runtime provenance assertions in mutable state constructors | `cljrs-value/src/types.rs` |
| 8 | `with-region` macro, docs, integration tests | `cljrs-stdlib`, `tests/` |

---

## Benefits

- **Zero GC pauses** — no STW, no safepoints, no thread parking
- **~10x faster allocation** — bump pointer vs GC heap with header overhead
- **Deterministic memory** — region reset at scope exit, not "whenever GC fires"
- **RAII-compatible** — all memory has lexically-scoped lifetimes
- **No user annotations** — allocation context is inferred from call chain; no
  `^:static` or other markers required
- **Backward compatible** — default build keeps GC; `no-gc` is opt-in
