# cljrs-blake3

BLAKE3 cryptographic hash functions exposed as Clojure native functions via
the cljrs interop layer. Wraps the [`blake3`](https://crates.io/crates/blake3)
crate and registers the `blake3` Clojure namespace.

**Phase:** 9 (Rust Interop) — fully implemented.

---

## File layout

```
src/
  lib.rs                          — NativeObject impl, helper fns, cljrs_init
test/
  cljrs/
    blake3_test.cljrs             — clojure.test suite (known vectors + properties)
tests/
  clojure_tests.rs                — Rust harness that drives the Clojure test suite
cljrs.edn                         — project descriptor (paths, Rust init hook)
```

---

## Public API

### Rust

```rust
/// Register all `blake3` Clojure functions. Call from an embedding `main.rs`,
/// from an integration test, or from another crate's init hook.
pub fn register(registry: &mut cljrs_interop::Registry);

/// C-ABI entry point looked up by `cljrs build-native` / `cljrs run`. Calls
/// `register` internally.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_init(registry: *mut cljrs_interop::Registry);
```

### Clojure namespace: `blake3`

All `data` arguments accept either a `String` (hashed as UTF-8 bytes) or a
Clojure vector of integers in 0–255.

| Symbol | Signature | Returns | Description |
|---|---|---|---|
| `hash` | `(hash data)` | `String` | 64-char lowercase hex digest |
| `hash-raw` | `(hash-raw data)` | `[Long …]` | 32-element byte vector |
| `keyed-hash` | `(keyed-hash key data)` | `String` | BLAKE3 MAC; `key` must be a 32-byte vector |
| `derive-key` | `(derive-key context material)` | `String` | Domain-separated KDF; `context` is a `String` |
| `hasher-new` | `(hasher-new)` | `Blake3Hasher` | Create an incremental hasher |
| `hasher-update!` | `(hasher-update! h data)` | `Blake3Hasher` | Feed bytes into `h`; returns `h` for chaining |
| `hasher-finalize` | `(hasher-finalize h)` | `String` | Current digest as hex; `h` stays usable |
| `hasher-finalize-raw` | `(hasher-finalize-raw h)` | `[Long …]` | Current digest as 32-byte vector |

`Blake3Hasher` is an opaque `NativeObject`; its `type-tag` is `"Blake3Hasher"`.

### Usage examples

```clojure
(require '[blake3 :as b])

;; One-shot hash
(b/hash "hello world")
;=> "d74981efa70a0c880b8d8c1985d075dbcbf679b99a5f9914e5aaf96b831a9e24"

;; Hash a byte vector
(b/hash [0 1 2 3])

;; Keyed hash (MAC)
(b/keyed-hash (vec (repeat 32 0)) "message")

;; Key derivation
(b/derive-key "MyApp 2024-01-01 subkey" "master-secret")

;; Incremental hasher
(let [h (b/hasher-new)]
  (b/hasher-update! h "chunk-one")
  (b/hasher-update! h "chunk-two")
  (b/hasher-finalize h))
```

---

## Integration

Add to your `cljrs.edn`:

```edn
{:paths ["src"]
 :rust  {:crate "path/to/cljrs-blake3"
         :init  "cljrs_blake3::cljrs_init"}}
```

Or call `register` directly from your own init hook:

```rust
pub fn cljrs_init(registry: &mut Registry) {
    cljrs_blake3::register(registry);
    // … your own registrations …
}
```

> **Note on `Cargo.toml`:** this crate declares
> `[lib] crate-type = ["cdylib", "rlib"]` — `cdylib` produces the
> `.so`/`.dylib`/`.dll` loaded by `cljrs build-native`, while `rlib` keeps
> the crate usable as a normal workspace dependency (integration tests, AOT
> static linking). When wiring up your own interop crate, mirror this setup.

---

## Dependencies

| Crate | Role |
|---|---|
| `blake3` (1.x) | BLAKE3 hash implementation |
| `cljrs-interop` (workspace) | `Registry`, `wrap_fn*`, `NativeObject`, marshalling |
| `cljrs-gc` (workspace) | `GcPtr`, `Trace`, `MarkVisitor` |
| `cljrs-value` (workspace) | `Value`, `NativeFn`, `Arity` |
