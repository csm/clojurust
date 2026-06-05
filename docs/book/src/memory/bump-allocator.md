# The Bump Allocator

The bump allocator — called a **region** in the code — is the fastest way
clojurust can hand out memory. Instead of allocating each object individually
on the GC heap and later tracing it, a region carves objects out of a
contiguous block by advancing ("bumping") a single pointer, and frees them all
at once when the region's scope ends.

It is roughly **2.6× faster** than a GC-heap allocation: there is no mutex, no
per-object `Box::new`, and no collection pause.

> **The bump allocator only works in AOT mode right now.** It is enabled only
> in binaries produced by [`cljrs compile`](../rust-interop/aot.md). When you
> run code through the interpreter — `cljrs run`, `cljrs repl`, or
> `cljrs eval` — every allocation goes to the GC heap. The [Why AOT
> only?](#why-aot-only) section explains the reason.

## How it works

A region owns one or more **chunks** of raw memory (the default chunk is
4 KiB). It tracks a single bump pointer into the active chunk:

```text
chunk:  [ obj A ][ obj B ][ obj C ][ free ............ ]
                                    ^
                                    bump pointer
```

Allocating an object is:

1. Align the bump pointer up to the object's alignment.
2. If the object fits in the current chunk, write it there and advance the
   pointer past it. This is the common, near-instant path.
3. If it doesn't fit, allocate a fresh chunk (sized `max(4 KiB, 2 × object
   size)`), chain it on, and allocate from it.

There is no per-object bookkeeping for *freeing* — a region does not free
objects one at a time. When the region's scope ends, it:

1. Runs any registered destructors in **reverse (LIFO)** order, so objects that
   may reference earlier ones are torn down first.
2. Releases its chunks back to the system allocator (keeping the first chunk to
   reuse) and rewinds the bump pointer.

That bulk reset is what makes the allocator cheap: the cost of freeing a
thousand short-lived objects is one chunk free, not a thousand.

### Scopes and the region stack

Regions live on a thread-local **region stack**. AOT-compiled code brackets a
region-eligible scope with two runtime calls:

- `rt_region_start` pushes a fresh region onto the stack at scope entry.
- `rt_region_end` pops it at scope exit, running destructors and freeing chunks.

Only the **top** region receives allocations. While a region is active, the
runtime's allocation helpers route region-eligible collections into it and fall
back to the GC heap when no region is active:

```rust
fn box_coll_val(v: Value) -> *const Value {
    if region_is_active() {
        // bump-allocate into the active region
        try_alloc_in_region(v).unwrap().get() as *const Value
    } else {
        box_val(v) // fall back to the GC heap
    }
}
```

This fallback is why region promotion is always safe: if escape analysis is
conservative, or no region happens to be active, the object simply lands on the
GC heap with identical semantics — just a little slower.

## How the compiler decides what to bump-allocate

You never mark an allocation as region-eligible yourself. During AOT
compilation, an **escape analysis** pass classifies every allocation on a
four-level lattice:

| State | Meaning | Allocator |
|---|---|---|
| `NoEscape` | never leaves the function | **region** |
| `ArgEscape` | stored into an argument that escapes | GC heap |
| `Returns` | returned to the caller | region *if the caller doesn't let it escape* |
| `Escapes` | stored in the heap, captured by a closure, returned to the world | GC heap |

An allocation is promoted to a region only when it provably **does not escape**
the scope that created it — it is not returned, not stored in a longer-lived
container, not captured by a closure, and not passed to a call that could
retain it. The analysis understands many built-ins precisely (for example
`(first coll)` and `(count coll)` don't cause their argument to escape, while
`(conj coll x)` lets `coll` escape but not `x`), and it follows `recur` into
loop headers so loop-local intermediates can be region-allocated too.

Escape analysis also reaches across function boundaries: small non-capturing
callees are inlined so their allocations become local again, and larger callees
can be specialized to inherit the caller's region, so a helper that builds and
returns a short-lived vector can still be bump-allocated at a call site that
immediately discards it.

## Why AOT only?

The bump allocator depends on **compile-time** escape analysis, and that pass
runs only in the AOT compilation pipeline. The decision of *what* may be
region-allocated, and *where* the `rt_region_start` / `rt_region_end` brackets
go, is baked statically into the generated machine code.

The interpreter takes a different path: Clojure source is read, macro-expanded,
and lowered to IR, but the escape-optimization pass is **not** run, so the IR
the tree-walking evaluator executes contains no region instructions. With no
escape information available, the interpreter cannot safely decide at runtime
which objects outlive their scope, so it allocates everything on the GC heap —
the always-correct default. (The IR interpreter *can* execute region
instructions if they are already present in the IR, but the interpreter's own
front end never emits them.)

In short:

- **`cljrs compile` (AOT):** escape analysis runs, regions are used, GC handles
  the rest.
- **`cljrs run` / `repl` / `eval` (interpreter):** no escape analysis,
  everything on the GC heap, no bump allocation.

## Relationship to the GC

In the default build the two allocators run side by side, and the bump
allocator never hides memory from the collector:

- Region-allocated pointers carry a low-bit tag. The GC's mark phase checks the
  tag **without dereferencing** the pointer and skips region objects — their
  chunk memory may already have been freed and reused once the region's scope
  ended, so following them would be unsafe.
- Instead, every live region on the thread's region stack is treated as a **GC
  root**. The collector walks the objects inside active regions, so any GC-heap
  object reachable *only* through a region stays alive during collection.

So the bump allocator is a fast path for provably short-lived objects, and the
tracing GC remains the backstop for everything with a longer or unknown
lifetime.

## The `no-gc` build

clojurust can also be built with the `no-gc` Cargo feature, which removes the
tracing GC entirely and makes regions the *only* allocator. In that mode every
function call and every `loop` iteration pushes a scratch region that is freed
when the scope exits, return values are evaluated in the caller's region, and
program-lifetime values (from `def`, `defn`, `atom`, and friends) live in a
global static arena. This trades the GC's generality for zero collection pauses
and is documented separately; the default distribution ships with the GC
enabled.

## See also

- [Memory management overview](index.md) — how the GC and bump allocator fit
  together, and the `CLJRS_GC_STATS` counters.
- [AOT mode](../rust-interop/aot.md) — how `cljrs compile` builds the native
  binary the bump allocator runs in.
