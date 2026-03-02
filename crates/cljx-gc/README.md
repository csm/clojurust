# cljx-gc

Tracing garbage collector for clojurust. All Clojure runtime values are managed
by this GC; Rust code holds the root set.

**Phase:** 8 — stub only, not yet implemented.

---

## File layout

```
src/
  lib.rs    — doc-comment stub describing planned implementation
```

---

## Planned public API (Phase 8)

```rust
/// GC-managed smart pointer. Opaque to Rust's borrow checker;
/// all lifetime tracking is handled by the collector.
pub struct GcPtr<T> { /* private */ }

impl<T> GcPtr<T> {
    /// Allocate a new GC-managed value.
    pub fn new(value: T) -> Self

    /// Obtain a temporary reference valid until the next safepoint.
    pub fn get(&self) -> &T
}
```

Planned features:
- Mark-and-sweep with generational promotion
- Write barriers for pointer stores
- Weak references and finalization hooks
- Safepoint integration with the eval loop and JIT frames

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` (workspace) | `CljxError`, `CljxResult` |
