//! Cross-isolate shared mutable state: `SharedValue`, `SharedAtom` (Phase B3).
//!
//! The isolate model is share-nothing for GC-heap data.  But some state
//! genuinely needs to be visible across isolates — global configuration, shared
//! counters, published results.  Phase B3 adds an *explicit*, *honest* escape
//! hatch: `shared-atom`.
//!
//! ## Two-tier mutable references (per the ADR)
//!
//! | Primitive | Backing | `Send`? | Use case |
//! |-----------|---------|---------|----------|
//! | `atom`      | `GcPtr<Atom>` (`Mutex<Value>`) | `!Send` | isolate-local, fast |
//! | `shared-atom` | `Arc<SharedAtom>` (`ArcSwap<SharedValue>`) | ✓ | cross-isolate, lock-free CAS |
//!
//! A value stored in a `shared-atom` must be **promotable** — it must be
//! representable as a `SharedValue`.  The promotion cost is paid once on
//! publish; reads (`deref`) are an atomic load.
//!
//! ## `SharedValue`
//!
//! Covers only the "plain data" subset of `Value`:
//! - Scalars (stored inline, no allocation)
//! - Strings (`Arc<str>`, immutable, refcounted)
//! - Keywords / symbols (`StaticGcPtr<T>`, interned, program-lifetime)
//! - Large byte buffers (`Arc<[u8]>`, the BEAM off-heap-binary trick)
//!
//! Closures, native resources, and isolate-bound GC objects are not
//! promotable.  This restriction is enforced at publish time via `promote`.

use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use cljrs_gc::{GcPtr, StaticGcPtr};

use crate::intern::{intern_keyword, intern_symbol};
use crate::keyword::Keyword;
use crate::symbol::Symbol;
use crate::value::Value;

// ── PromoteError ─────────────────────────────────────────────────────────────

/// Returned when a `Value` cannot be promoted to [`SharedValue`].
#[derive(Debug, Clone)]
pub struct PromoteError {
    pub type_name: &'static str,
}

impl PromoteError {
    pub fn not_promotable(type_name: &'static str) -> Self {
        Self { type_name }
    }
}

impl std::fmt::Display for PromoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "value of type '{}' cannot be promoted to a shared-atom: \
             only scalars, strings, keywords, symbols, and byte-blobs are supported",
            self.type_name
        )
    }
}

impl std::error::Error for PromoteError {}

// ── SharedValue ───────────────────────────────────────────────────────────────

/// A `Send + Sync` value representation for cross-isolate sharing.
///
/// Covers a carefully chosen subset of `Value`: scalars (stored inline),
/// immutable strings and byte buffers (refcounted via `Arc`), and interned
/// keywords/symbols (program-lifetime `StaticGcPtr`).  Anything that holds
/// isolate-local `GcPtr`s is excluded.
///
/// Cycles are impossible — `SharedValue` contains no `SharedValue` references
/// — so plain `Arc` refcounting is sufficient and cycle-free.
#[derive(Debug, Clone)]
pub enum SharedValue {
    Nil,
    Bool(bool),
    Long(i64),
    Double(f64),
    Char(char),
    Uuid(u128),
    /// Immutable interned string slice.
    Str(Arc<str>),
    /// Keyword interned into the global static arena.
    Keyword(StaticGcPtr<Keyword>),
    /// Symbol interned into the global static arena.
    Symbol(StaticGcPtr<Symbol>),
    /// Large refcounted byte buffer (the BEAM off-heap-binary trick).
    /// Shared without copy across the isolate boundary; freed when the last
    /// reference is dropped.
    ByteBlob(Arc<[u8]>),
}

// SAFETY: all variants are either value types or `Arc`/`StaticGcPtr` wrappers
// that are themselves `Send + Sync`.
unsafe impl Send for SharedValue {}
unsafe impl Sync for SharedValue {}

impl SharedValue {
    pub fn type_name(&self) -> &'static str {
        match self {
            SharedValue::Nil => "nil",
            SharedValue::Bool(_) => "boolean",
            SharedValue::Long(_) => "long",
            SharedValue::Double(_) => "double",
            SharedValue::Char(_) => "char",
            SharedValue::Uuid(_) => "uuid",
            SharedValue::Str(_) => "string",
            SharedValue::Keyword(_) => "keyword",
            SharedValue::Symbol(_) => "symbol",
            SharedValue::ByteBlob(_) => "byte-blob",
        }
    }
}

// ── promote / demote ──────────────────────────────────────────────────────────

