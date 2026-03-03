# cljx-gc

Garbage collector for clojurust. All Clojure runtime values are managed by
this GC; Rust code holds the root set.

**Phase:** 8 — real GC not yet implemented. This crate currently provides a
`GcPtr<T>` shim backed by `Arc<T>` so that `cljx-value` and all other crates
can depend on the planned API today. Phase 8 will replace the internals without
changing the public interface.

---

## File layout

```
src/
  lib.rs    — GcPtr<T> Arc shim, Trace marker trait
```

---

## Public API

### `Trace`

Marker trait for types that can be stored in a `GcPtr`.  Phase 8 will add a
real visitor method; for now the default no-op implementation satisfies the
bound.

```rust
pub trait Trace: Send + Sync {
    fn trace(&self) {}
}
```

### `GcPtr<T>`

A GC-managed smart pointer.  Currently wraps `Arc<T>`; Phase 8 replaces the
internals with a real GC handle.

```rust
pub struct GcPtr<T: ?Sized> { /* Arc<T> */ }

impl<T> GcPtr<T> {
    pub fn new(value: T) -> Self   // Phase 8 will add T: Trace bound
}

impl<T: ?Sized> GcPtr<T> {
    pub fn get(&self) -> &T
    pub fn ptr_eq(a: &Self, b: &Self) -> bool
}

impl<T: ?Sized> Clone for GcPtr<T> { … }
impl<T: ?Sized + Debug> Debug for GcPtr<T> { … }
```

---

## Planned Phase 8 additions

- Mark-and-sweep GC with generational promotion
- Write barriers for pointer stores into `GcPtr` fields
- Weak references (`WeakGcPtr<T>`) and finalization hooks
- Safepoint integration with the eval loop and JIT frames
- `GcVisitor` trait used by `Trace::trace` implementations
