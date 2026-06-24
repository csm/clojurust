# cljrs-base64

Base64 encoding and decoding for Clojurust, wrapping the [`base64`](https://crates.io/crates/base64) crate.

## Purpose

Exposes standard and URL-safe Base64 encode/decode functions to Clojure code under the `cljrs.base64` namespace.

## Status

Phase 1 — implemented. Linked statically into the `cljrs` binary via the `base64` feature (enabled by default) and also loadable as a dynamic plugin via the `cljrs_init` FFI entry point.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | All implementation: byte-conversion helpers, `init`, `register`, and the `cljrs_init` FFI entry point |

## Public API

```rust
/// Clojure namespace registered by this crate.
pub const NS: &str = "cljrs.base64";

/// Register `cljrs.base64` into `globals`. Idempotent.
pub fn init(globals: &Arc<GlobalEnv>);

/// Register all functions via an existing `Registry` (used by the FFI path).
pub fn register(registry: &mut Registry);

/// C-ABI entry point for dynamic plugin loading.
#[no_mangle]
pub unsafe extern "C" fn cljrs_init(registry: *mut Registry);
```

### Clojure functions

| Symbol | Signature | Description |
|---|---|---|
| `cljrs.base64/encode` | `(encode data)` | Base64-encode `data` (string or byte vector) using standard alphabet with padding; returns a `String` |
| `cljrs.base64/decode` | `(decode data)` | Decode a standard Base64 string or byte vector; returns a `ByteArray` |
| `cljrs.base64/encode-url` | `(encode-url data)` | URL-safe Base64 encode without padding; returns a `String` |
| `cljrs.base64/decode-url` | `(decode-url data)` | URL-safe Base64 decode (no padding); returns a `ByteArray` |

`data` may be a `String`, a `Vector` of integers in `[0, 255]`, a `ByteArray`, or a `ByteBlob`.