/// Promote a `Value` to a [`SharedValue`] for cross-isolate publishing.
///
/// The promotion cost is paid once (at `reset!`/`swap!` time); reads of the
/// resulting `SharedAtom` are lock-free atomic loads.
///
/// Returns [`PromoteError`] for values that hold isolate-local state.
pub fn promote(value: &Value) -> Result<SharedValue, PromoteError> {
    match value {
        Value::Nil => Ok(SharedValue::Nil),
        Value::Bool(b) => Ok(SharedValue::Bool(*b)),
        Value::Long(n) => Ok(SharedValue::Long(*n)),
        Value::Double(d) => Ok(SharedValue::Double(*d)),
        Value::Char(c) => Ok(SharedValue::Char(*c)),
        Value::Uuid(u) => Ok(SharedValue::Uuid(*u)),
        Value::Str(s) => Ok(SharedValue::Str(Arc::from(s.get().as_str()))),
        Value::Keyword(kw) => {
            let kw = kw.get();
            let ptr = intern_keyword(kw.namespace.as_deref(), &kw.name);
            Ok(SharedValue::Keyword(ptr))
        }
        Value::Symbol(sym) => {
            let sym = sym.get();
            let ptr = intern_symbol(sym.namespace.as_deref(), &sym.name, sym.version.as_deref());
            Ok(SharedValue::Symbol(ptr))
        }
        Value::ByteArray(arr) => {
            let bytes = arr.get().lock().unwrap();
            let blob: Arc<[u8]> = bytes.iter().map(|&b| b as u8).collect::<Vec<_>>().into();
            Ok(SharedValue::ByteBlob(blob))
        }
        Value::ByteBlob(blob) => Ok(SharedValue::ByteBlob(blob.clone())),
        other => Err(PromoteError::not_promotable(other.type_name())),
    }
}

/// Demote a [`SharedValue`] back into an isolate-local `Value`.
///
/// Always succeeds.  Keywords and symbols are cloned from their interned
/// static-arena allocation into a fresh `GcPtr` on the calling isolate's heap.
/// Map lookups compare by namespace/name content (not pointer identity) so
/// equality is preserved across the round-trip.
pub fn demote(sv: &SharedValue) -> Value {
    match sv {
        SharedValue::Nil => Value::Nil,
        SharedValue::Bool(b) => Value::Bool(*b),
        SharedValue::Long(n) => Value::Long(*n),
        SharedValue::Double(d) => Value::Double(*d),
        SharedValue::Char(c) => Value::Char(*c),
        SharedValue::Uuid(u) => Value::Uuid(*u),
        SharedValue::Str(s) => Value::Str(GcPtr::new(s.as_ref().to_owned())),
        SharedValue::Keyword(kw) => Value::Keyword(GcPtr::new(kw.get().clone())),
        SharedValue::Symbol(sym) => Value::Symbol(GcPtr::new(sym.get().clone())),
        SharedValue::ByteBlob(blob) => Value::ByteBlob(blob.clone()),
    }
}

// ── SharedAtom ────────────────────────────────────────────────────────────────

/// A cross-isolate mutable reference backed by a lock-free `ArcSwap`.
///
/// `reset!` and `swap!` promote the new value and perform an atomic pointer
/// swap — no locking, O(1) for simple resets, O(retries) for CAS-retry
/// `swap!` under write contention.  `deref` is a single atomic load plus an
/// `Arc` reference-count bump.
///
/// Wrapped in `Value::SharedAtom(Arc<SharedAtom>)`.  The `Arc` gives the atom
/// identity (pointer equality) and allows any number of isolates to hold a
/// reference.
#[derive(Debug)]
pub struct SharedAtom {
    pub cell: Arc<ArcSwap<SharedValue>>,
    pub meta: Mutex<Option<SharedValue>>,
}

impl SharedAtom {
    pub fn new(val: SharedValue) -> Self {
        Self {
            cell: Arc::new(ArcSwap::new(Arc::new(val))),
            meta: Mutex::new(None),
        }
    }

    /// Load the current value.  Lock-free atomic load + refcount bump.
    pub fn deref_val(&self) -> Arc<SharedValue> {
        self.cell.load_full()
    }

    /// Atomically replace the value and return the new `Arc`.
    pub fn reset(&self, val: SharedValue) -> Arc<SharedValue> {
        let arc = Arc::new(val);
        self.cell.store(arc.clone());
        arc
    }

    /// CAS-retry swap: apply `f` to the current value and store the result.
    /// Returns the new value.  Retries automatically if another writer races.
    pub fn swap<F>(&self, mut f: F) -> Arc<SharedValue>
    where
        F: FnMut(&SharedValue) -> SharedValue,
    {
        self.cell.rcu(|old| Arc::new(f(old)))
    }

