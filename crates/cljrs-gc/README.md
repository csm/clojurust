# cljrs-gc

Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust;
or, with the `no-gc` Cargo feature, a region-based allocator with no GC pauses.

**Phase:** 8.1 (GcVisitor + Trace infrastructure) + 8.2 (GcBox/GcHeap
raw-pointer implementation) — implemented.  `no-gc` mode (Phase 1–5 of
`docs/no-gc-plan.md`) — implemented.

---

## Purpose

Manages all Clojure runtime values.  `GcPtr<T>` is a raw pointer into either
the GC heap or a bump-allocated region; `clone` is O(1); `drop` is a no-op.

**Default build (GC mode):** memory is freed only during `GcHeap::collect`.

**`no-gc` build:** every function call and every `loop` iteration pushes a
scratch `Region`; intermediates are freed when the scope exits.  Return values
and `recur` arguments are evaluated in the caller's context (the
"return-expression-in-caller" mechanism).  Top-level `def`, `atom`, and `reset!`
values go to the global `StaticArena` and live for the program lifetime.
No `GcHeap`, no stop-the-world pauses, no `Trace` overhead at runtime.

---

## File layout

```
src/
  lib.rs          — GcVisitor, Trace, GcBox<T>, GcPtr<T>, MarkVisitor, HEAP,
                    leaf Trace impls; conditional GC vs no-gc implementations
  gc_header       — (GC mode only) GcBoxHeader, drop/trace fns
  gc_full         — (GC mode only) GcHeap, ALLOC_ROOTS, AllocRootGuard
  nogc_stubs      — (no-gc mode) stub GcHeap, GcConfig, cancellation stubs
  static_arena.rs — (no-gc mode) global program-lifetime bump allocator
  alloc_ctx.rs    — (no-gc mode) thread-local allocation context stack;
                    ScratchGuard, StaticCtxGuard
  region.rs       — Region bump allocator, RegionGuard, thread-local region stack
  cancellation.rs — (GC mode) STW coordination, MutatorGuard, safepoints
  config.rs       — (GC mode) GcConfig, GcCancellation, GcParked
```

---

## Public API

### `GcVisitor`

```rust
pub trait GcVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>);
}
```

Implemented by [`MarkVisitor`].  Call `visitor.visit(ptr)` inside
`Trace::trace` for every `GcPtr` field.

### `Trace`

```rust
pub trait Trace: Send + Sync {
    fn trace(&self, visitor: &mut MarkVisitor);
}
```

Implemented by every type stored behind a `GcPtr`.  Must call
`visitor.visit(ptr)` for every `GcPtr` reachable from `self` (directly or
through `Arc`/`Mutex`/etc.).

Built-in leaf impls: `String`, `i64`, `f64`, `bool`,
`num_bigint::BigInt`, `bigdecimal::BigDecimal`,
`num_rational::Ratio<BigInt>`.

### `GcPtr<T: Trace + 'static>`

```rust
pub struct GcPtr<T: Trace + 'static>(NonNull<GcBox<T>>);

impl<T: Trace + 'static> GcPtr<T> {
    pub fn new(value: T) -> Self        // allocates on HEAP
    pub fn get(&self) -> &T             // borrow; invalid after collect frees it
    pub fn ptr_eq(a: &Self, b: &Self) -> bool
}
impl<T: Trace + 'static> Clone for GcPtr<T> { /* O(1) raw-pointer copy */ }
impl<T: Trace + 'static> Drop  for GcPtr<T> { /* no-op */ }
```

### `GcHeap`

```rust
pub struct GcHeap { /* Mutex<GcHeapInner> */ }

impl GcHeap {
    pub const fn new() -> Self
    pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T>
    pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, trace_roots: F)
    pub fn count(&self) -> usize
    pub fn total_allocated(&self) -> usize
    pub fn total_freed(&self) -> usize
}
```

`collect` is stop-the-world: must only be called when no other thread is
creating or dereferencing `GcPtr` values.

### `MarkVisitor`

```rust
pub struct MarkVisitor { /* grey stack */ }
impl GcVisitor for MarkVisitor { … }
```

Uses a grey stack (avoids recursion stack overflow) and handles cycles via
already-marked check.

### `HEAP`

```rust
pub static HEAP: GcHeap;
```

Global singleton; all `GcPtr::new` calls allocate here.

### `region::Region`

```rust
pub struct Region { /* chunks, bump pointer, drop registry */ }

impl Region {
    pub fn new() -> Self
    pub fn with_capacity(cap: usize) -> Self
    pub fn alloc<T: Trace + 'static>(&mut self, value: T) -> GcPtr<T>
    pub fn reset(&mut self)
    pub fn bytes_used(&self) -> usize
    pub fn object_count(&self) -> usize
}
```

Bump allocator for short-lived objects. ~2.6x faster than `GcHeap::alloc`
(no mutex, no `Box::new`). Objects are NOT in the GC heap linked list.
Destructors run on `reset()` or `drop`.

### `region::RegionGuard`

RAII guard that pushes a `Region` onto the thread-local stack. Use with
`try_alloc_in_region()` for opportunistic region allocation.

---

## Design notes

- **Non-moving**: `GcPtr<T>` stores a stable `NonNull<GcBox<T>>` address.
- **Stop-the-world**: `collect` must pause all other threads that hold `GcPtr`s.
- **Intrusive linked list**: all `GcBox`es are linked via `GcBoxHeader::next`.
- **Type erasure**: `trace_fn` / `drop_fn` in the header enable type-erased
  mark and sweep without a vtable pointer per allocation.
- **Cycle collection**: because `GcPtr::drop` is a no-op, reference cycles do
  not prevent collection — any object unreachable from roots is freed.

---

## Deferred to later phases

- Incremental/concurrent collection — Phase 10+
- Write barriers for generational GC — Phase 10+
- Weak references (`WeakGcPtr<T>`) — deferred
- Safepoint integration with JIT frames — Phase 10+
- Automatic collection trigger (threshold-based) — deferred
