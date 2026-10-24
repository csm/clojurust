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
//! - Large byte buffers (`Arc<ByteBlob>`, the BEAM off-heap-binary trick)
//!
//! Closures, native resources, and isolate-bound GC objects are not
//! promotable.  This restriction is enforced at publish time via `promote`.
//!
//! ## Memory accounting
//!
//! Shared allocations live outside every isolate heap, so each one owns a
//! governor [`MemoryCharge`] (`MemoryClass::SharedValue`): one per
//! [`ByteBlob`] and one per stored [`SharedSlot`].  Cloning the `Arc` does
//! not add a charge; dropping the last reference returns it.

use std::ops::Deref;
use std::sync::{Arc, Mutex};

use arc_swap::ArcSwap;
use cljrs_gc::{GcPtr, MemoryCharge, MemoryClass, MemoryLimitExceeded, StaticGcPtr, governor};

use crate::intern::{intern_keyword, intern_symbol};
use crate::keyword::Keyword;
use crate::symbol::Symbol;
use crate::value::Value;

// ── PromoteError ─────────────────────────────────────────────────────────────

/// Returned when a `Value` cannot be promoted to [`SharedValue`].
#[derive(Debug, Clone)]
pub struct PromoteError {
    pub type_name: &'static str,
    /// Set when the value is promotable but the governor refused its shared
    /// allocation (strict mode).
    pub memory: Option<MemoryLimitExceeded>,
}

impl PromoteError {
    pub fn not_promotable(type_name: &'static str) -> Self {
        Self {
            type_name,
            memory: None,
        }
    }

    /// The governor refused the shared allocation for a `type_name` value.
    pub fn memory_limit(type_name: &'static str, error: MemoryLimitExceeded) -> Self {
        Self {
            type_name,
            memory: Some(error),
        }
    }
}

impl PromoteError {
    /// The `ValueError` a builtin reports for this failure: the memory
    /// refusal itself, or `Other` with the message.
    pub fn into_value_error(self) -> crate::error::ValueError {
        match self.memory {
            Some(limit) => crate::error::ValueError::MemoryLimitExceeded(limit),
            None => crate::error::ValueError::Other(self.to_string()),
        }
    }
}

impl std::fmt::Display for PromoteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(limit) = &self.memory {
            return limit.fmt(f);
        }
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
    ByteBlob(Arc<ByteBlob>),
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

// ── Shared allocations ────────────────────────────────────────────────────────

/// Bytes of an `Arc<T>` allocation: two reference counts plus `T`.
const fn arc_bytes<T>() -> usize {
    2 * std::mem::size_of::<usize>() + std::mem::size_of::<T>()
}

/// Reserve `bytes` of shared-value memory for an infallible constructor.
/// A strict-mode refusal raises the thread's runtime fault and charges the
/// critical reserve instead, like `GcPtr::new`.
fn reserve(bytes: usize) -> Option<MemoryCharge> {
    match governor().reserve_shared(MemoryClass::SharedValue, bytes) {
        Ok(charge) => Some(charge),
        Err(refused) => {
            cljrs_gc::governor::raise_runtime_fault(refused);
            governor()
                .reserve_critical(MemoryClass::SharedValue, bytes)
                .ok()
        }
    }
}

/// Immutable byte buffer shared across isolates by `Arc` (`Value::ByteBlob`).
///
/// Owns one governor charge for its buffer and `Arc` allocation, however many
/// isolates hold a reference.
pub struct ByteBlob {
    bytes: Box<[u8]>,
    charge: Option<MemoryCharge>,
}

impl ByteBlob {
    /// Take ownership of `bytes` and charge them to the governor.
    /// Infallible: a strict-mode refusal raises the thread's runtime fault
    /// (see [`Self::try_new`]).
    pub fn new(bytes: impl Into<Box<[u8]>>) -> Self {
        let bytes = bytes.into();
        let charge = reserve(arc_bytes::<ByteBlob>() + bytes.len());
        Self { bytes, charge }
    }

    /// Take ownership of `bytes` if the governor admits their charge.  On
    /// refusal the buffer is dropped and the error returned.
    pub fn try_new(bytes: impl Into<Box<[u8]>>) -> Result<Self, MemoryLimitExceeded> {
        let bytes = bytes.into();
        let charge = governor().reserve_shared(
            MemoryClass::SharedValue,
            arc_bytes::<ByteBlob>() + bytes.len(),
        )?;
        Ok(Self {
            bytes,
            charge: Some(charge),
        })
    }