    /// Single lock-free compare-and-set.  Atomically stores `new` iff the cell
    /// still holds `current` (by `Arc` identity).  Returns `true` on success.
    ///
    /// This is the primitive used by Clojure-level `compare-and-set!` and by the
    /// retry loop behind `swap!`: a caller that needs to run arbitrary
    /// (interpreter) code between the load and the store cannot use the closure
    /// form of [`swap`], so it loads via [`deref_val`](Self::deref_val), computes
    /// the next value, then commits with this method, retrying on contention.
    pub fn compare_and_set(&self, current: &Arc<SharedValue>, new: SharedValue) -> bool {
        let prev = self.cell.compare_and_swap(current, Arc::new(new));
        // The swap committed iff the value we replaced is the one we expected.
        std::ptr::eq(Arc::as_ptr(current), Arc::as_ptr(&prev))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::keyword::Keyword;
    use crate::value::Value;
    use cljrs_gc::GcPtr;

    use super::*;

    fn kw(name: &str) -> Value {
        Value::Keyword(GcPtr::new(Keyword::simple(name)))
    }

    #[test]
    fn promote_scalars() {
        assert!(matches!(promote(&Value::Nil), Ok(SharedValue::Nil)));
        assert!(matches!(
            promote(&Value::Bool(true)),
            Ok(SharedValue::Bool(true))
        ));
        assert!(matches!(
            promote(&Value::Long(42)),
            Ok(SharedValue::Long(42))
        ));
    }

    #[test]
    fn promote_keyword_interns() {
        let v = kw("foo");
        let sv = promote(&v).unwrap();
        let sv2 = promote(&kw("foo")).unwrap();
        if let (SharedValue::Keyword(a), SharedValue::Keyword(b)) = (sv, sv2) {
            assert!(
                cljrs_gc::StaticGcPtr::ptr_eq(&a, &b),
                "same keyword should intern to same StaticGcPtr"
            );
        } else {
            panic!("expected SharedValue::Keyword");
        }
    }

    #[test]
    fn demote_roundtrip_long() {
        let sv = SharedValue::Long(99);
        assert!(matches!(demote(&sv), Value::Long(99)));
    }

    #[test]
    fn demote_roundtrip_keyword() {
        let sv = promote(&kw("test")).unwrap();
        let v = demote(&sv);
        if let Value::Keyword(kw_ptr) = v {
            assert_eq!(kw_ptr.get().name.as_ref(), "test");
        } else {
            panic!("expected Value::Keyword");
        }
    }

    #[test]
    fn promote_non_promotable_returns_err() {
        let atom = Value::Atom(GcPtr::new(crate::types::Atom::new(Value::Nil)));
        assert!(promote(&atom).is_err());
    }

    #[test]
    fn promote_byte_blob() {
        let arr: Vec<i8> = vec![1, 2, 3];
        let v = Value::ByteArray(GcPtr::new(std::sync::Mutex::new(arr)));
        let sv = promote(&v).unwrap();
        assert!(matches!(sv, SharedValue::ByteBlob(_)));
    }

    #[test]
    fn shared_atom_reset_and_deref() {
        let atom = SharedAtom::new(SharedValue::Long(0));
        atom.reset(SharedValue::Long(42));
        let val = atom.deref_val();
        assert!(matches!(val.as_ref(), SharedValue::Long(42)));
    }

    #[test]
    fn shared_atom_swap() {
        let atom = SharedAtom::new(SharedValue::Long(1));
        atom.swap(|old| {
            if let SharedValue::Long(n) = old {
                SharedValue::Long(n + 1)
            } else {
                SharedValue::Long(0)
            }
        });
        let val = atom.deref_val();
        assert!(matches!(val.as_ref(), SharedValue::Long(2)));
    }

    #[test]
    fn shared_atom_compare_and_set() {
        let atom = SharedAtom::new(SharedValue::Long(1));
        let cur = atom.deref_val();
        // Stale expectation succeeds while no one races us.
        assert!(atom.compare_and_set(&cur, SharedValue::Long(2)));
        assert!(matches!(atom.deref_val().as_ref(), SharedValue::Long(2)));
        // `cur` is now stale: a second CAS against it must fail and not write.
        assert!(!atom.compare_and_set(&cur, SharedValue::Long(99)));
        assert!(matches!(atom.deref_val().as_ref(), SharedValue::Long(2)));
    }

    #[test]
    fn shared_atom_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedAtom>();
        assert_send_sync::<Arc<SharedAtom>>();
    }

    #[test]
    fn byte_blob_shared_across_clone() {
        let blob: Arc<[u8]> = vec![10u8, 20, 30].into();
        let v1 = Value::ByteBlob(blob.clone());
        let v2 = Value::ByteBlob(blob.clone());
        // Same underlying buffer
        if let (Value::ByteBlob(a), Value::ByteBlob(b)) = (&v1, &v2) {
            assert!(Arc::ptr_eq(a, b));
        }
    }
}
