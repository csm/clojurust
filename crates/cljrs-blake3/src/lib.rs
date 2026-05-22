//! BLAKE3 cryptographic hash functions for clojurust.
//!
//! Registers the `blake3` Clojure namespace with the following functions:
//!
//! | Symbol | Signature | Description |
//! |---|---|---|
//! | `blake3/hash` | `(hash data)` | One-shot hash → 64-char hex string |
//! | `blake3/hash-raw` | `(hash-raw data)` | One-shot hash → 32-element byte vector |
//! | `blake3/keyed-hash` | `(keyed-hash key data)` | BLAKE3 MAC; key is 32-byte vector |
//! | `blake3/derive-key` | `(derive-key context material)` | Domain-separated KDF → hex string |
//! | `blake3/hasher-new` | `(hasher-new)` | Create incremental hasher (NativeObject) |
//! | `blake3/hasher-update!` | `(hasher-update! h data)` | Feed data into hasher; returns `h` |
//! | `blake3/hasher-finalize` | `(hasher-finalize h)` | Produce 64-char hex; hasher stays usable |
//! | `blake3/hasher-finalize-raw` | `(hasher-finalize-raw h)` | Produce 32-element byte vector |
//!
//! `data` arguments accept either a `String` (hashed as UTF-8 bytes) or a
//! Clojure vector of integers in the range 0–255.

use std::sync::Mutex;

use cljrs_gc::{MarkVisitor, Trace};
use cljrs_interop::{
    IntoValue, NativeObject, Registry, Value, ValueError, ValueResult, gc_native_object, wrap_fn0,
    wrap_fn1, wrap_fn2,
};
use cljrs_value::{Arity, NativeFn};

// ── Blake3Hasher NativeObject ─────────────────────────────────────────────────

/// Incremental BLAKE3 hasher wrapped as an opaque Clojure value.
///
/// Thread-safe: the inner `blake3::Hasher` is guarded by a `Mutex` so the
/// same hasher object can be updated from Clojure code without data races.
pub struct Blake3Hasher {
    inner: Mutex<blake3::Hasher>,
}

impl std::fmt::Debug for Blake3Hasher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Blake3Hasher").finish_non_exhaustive()
    }
}