    /// Reserve a `len`-byte blob's charge before building its buffer, so a
    /// refused blob never allocates.  `fill` builds the buffer.
    pub fn try_with(
        len: usize,
        fill: impl FnOnce() -> Box<[u8]>,
    ) -> Result<Self, MemoryLimitExceeded> {
        let charge =
            governor().reserve_shared(MemoryClass::SharedValue, arc_bytes::<ByteBlob>() + len)?;
        Ok(Self {
            bytes: fill(),
            charge: Some(charge),
        })
    }

    /// The buffer contents.
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Bytes charged to the governor for this blob.
    pub fn charged_bytes(&self) -> usize {
        self.charge.as_ref().map_or(0, MemoryCharge::bytes)
    }
}

impl Deref for ByteBlob {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl AsRef<[u8]> for ByteBlob {
    fn as_ref(&self) -> &[u8] {
        &self.bytes
    }
}

impl From<Vec<u8>> for ByteBlob {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<&[u8]> for ByteBlob {
    fn from(bytes: &[u8]) -> Self {
        Self::new(bytes)
    }
}

impl std::fmt::Debug for ByteBlob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ByteBlob({} bytes)", self.bytes.len())
    }
}

/// One value stored in a [`SharedAtom`].  Each `reset!`/`swap!` stores a new
/// slot; the slot owns the governor charge for its `Arc` allocation and any
/// string it owns.  Byte-blob payloads are charged by the blob.
pub struct SharedSlot {
    value: SharedValue,
    charge: Option<MemoryCharge>,
}

impl SharedSlot {
    /// Wrap `value` and charge its storage to the governor.  Infallible: a
    /// strict-mode refusal raises the thread's runtime fault (see
    /// [`Self::try_new`]).
    pub fn new(value: SharedValue) -> Self {
        let charge = reserve(Self::bytes_for(&value));
        Self { value, charge }
    }

    /// Wrap `value` if the governor admits its storage charge.
    pub fn try_new(value: SharedValue) -> Result<Self, MemoryLimitExceeded> {
        let charge =
            governor().reserve_shared(MemoryClass::SharedValue, Self::bytes_for(&value))?;
        Ok(Self {
            value,
            charge: Some(charge),
        })
    }

    fn bytes_for(value: &SharedValue) -> usize {
        let payload = match value {
            SharedValue::Str(s) => s.len(),
            _ => 0,
        };
        arc_bytes::<SharedSlot>() + payload
    }

    /// The stored value.
    pub fn value(&self) -> &SharedValue {
        &self.value
    }

    /// Bytes charged to the governor for this slot.
    pub fn charged_bytes(&self) -> usize {
        self.charge.as_ref().map_or(0, MemoryCharge::bytes)
    }
}

impl Deref for SharedSlot {
    type Target = SharedValue;
    fn deref(&self) -> &SharedValue {
        &self.value
    }
}

impl AsRef<SharedValue> for SharedSlot {
    fn as_ref(&self) -> &SharedValue {
        &self.value
    }
}

impl std::fmt::Debug for SharedSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
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
        Value::Uuid(u) => Ok(SharedValue::Uuid(u.get().0)),
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
            // Reserve before copying, so a refused promotion builds nothing.
            let blob = ByteBlob::try_with(bytes.len(), || bytes.iter().map(|&b| b as u8).collect())
                .map_err(|e| PromoteError::memory_limit("byte-array", e))?;
            Ok(SharedValue::ByteBlob(Arc::new(blob)))
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
        SharedValue::Uuid(u) => Value::uuid(*u),
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
///
/// Each stored value is a [`SharedSlot`] that carries its own governor
/// charge; a replaced slot returns its charge when the last reader drops it.
#[derive(Debug)]
pub struct SharedAtom {
    pub cell: Arc<ArcSwap<SharedSlot>>,
    pub meta: Mutex<Option<SharedValue>>,
}

impl SharedAtom {
    pub fn new(val: SharedValue) -> Self {
        Self {
            cell: Arc::new(ArcSwap::new(Arc::new(SharedSlot::new(val)))),
            meta: Mutex::new(None),
        }
    }

    /// Fallible [`Self::new`]: refuse if the governor cannot admit the slot.
    pub fn try_new(val: SharedValue) -> Result<Self, MemoryLimitExceeded> {
        Ok(Self {
            cell: Arc::new(ArcSwap::new(Arc::new(SharedSlot::try_new(val)?))),
            meta: Mutex::new(None),
        })
    }

    /// Load the current value.  Lock-free atomic load + refcount bump.
    pub fn deref_val(&self) -> Arc<SharedSlot> {
        self.cell.load_full()
    }

    /// Atomically replace the value and return the new `Arc`.
    pub fn reset(&self, val: SharedValue) -> Arc<SharedSlot> {
        let arc = Arc::new(SharedSlot::new(val));
        self.cell.store(arc.clone());
        arc
    }

