# cljrs-interop

Rust ↔ Clojure interoperability layer. Exposes Rust functions to Clojure code,
marshals values across the boundary, and wraps opaque Rust structs as
GC-managed `NativeObject` values.

**Phase:** 9 — partially implemented (NativeObject, marshalling, error bridging, registration helpers).

---

## File layout

```
src/
  lib.rs       — re-exports, crate entry point
  error.rs     — wrap_result: Rust Result → ValueResult<Value>
  marshal.rs   — FromValue / IntoValue traits with impls for common Rust types
  register.rs  — wrap_fn0..wrap_fn3, wrap_fn_variadic: auto-marshalling function wrappers
```

The `NativeObject` trait and `NativeObjectBox` wrapper live in `cljrs-value::native_object`
and are re-exported from this crate for convenience.

---

## Public API

### NativeObject (re-exported from cljrs-value)

```rust
pub trait NativeObject: Send + Sync + Debug + Trace + 'static {
    fn type_tag(&self) -> &str;       // used for protocol dispatch
    fn as_any(&self) -> &dyn Any;     // downcast support
}

pub struct NativeObjectBox { /* wraps Box<dyn NativeObject> */ }
pub fn gc_native_object(obj: impl NativeObject) -> GcPtr<NativeObjectBox>;
```

### Type marshalling

```rust
pub trait IntoValue { fn into_value(self) -> Value; }
pub trait FromValue: Sized { fn from_value(v: &Value) -> ValueResult<Self>; }
```

Implemented for: `()`, `bool`, `i64`, `f64`, `String`, `&str`, `BigInt`, `Option<T>`, `Vec<Value>`, `Value`.

### Error bridging

```rust
pub fn wrap_result<T: IntoValue, E: Display>(r: Result<T, E>) -> ValueResult<Value>;
```

### Registration helpers

Auto-marshalling wrappers that convert idiomatic Rust function signatures into `NativeFn`:

```rust
pub fn wrap_fn0<R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn;
pub fn wrap_fn1<A, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn;
pub fn wrap_fn2<A, B, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn;
pub fn wrap_fn3<A, B, C, R, E, F>(name: impl Into<Arc<str>>, f: F) -> NativeFn;
pub fn wrap_fn_variadic<R, E, F>(name: impl Into<Arc<str>>, min: usize, f: F) -> NativeFn;
```

These accept closures (not just bare `fn` pointers) since `NativeFnFunc` is now
`Arc<dyn Fn(&[Value]) -> ValueResult<Value> + Send + Sync>`.

---

## Remaining work (Phase 9)

- `#[cljx::export]` proc-macro — syntactic sugar over manual registration
- `cljx.rust` namespace with intrinsics
- Dynamic linking — load `.so`/`.dylib` Rust extensions at runtime
- RAII / `with-open` resource management

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr`, `Trace`, `MarkVisitor` |
| `cljrs-value` (workspace) | `Value`, `NativeFn`, `NativeObject`, `NativeObjectBox` |
| `num-bigint` (workspace) | `BigInt` marshalling |
