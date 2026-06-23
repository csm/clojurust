# cljrs-interop

Rust ↔ Clojure interoperability layer. Exposes Rust functions to Clojure code,
marshals values across the boundary, and wraps opaque Rust structs as
GC-managed `NativeObject` values.

**Phase:** 9 — partially implemented (NativeObject, marshalling, error bridging,
registration helpers, Registry, `#[export]` proc-macro).

---

## File layout

```
src/
  lib.rs       — re-exports, crate entry point
  error.rs     — wrap_result: Rust Result → ValueResult<Value>
  exports.rs   — ExportEntry / ProvenanceEntry, inventory::collect!, register_exports,
                 register_provenance! macro
  marshal.rs   — FromValue / IntoValue traits with impls for common Rust types
  register.rs  — wrap_fn0..wrap_fn3, wrap_fn_variadic: auto-marshalling function wrappers
  registry.rs  — Registry struct and InitFn type alias for cljrs_init convention
```

The `NativeObject` trait and `NativeObjectBox` wrapper live in `cljrs-value::native_object`
and are re-exported from this crate for convenience.

The `#[export]` attribute macro is implemented in `cljrs-export-macro` and
re-exported here for ergonomic use as `#[cljrs_interop::export(...)]`.

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

### `#[export]` proc-macro and `register_exports`

Annotate any free Rust function with `#[export(ns = "...")]` to register it
automatically. Then call `register_exports` once inside `cljrs_init`:

```rust
use cljrs_interop::{export, register_exports, Registry};

#[export(ns = "math")]
pub fn add(a: i64, b: i64) -> Result<i64, String> { Ok(a + b) }

#[export(ns = "math")]
pub fn pi() -> f64 { std::f64::consts::PI }

pub fn cljrs_init(registry: &mut Registry) {
    register_exports(registry);
}
```

Supported signatures and attribute options are documented in the
`cljrs-export-macro` crate README.

```rust
pub struct ExportEntry {
    pub qualified: &'static str,  // "ns/name"
    pub make_fn:   fn() -> NativeFn,
}

pub fn register_exports(registry: &mut Registry);
```

### Native provenance (verified HEAD binding)

Pinned lookups of native functions (`math/add@<sha>`) always resolve to the
current binary's implementation — but the resolver verifies the pin against
the package's recorded provenance: a match (either side may be an
abbreviated hash) is silent; a mismatch or missing provenance warns once per
pin, or errors under `--enforce-native-versions` (cljrs.edn
`:enforce-native-versions true`).

```rust
pub struct ProvenanceEntry { pub ns: &'static str, pub commit: &'static str }

// Declare once per exported namespace (commit typically from build.rs):
cljrs_interop::register_provenance!("math", env!("CLJRS_PKG_COMMIT"));

// Or imperatively inside cljrs_init:
registry.set_provenance("math", commit);
```

### Registry and InitFn

The entry point for mixed Rust/Clojure projects.  User crates implement a
`cljrs_init` function and list it under `:rust :init` in `cljrs.edn`; the
build toolchain calls it before loading any Clojure source.

```rust
/// Signature of the hook-registration function.
pub type InitFn = fn(&mut Registry);

pub struct Registry { /* wraps Arc<GlobalEnv> */ }

impl Registry {
    pub fn new(env: Arc<GlobalEnv>) -> Self;

    /// Versioned view: registrations land in "<ns>@<commit>" namespaces.
    /// Used by cljrs-dylib when loading a pinned native package; does NOT
    /// auto-register the calling binary's #[export] inventory.
    pub fn versioned(env: Arc<GlobalEnv>, commit: &str) -> Self;

    /// Unversioned view that does NOT auto-register the host's #[export]
    /// inventory.  Used by cljrs-dylib when a `:rust/load :dylib` dep is
    /// brought in by a plain `require`: the dep's exports land in their real
    /// (unversioned) namespaces, but the dylib registers its own inventory.
    pub fn for_require(env: Arc<GlobalEnv>) -> Self;

    /// Register f under "my.ns/my-fn" (panics if no '/' present).
    pub fn define(&self, qualified: &str, f: NativeFn);

    /// Register f into an explicit namespace under a plain name.
    pub fn define_in(&self, ns: &str, name: &str, f: NativeFn);

    /// Record the commit a native package was built from (no-op on a
    /// versioned view, which registers a pinned package).
    pub fn set_provenance(&self, ns: &str, commit: &str);

    /// Access the underlying GlobalEnv for advanced operations.
    pub fn env(&self) -> &Arc<GlobalEnv>;
}
```

**Usage pattern:**

```rust
// user's lib.rs
use cljrs_interop::{Registry, wrap_fn1, wrap_fn2};

pub fn cljrs_init(registry: &mut Registry) {
    registry.define("my.project/greet",
        wrap_fn1("greet", |name: String| Ok::<String, String>(format!("Hello, {name}!"))));
    registry.define("my.project/add",
        wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)));
}
```

```edn
;; cljrs.edn
{:paths ["src"]
 :rust  {:crate "."
         :init  "my_project::cljrs_init"}}
```

### Versioned symbols and native functions

Clojure code can pin a function to a specific git commit with the
`name@<hash>` syntax.  For native (Rust-backed) functions, the
implementation lives in the running binary — we can't fetch and execute a
historical compiled version.  The current contract is: a versioned
lookup of a native symbol resolves to the **HEAD** (current)
implementation regardless of the requested commit.

Resolution order for `my.ns/greet@abc1234`:

1. **Version cache** — already resolved this session → return immediately.
2. **Git source** — fetch `my/ns.cljrs` at `abc1234`, find `(defn greet …)`, evaluate in snapshot env.
3. **HEAD fallback** — no Clojure source definition exists (or the namespace has no git context at all) but `greet` is currently a `NativeFunction` → return the HEAD value.
4. **Error** — symbol not found.

A future design may fetch Rust source at the commit, compile it, and
`dlopen` the result to provide true per-commit native semantics; that
codepath would replace the HEAD fallback.

---

## Remaining work (Phase 9)

- `cljrs.rust` namespace with intrinsics
- Dynamic linking — load `.so`/`.dylib` Rust extensions at runtime via `cljrs build-native`

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `CljxError`, `CljxResult` |
| `cljrs-gc` (workspace) | `GcPtr`, `Trace`, `MarkVisitor` |
| `cljrs-value` (workspace) | `Value`, `NativeFn`, `NativeObject`, `NativeObjectBox` |
| `cljrs-env` (workspace) | `GlobalEnv` — used by `Registry` to intern native functions |
| `cljrs-export-macro` (workspace) | `#[export]` proc-macro |
| `inventory` (workspace) | Link-time collection of `ExportEntry` items |
| `num-bigint` (workspace) | `BigInt` marshalling |