    /// Fallible [`Self::reset`]: the slot is reserved before it is
    /// published, so a refusal leaves the current value unchanged.
    pub fn try_reset(&self, val: SharedValue) -> Result<Arc<SharedSlot>, MemoryLimitExceeded> {
        let arc = Arc::new(SharedSlot::try_new(val)?);
        self.cell.store(arc.clone());
        Ok(arc)
    }

    /// CAS-retry swap: apply `f` to the current value and store the result.
    /// Returns the new value.  Retries automatically if another writer races.
    pub fn swap<F>(&self, mut f: F) -> Arc<SharedSlot>
    where
        F: FnMut(&SharedValue) -> SharedValue,
    {
        self.cell.rcu(|old| Arc::new(SharedSlot::new(f(old))))
    }

    /// Single lock-free compare-and-set.  Atomically stores `new` iff the cell
    /// still holds `current` (by `Arc` identity).  Returns `true` on success.
    ///
    /// This is the primitive used by Clojure-level `compare-and-set!` and by the
    /// retry loop behind `swap!`: a caller that needs to run arbitrary
    /// (interpreter) code between the load and the store cannot use the closure
    /// form of [`swap`], so it loads via [`deref_val`](Self::deref_val), computes
    /// the next value, then commits with this method, retrying on contention.
    pub fn compare_and_set(&self, current: &Arc<SharedSlot>, new: SharedValue) -> bool {
        let prev = self
            .cell
            .compare_and_swap(current, Arc::new(SharedSlot::new(new)));
        // The swap committed iff the value we replaced is the one we expected.
        std::ptr::eq(Arc::as_ptr(current), Arc::as_ptr(&prev))
    }

    /// Fallible [`Self::compare_and_set`]: the new slot is reserved before
    /// the swap, so a refusal leaves the current value unchanged.
    pub fn try_compare_and_set(
        &self,
        current: &Arc<SharedSlot>,
        new: SharedValue,
    ) -> Result<bool, MemoryLimitExceeded> {
        let slot = Arc::new(SharedSlot::try_new(new)?);
        let prev = self.cell.compare_and_swap(current, slot);
        Ok(std::ptr::eq(Arc::as_ptr(current), Arc::as_ptr(&prev)))
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
        assert!(matches!(val.value(), SharedValue::Long(42)));
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
        assert!(matches!(val.value(), SharedValue::Long(2)));
    }

    #[test]
    fn shared_atom_compare_and_set() {
        let atom = SharedAtom::new(SharedValue::Long(1));
        let cur = atom.deref_val();
        // Stale expectation succeeds while no one races us.
        assert!(atom.compare_and_set(&cur, SharedValue::Long(2)));
        assert!(matches!(atom.deref_val().value(), SharedValue::Long(2)));
        // `cur` is now stale: a second CAS against it must fail and not write.
        assert!(!atom.compare_and_set(&cur, SharedValue::Long(99)));
        assert!(matches!(atom.deref_val().value(), SharedValue::Long(2)));
    }

    #[test]
    fn shared_atom_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SharedAtom>();
        assert_send_sync::<Arc<SharedAtom>>();
    }

    #[test]
    fn byte_blob_shared_across_clone() {
        let blob = Arc::new(ByteBlob::new(vec![10u8, 20, 30]));
        let v1 = Value::ByteBlob(blob.clone());
        let v2 = Value::ByteBlob(blob.clone());
        // Same underlying buffer
        if let (Value::ByteBlob(a), Value::ByteBlob(b)) = (&v1, &v2) {
            assert!(Arc::ptr_eq(a, b));
        }
    }

    // Other tests charge the process governor concurrently, so these tests
    // check each value's own charge rather than the class total.

    #[test]
    fn byte_blob_holds_one_charge() {
        let blob = Arc::new(ByteBlob::new(vec![0u8; 4096]));
        assert!(blob.charged_bytes() >= 4096);
        let clone = blob.clone();
        assert_eq!(clone.charged_bytes(), blob.charged_bytes());
        assert_eq!(&clone[..3], &[0, 0, 0]);
    }

    #[test]
    fn shared_slot_charges_string_payload() {
        let short = SharedSlot::new(SharedValue::Long(1));
        let long = SharedSlot::new(SharedValue::Str(Arc::from("x".repeat(1000))));
        assert!(short.charged_bytes() > 0);
        assert_eq!(long.charged_bytes(), short.charged_bytes() + 1000);
    }

    #[test]
    fn shared_value_class_reports_live_charges() {
        let blob = ByteBlob::new(vec![1u8; 1 << 20]);
        // Concurrent tests only add their own small charges, and this blob
        // stays alive across the read.
        assert!(governor().class_bytes(MemoryClass::SharedValue) >= blob.charged_bytes());
    }
}
