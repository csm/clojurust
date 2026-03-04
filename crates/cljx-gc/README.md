# cljx-gc

Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust.

**Phase:** 8.1 (GcVisitor + Trace infrastructure) + 8.2 (GcBox/GcHeap raw-pointer implementation) — implemented.

---

## Purpose

Manages all Clojure runtime values.  Rust code owns the root set and triggers
collection explicitly.  `GcPtr<T>` is a raw pointer into the GC heap; `clone`
is O(1); `drop` is a no-op.  Memory is freed only during `GcHeap::collect`.

---

## File layout

```
src/
  lib.rs    — GcVisitor, Trace, GcBoxHeader, GcBox<T>, GcHeap, MarkVisitor,
              HEAP singleton, GcPtr<T>, leaf Trace impls (i64, f64, BigInt, …)
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