impl NativeObject for Blake3Hasher {
    fn type_tag(&self) -> &str {
        "Blake3Hasher"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// Blake3Hasher holds no GcPtr/Value fields, so Trace is a no-op.
impl Trace for Blake3Hasher {
    fn trace(&self, _: &mut MarkVisitor) {}
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn as_blake3_hasher(v: &Value) -> Result<&Blake3Hasher, String> {
    match v {
        Value::NativeObject(obj) => obj
            .get()
            .downcast_ref::<Blake3Hasher>()
            .ok_or_else(|| format!("expected Blake3Hasher, got {}", obj.get().type_tag())),
        other => Err(format!("expected Blake3Hasher, got {}", other.type_name())),
    }
}

/// Convert a Clojure `Value` to raw bytes.
///
/// Accepts:
/// - `String` — hashed as UTF-8
/// - `Vector` of `Long` values in 0–255 — treated as a byte sequence
fn to_bytes(v: &Value) -> Result<Vec<u8>, String> {
    match v {
        Value::Str(s) => Ok(s.get().as_bytes().to_vec()),
        Value::Vector(vec) => {
            let mut bytes = Vec::with_capacity(vec.get().count());
            for elem in vec.get().iter() {
                match elem {
                    Value::Long(n) if (0..=255).contains(n) => bytes.push(*n as u8),
                    Value::Long(n) => return Err(format!("byte out of range 0–255: {n}")),
                    other => {
                        return Err(format!(
                            "byte vector must contain integers 0–255, got {}",
                            other.type_name()
                        ));
                    }
                }
            }
            Ok(bytes)
        }
        other => Err(format!(
            "expected string or byte vector, got {}",
            other.type_name()
        )),
    }
}

fn hash_to_hex(hash: &blake3::Hash) -> String {
    hash.to_hex().to_string()
}

fn hash_to_byte_vec(hash: &blake3::Hash) -> Vec<Value> {
    hash.as_bytes()
        .iter()
        .map(|b| Value::Long(*b as i64))
        .collect()
}

// ── cljrs_init ────────────────────────────────────────────────────────────────

/// Register all `blake3` namespace functions into the Clojure runtime.
///
/// The `cljrs compile` toolchain calls this automatically when the crate is
/// listed under `:rust :init` in `cljrs.edn`:
///
/// ```edn
/// {:rust {:crate "."
///         :init  "cljrs_blake3::cljrs_init"}}
/// ```
pub fn cljrs_init(registry: &mut Registry) {
    // blake3/hash — one-shot hash of a string or byte vector → 64-char hex
    registry.define(
        "blake3/hash",
        wrap_fn1("blake3/hash", |data: Value| -> Result<String, String> {
            let bytes = to_bytes(&data)?;
            Ok(hash_to_hex(&blake3::hash(&bytes)))
        }),
    );

    // blake3/hash-raw — one-shot hash → 32-element vector of byte integers
    registry.define(
        "blake3/hash-raw",
        wrap_fn1(
            "blake3/hash-raw",
            |data: Value| -> Result<Vec<Value>, String> {
                let bytes = to_bytes(&data)?;
                Ok(hash_to_byte_vec(&blake3::hash(&bytes)))
            },
        ),
    );

    // blake3/keyed-hash — BLAKE3 MAC; key must be exactly 32 bytes → hex
    registry.define(
        "blake3/keyed-hash",
        NativeFn::with_closure(
            "blake3/keyed-hash",
            Arity::Fixed(2),
            |args| -> ValueResult<Value> {
                let key_bytes = to_bytes(&args[0]).map_err(ValueError::Other)?;
                if key_bytes.len() != 32 {
                    return Err(ValueError::Other(format!(
                        "keyed-hash key must be exactly 32 bytes, got {}",
                        key_bytes.len()
                    )));
                }
                let key: [u8; 32] = key_bytes.try_into().unwrap();
                let data_bytes = to_bytes(&args[1]).map_err(ValueError::Other)?;
                Ok(hash_to_hex(&blake3::keyed_hash(&key, &data_bytes)).into_value())
            },
        ),
    );

    // blake3/derive-key — domain-separated KDF → 64-char hex string
    registry.define(
        "blake3/derive-key",
        wrap_fn2(
            "blake3/derive-key",
            |context: String, material: Value| -> Result<String, String> {
                let bytes = to_bytes(&material)?;
                let mut h = blake3::Hasher::new_derive_key(&context);
                h.update(&bytes);
                Ok(hash_to_hex(&h.finalize()))
            },
        ),
    );

    // blake3/hasher-new — create a new incremental hasher
    registry.define(
        "blake3/hasher-new",
        wrap_fn0("blake3/hasher-new", || -> Result<Value, String> {
            Ok(Value::NativeObject(gc_native_object(Blake3Hasher {
                inner: Mutex::new(blake3::Hasher::new()),
            })))
        }),
    );

    // blake3/hasher-update! — feed data into hasher; returns the same hasher
    registry.define(
        "blake3/hasher-update!",
        wrap_fn2(
            "blake3/hasher-update!",
            |h: Value, data: Value| -> Result<Value, String> {
                let bytes = to_bytes(&data)?;
                as_blake3_hasher(&h)?.inner.lock().unwrap().update(&bytes);
                Ok(h)
            },
        ),
    );

    // blake3/hasher-finalize — produce hex digest; hasher stays usable
    registry.define(
        "blake3/hasher-finalize",
        wrap_fn1(
            "blake3/hasher-finalize",
            |h: Value| -> Result<String, String> {
                Ok(hash_to_hex(
                    &as_blake3_hasher(&h)?.inner.lock().unwrap().finalize(),
                ))
            },
        ),
    );

    // blake3/hasher-finalize-raw — produce 32-element byte vector
    registry.define(
        "blake3/hasher-finalize-raw",
        wrap_fn1(
            "blake3/hasher-finalize-raw",
            |h: Value| -> Result<Vec<Value>, String> {
                Ok(hash_to_byte_vec(
                    &as_blake3_hasher(&h)?.inner.lock().unwrap().finalize(),
                ))
            },
        ),
    );

    registry.env().mark_loaded("blake3");
}
