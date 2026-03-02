# cljx-interop

Rust ↔ Clojure interoperability layer. Exposes Rust functions to Clojure code,
marshals values across the boundary, and wraps opaque Rust structs as
GC-managed `NativeObject` values.

**Phase:** 9 — stub only, not yet implemented.

---

## File layout

```
src/
  lib.rs    — doc-comment stub describing planned implementation
```

---

## Planned public API (Phase 9)

```rust
/// Opaque wrapper that lets the GC manage an arbitrary Rust struct.
/// Created via the `#[cljx::export]` proc-macro or `NativeObject::new`.
pub struct NativeObject { /* private */ }

impl NativeObject {
    pub fn new<T: 'static + Send>(value: T) -> GcPtr<NativeObject>
    pub fn downcast<T: 'static>(&self) -> Option<&T>
}
```

The primary developer-facing surface is the `#[cljx::export]` proc-macro
(implemented as a separate proc-macro crate in a later phase):

```rust
#[cljx::export(name = "my-ns/my-fn")]
fn my_fn(x: i64) -> CljxResult<i64> {
    Ok(x * 2)
}
```

Planned features:
- `#[cljx::export]` proc-macro — exposes a Rust `fn` as a Clojure native function
- Type marshalling — bidirectional conversion between `Value` and Rust primitives
  (`i64`, `f64`, `bool`, `String`, `Vec`, `HashMap`, …)
- Error bridging — Rust `Result::Err` and `panic!` are caught and re-raised as
  Clojure exceptions
- `cljx.rust` namespace — `rust/cast`, `rust/unsafe`, `rust/import`
- Dynamic linking — load compiled `.so`/`.dylib` Rust extensions at runtime

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` (workspace) | `CljxError`, `CljxResult` |
| `cljx-gc` (workspace) | `GcPtr` — NativeObject lives behind the GC |
