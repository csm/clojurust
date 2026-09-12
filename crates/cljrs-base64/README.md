# cljrs-base64

Base64 encoding and decoding for Clojurust, wrapping the [`base64`](https://crates.io/crates/base64) crate.

## Purpose

Exposes standard and URL-safe Base64 encode/decode functions to Clojure code under the `cljrs.base64` namespace.

## Status

Phase 1 — implemented. Linked statically into the `cljrs` binary via the `base64` feature (enabled by default) and also loadable as a dynamic plugin via the `cljrs_init_cljrs_base64` FFI entry point.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | All implementation: byte-conversion helpers, `init`, `register`, and the `cljrs_init_cljrs_base64` FFI entry point |

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
pub unsafe extern "C" fn cljrs_init_cljrs_base64(registry: *mut Registry);
```

### Clojure functions

| Symbol | Signature | Description |
|---|---|---|
| `cljrs.base64/encode` | `(encode data)` | Base64-encode `data` (string or byte vector) using standard alphabet with padding; returns a `String` |
| `cljrs.base64/decode` | `(decode data)` | Decode a standard Base64 string or byte vector; returns a `ByteArray` |
| `cljrs.base64/encode-url` | `(encode-url data)` | URL-safe Base64 encode without padding; returns a `String` |
| `cljrs.base64/decode-url` | `(decode-url data)` | URL-safe Base64 decode (no padding); returns a `ByteArray` |

`data` may be a `String`, a `Vector` of integers in `[0, 255]`, a `ByteArray`, or a `ByteBlob`.

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`, which trades Unicode character classes for ~277 KB of text. |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |

Every workspace dependency of this crate is taken with default features off (see
the note in the root `Cargo.toml`), so these pass-throughs are what put back what
those crates' defaults used to provide. `default` enables all of them, so a plain
build is unchanged.

`regex-full` wins when both regex features are enabled, so selecting the small
engine means turning default features off **on this crate** and re-adding what
you want:

```toml
cljrs-base64 = { version = "0.1", default-features = false, features = ["small-regex"] }
```

Adding a second, direct dependency on `cljrs-runtime` with
`default-features = false` would not undo it — Cargo unions features across every
edge to a package, so one edge left at its defaults re-enables `regex-full` for
the whole graph. `deps` has to be off as well for the size win to land, since
`cljrs-project/vcs` pulls `regex` in through `pgp`. See
[cljrs-value's README](../cljrs-value/README.md#features).
