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

## Layer 4 — `^:static` Metadata Routing

In the evaluator (`cljrs-interp`/`cljrs-eval`), check for `^:static` metadata
before evaluating any form:

```rust
fn eval_form(form: &Form, env: &Env) -> ValueResult<Value> {
    #[cfg(feature = "no-gc")]
    if form.meta().contains_key(keyword("static")) {
        return eval_in_static_context(form, env);
    }
    // ... normal eval
}
```

`eval_in_static_context` swaps the active allocator to `StaticArena` for the
duration of that expression's evaluation. All `GcPtr::new()` calls made during
evaluation of that subexpression go to the static arena.

### Implicit `^:static` for top-level `def` / `defn`

Top-level `def` and `defn` forms are **always** treated as static — no
annotation required. They are interned into a `Namespace` which lives for the
entire program, so any region-local allocation for their values would be a
use-after-free. The evaluator detects "top-level" context (depth = 0, not
inside a fn body or let block) and automatically routes through
`eval_in_static_context`.

```clojure
; all of these are automatically static at the top level:
(def config {:host "localhost" :port 8080})
(defn greet [name] (str "hello " name))
(def handlers {:get handle-get :post handle-post})

; ^:static is still useful for values nested inside other forms:
(atom ^:static {:state :running})
```

### Metadata follows its value's provenance

A value's metadata map is allocated with the same provenance as the value
itself. If the value is region-local, its metadata is region-local. If the
value is static (via `^:static` or top-level `def`), its metadata is also
allocated in the `StaticArena`.

`with-meta` and `vary-meta` inherit the provenance of the value being
annotated, not the metadata map argument. If a region-local metadata map is
attached to a static value, it is deep-copied into the `StaticArena` at the
point of attachment. This copy is the only case where provenance promotion
occurs implicitly — and the compiler should warn when it happens.

---

## Layer 5 — Escape Analysis Enforcement

Extend the existing escape analysis pass (`cljrs-compiler/src/escape.rs`) with
a `no-gc` mode check.

**Rule**: A value allocated in region R may not be stored into any container
with a lifetime longer than R.

Violation patterns to detect and reject:

1. Returning a region-local value from a function (unless `^:static`)
2. Storing into a `Var`, `Atom`, `Agent`, or `Namespace` binding
3. Capturing a region-local value in a closure that outlives the region
4. Storing into a data structure owned by the calling scope
5. An unrealized `LazySeq` escaping the region in which it was created (see
   below)

Emit a compile error pointing to the allocation site and the escape point, with
a suggestion to add `^:static`. This extends the existing `EscapeInfo` /
`AllocationSite` infrastructure — mark region-local sites as
`Lifetime::Region(scope_id)` and flag any escape where `scope_id` of the
destination outlives the source.

### `LazySeq` rules under `no-gc`

A `LazySeq` that is **fully realized within its creating scope** is legal:
the thunk and all produced values live and die in the same region.

A `LazySeq` that **escapes its creating scope** is a compiler error, because
the thunk captures region-local state that will have been reset by the time
the seq is forced. There is no implicit promotion.

```clojure
; OK — realized in the same let block
(let [xs (map inc [1 2 3])]
  (doall xs))   ; forces all elements before region resets

; COMPILER ERROR — lazy seq escapes the function's region
(defn bad []
  (map inc [1 2 3]))  ; returned unrealized; thunk is dangling

; OK — mark the result static so its thunk and elements go to StaticArena
(defn ok []
  ^:static (map inc [1 2 3]))
```

The escape analysis detects `LazySeq` escape the same way it detects any other
region-local escape: if the `LazySeq` value's `scope_id` is shorter than the
destination lifetime, it is an error. There is no special `LazySeq` path —
the general escape rule covers it.

---

## Layer 6 — Mutable State Constraints

Under `no-gc`, mutable containers must hold `^:static`-tagged values because
they outlive any region:

```rust
#[cfg(feature = "no-gc")]
impl Value {
    pub fn make_atom(initial: Value) -> ValueResult<Value> {
        assert_static(&initial)?;  // errors if initial came from a region
        Ok(Value::Atom(GcPtr::static_alloc(Atom::new(initial))))
    }
}
```

Apply similarly to: `swap!`, `reset!`, `Var` root bindings, namespace interns.

`assert_static` checks the provenance tag on the `GcPtr` (see Layer 7).

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
| 3 | `GcPtr::new()` → region dispatch; tag-bit provenance | `cljrs-gc/src/lib.rs` |
| 4 | Implicit `RegionGuard` in interpreter (fn body, `let`, `loop`) | `cljrs-interp/src/eval.rs` |
| 5 | `^:static` metadata → `eval_in_static_context` | `cljrs-interp/src/eval.rs`, `cljrs-eval/src/apply.rs` |
| 6 | Mutable state guards (`atom`, `var`, namespace interns) | `cljrs-value/src/types.rs` |
| 7 | Escape analysis extension for region lifetimes | `cljrs-compiler/src/escape.rs` |
| 8 | `with-region` macro, docs, integration tests | `cljrs-stdlib`, `tests/` |

---

## Benefits

- **Zero GC pauses** — no STW, no safepoints, no thread parking
- **~10x faster allocation** — bump pointer vs GC heap with header overhead
- **Deterministic memory** — region reset at scope exit, not "whenever GC fires"
- **RAII-compatible** — all memory has lexically-scoped lifetimes
- **`^:static` escape hatch** — for data that genuinely outlives a scope
- **Backward compatible** — default build keeps GC; `no-gc` is opt-in

---

## Open Questions

1. **Accumulation pattern**: A persistent map built up over many `assoc` calls
   across region boundaries — each intermediate allocation is region-local but
   the final result needs to escape. The escape analysis needs to handle this
   gracefully (e.g., promote the final result to the caller's region or require
   `^:static`).
