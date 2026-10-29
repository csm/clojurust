//! Heap promotion of region-allocated values at publish boundaries
//! (Phase 10.5 — the safety net that lets bump regions coexist with the
//! tracing GC).
//!
//! Escape analysis decides which allocations may live in a scratch region;
//! correctness must not depend on that analysis being perfect.  This module
//! is the runtime fallback: every store into a program-lifetime cell
//! (`Var::bind`, `Atom`, `Volatile`) and every cross-thread handoff runs the
//! value through [`publish_value`]:
//!
//! * **Clean** — the value visibly contains no region-allocated `GcPtr`s
//!   (the overwhelmingly common case; one pointer-tag scan).  Stored as-is.
//! * **Dirty** — a region-tagged box was found.  The value is deep-copied to
//!   the GC heap via the structured-clone machinery (`crate::clone`) and the
//!   copy is stored instead; the region originals die with their region.
//! * **Opaque** — the value (or a part of it) cannot be scanned or copied
//!   (closures, unrealized lazy seqs, native objects, region-allocated
//!   mutable cells whose identity a copy would break).  The publisher then
//!   *poisons* the thread's active regions
//!   ([`cljrs_gc::region::poison_active_regions`]): each is retired (kept
//!   alive forever and traced as a GC root) instead of reset when its scope
//!   closes — a deliberate bounded leak that can never dangle.
//!
//! The whole protocol short-circuits to a single thread-local depth check
//! when no region is active, which is the case for virtually every top-level
//! `def`.
//!
//! No-gc publication retains backing scopes without copying values. Replaceable
//! destinations release their ownership edges when the payload changes. Raw
//! program-lifetime publications retain their actual backing scopes.

use crate::Value;
#[cfg(not(feature = "no-gc"))]
use crate::clone::{deserialize, serialize};
#[cfg(not(feature = "no-gc"))]
use crate::types::{DelayState, LazySeqState};
#[cfg(not(feature = "no-gc"))]
use crate::value::{MapValue, SetValue};
#[cfg(not(feature = "no-gc"))]
use cljrs_gc::{GcPtr, Trace};

/// Scan verdict for one value graph.
#[cfg(not(feature = "no-gc"))]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scan {
    /// No region-allocated box reachable.
    Clean,
    /// At least one region-allocated box found; all of it is copyable.
    Dirty,
    /// Contains something we can neither scan through nor deep-copy.
    Opaque,
}

#[cfg(not(feature = "no-gc"))]
impl Scan {
    fn join(self, other: Scan) -> Scan {
        use Scan::*;
        match (self, other) {
            (Opaque, _) | (_, Opaque) => Opaque,
            (Dirty, _) | (_, Dirty) => Dirty,
            _ => Clean,
        }
    }
}

/// Tag check for one box.
#[cfg(not(feature = "no-gc"))]
fn tag<T: Trace + 'static>(p: &GcPtr<T>) -> Scan {
    let dirty = p.is_region_alloc();
    if dirty { Scan::Dirty } else { Scan::Clean }
}

/// Like [`tag`] but for boxes whose pointee cannot be deep-copied (mutable
/// identity, closures, native state): a region-tagged box is unrecoverable.
#[cfg(not(feature = "no-gc"))]
fn tag_opaque<T: Trace + 'static>(p: &GcPtr<T>) -> Scan {
    if p.is_region_alloc() {
        Scan::Opaque
    } else {
        Scan::Clean
    }
}

#[cfg(not(feature = "no-gc"))]
fn scan_map(m: &MapValue) -> Scan {
    let mut acc = match m {
        MapValue::Array(p) => tag(p),
        MapValue::Hash(p) => tag(p),
        MapValue::Sorted(p) => tag(p),
    };
    m.for_each(|k, v| {
        acc = acc.join(scan(k)).join(scan(v));
    });
    acc
}

/// Scan a value graph for region-allocated boxes.
///
/// The conservative defaults matter more than the precise ones: anything we
/// cannot see through is `Opaque` *if* it could conceivably reference region
/// memory.  Heap-allocated mutable cells (`Atom`, `Volatile`, `Var`) are
/// `Clean` because their contents pass through this same barrier on every
/// store; heap closures and pending thunks are `Opaque` because their
/// captured environments are invisible here.
#[cfg(not(feature = "no-gc"))]
fn scan(v: &Value) -> Scan {
    use Scan::*;
    match v {
        // Immediates and Arc-backed values: nothing region-allocated.
        Value::Nil
        | Value::Bool(_)
        | Value::Long(_)
        | Value::Double(_)
        | Value::Char(_)
        | Value::ByteBlob(_)
        | Value::SharedAtom(_)
        | Value::Resource(_) => Clean,

        // Leaf boxes: copyable, no Value children.
        Value::BigInt(p) => tag(p),
        Value::BigDecimal(p) => tag(p),
        Value::Ratio(p) => tag(p),
        Value::Uuid(p) => tag(p),
        Value::Str(p) => tag(p),
        Value::Pattern(p) => tag(p),
        Value::Symbol(p) => tag(p),
        Value::Keyword(p) => tag(p),

        // Collections: box tag plus children.
        Value::List(p) => p.get().iter().fold(tag(p), |acc, e| acc.join(scan(e))),
        Value::Vector(p) => p.get().iter().fold(tag(p), |acc, e| acc.join(scan(e))),
        Value::Queue(p) => p.get().iter().fold(tag(p), |acc, e| acc.join(scan(e))),
        Value::Map(m) => scan_map(m),
        Value::Set(s) => {
            let t = match s {
                SetValue::Hash(p) => tag(p),
                SetValue::Sorted(p) => tag(p),
            };
            s.iter().fold(t, |acc, e| acc.join(scan(e)))
        }
        Value::Cons(p) => {
            let c = p.get();
            tag(p).join(scan(&c.head)).join(scan(&c.tail))
        }

        // Primitive arrays: contents are scalars; only the box matters.
        Value::IntArray(p) => tag(p),
        Value::LongArray(p) => tag(p),
        Value::ShortArray(p) => tag(p),
        Value::ByteArray(p) => tag(p),
        Value::FloatArray(p) => tag(p),
        Value::DoubleArray(p) => tag(p),
        Value::BooleanArray(p) => tag(p),
        Value::CharArray(p) => tag(p),
        Value::ObjectArray(p) => {
            let guard = p.get().0.lock().unwrap();
            guard.iter().fold(tag(p), |acc, e| acc.join(scan(e)))
        }

        // Wrappers.
        Value::WithMeta(inner, meta) => scan(inner).join(scan(meta)),
        Value::Reduced(inner) => scan(inner),

        // Records.
        Value::TypeInstance(p) => {
            if p.get().mutable.is_some() {
                Opaque
            } else {
                tag(p).join(scan_map(&p.get().fields))
            }
        }

        // Errors: box tag plus the data map / cause chain / thrown value.
        Value::Error(p) => {
            let mut acc = tag(p);
            let e = p.get();
            if let Some(data) = &e.data {
                acc = acc.join(scan_map(data));
            }
            if let crate::error::ValueError::Thrown(t) = &e.error {
                acc = acc.join(scan(t));
            }
            let mut cause = e.cause.clone();
            while let Some(c) = cause {
                acc = acc.join(tag(&c));
                if let Some(data) = &c.get().data {
                    acc = acc.join(scan_map(data));
                }
                cause = c.get().cause.clone();
            }
            acc
        }

        // Lazy cells: a realized cache is scannable; a pending thunk's
        // captured environment is not.
        Value::LazySeq(p) => {
            let t = tag_opaque(p);
            if t == Opaque {
                return Opaque;
            }
            match &*p.get().state.lock().unwrap() {
                LazySeqState::Forced(v) => t.join(scan(v)),
                LazySeqState::Error(_) => t,
                LazySeqState::Pending(_) | LazySeqState::Realizing { .. } => Opaque,
            }
        }
        Value::Delay(p) => {
            let t = tag_opaque(p);
            if t == Opaque {
                return Opaque;
            }
            match &*p.get().state.lock().unwrap() {
                DelayState::Forced(v) => t.join(scan(v)),
                DelayState::Pending(_) | DelayState::Realizing { .. } => Opaque,
                DelayState::Error(_) => t,
            }
        }

        // Mutable cells: identity-bearing, so a region-tagged box cannot be
        // promoted by copying.  Heap-allocated ones are Clean — every store
        // into them runs through this barrier itself.
        Value::Atom(p) => tag_opaque(p),
        Value::Volatile(p) => tag_opaque(p),
        Value::Var(p) => tag_opaque(p),
        Value::Promise(p) => tag_opaque(p),
        Value::Future(p) => tag_opaque(p),
        Value::Agent(p) => tag_opaque(p),
        Value::Namespace(p) => tag_opaque(p),
        Value::Protocol(p) => tag_opaque(p),

        // Closures and other callables: captured environments are invisible
        // here.  Under analysis-driven scopes captures are classified as
        // escaping (so they are never region-allocated), but we cannot verify
        // that locally — be conservative when one is published while a region
        // is open.
        Value::Fn(_)
        | Value::Macro(_)
        | Value::BoundFn(_)
        | Value::NativeFunction(_)
        | Value::ProtocolFn(_)
        | Value::MultiFn(_) => Opaque,

        // Mutable / native containers we cannot see through.
        Value::TransientMap(_)
        | Value::TransientSet(_)
        | Value::TransientVector(_)
        | Value::Matcher(_)
        | Value::NativeObject(_) => Opaque,
    }
}

/// Prepare `v` for publication into a program-lifetime cell (or another
/// thread): promote region-allocated parts to the GC heap, or poison the
/// active regions when the value is opaque to the scan.
///
/// Returns the value to store (the original, or its heap-promoted copy).
#[cfg(not(feature = "no-gc"))]
pub fn publish_value(v: Value) -> Value {
    // Fast path: no region open on this thread ⇒ nothing reachable from `v`
    // can be region-allocated (region values never outlive their region
    // except through exactly the publishes this barrier intercepts).
    if cljrs_gc::region::region_stack_depth() == 0
        || cljrs_gc::region::active_regions_are_poisoned()
    {
        return v;
    }
    // An opaque lazy tail poisons a scope once. Later cells need no graph
    // scan: every active region is already guaranteed to outlive its values.
    match scan(&v) {
        Scan::Clean => v,
        Scan::Dirty => match serialize(&v) {
            Ok(wire) => deserialize(wire),
            Err(_) => {
                cljrs_gc::region::poison_active_regions();
                v
            }
        },
        Scan::Opaque => {
            // Only poison when the opaque value could actually be holding
            // region memory; a fully-Clean graph with an opaque *leaf* is
            // still unverifiable, so this branch is reached for both.
            cljrs_gc::region::poison_active_regions();
            v
        }
    }
}

/// GC publication uses the same promotion barrier for every destination.
#[cfg(not(feature = "no-gc"))]
pub fn publish_for_owner(value: Value, _address: usize) -> Value {
    publish_value(value)
}

/// Retain actual backing scopes for an explicit program-lifetime result.
/// Values preserve their identity and laziness.
#[cfg(feature = "no-gc")]
pub fn publish_value(value: Value) -> Value {
    if !cljrs_gc::alloc_ctx::invocation_is_active() {
        cljrs_gc::retain_regions_for(&value);
    }
    value
}

#[cfg(feature = "no-gc")]
pub fn publish_for_owner(value: Value, address: usize) -> Value {
    publish_for_slot(value, address, 0)
}

/// Publish one independently replaceable mutable field or array index.
#[cfg(feature = "no-gc")]
pub fn publish_for_slot(value: Value, address: usize, slot: usize) -> Value {
    if cljrs_gc::alloc_ctx::invocation_is_active() {
        return value;
    }
    let (value, _compact) = compact_scalar(value);
    let lease = cljrs_gc::region::RegionLease::for_value(&value);
    cljrs_gc::region::AllocationOwner::for_address(address).set_slot(address, slot, &lease);
    value
}
#[cfg(not(feature = "no-gc"))]
pub fn publish_for_slot(value: Value, _address: usize, _slot: usize) -> Value {
    publish_value(value)
}
/// Publish a field identified by a stable application key.
pub fn publish_for_named_slot(value: Value, address: usize, slot: String) -> Value {
    #[cfg(feature = "no-gc")]
    {
        if cljrs_gc::alloc_ctx::invocation_is_active() {
            return value;
        }
        let lease = cljrs_gc::region::RegionLease::for_value(&value);
        cljrs_gc::region::AllocationOwner::for_address(address).set_slot(address, slot, &lease);
        value
    }
    #[cfg(not(feature = "no-gc"))]
    {
        let _ = (address, slot);
        publish_value(value)
    }
}
/// Protect a previously published slot while its caller uses the returned value.
pub fn read_slot(address: usize, slot: usize) {
    #[cfg(feature = "no-gc")]
    cljrs_gc::region::AllocationOwner::for_address(address).read_slot(address, slot);
    #[cfg(not(feature = "no-gc"))]
    let _ = (address, slot);
}

/// Borrow a named field while its returned value is in use.
pub fn read_named_slot(address: usize, slot: String) {
    #[cfg(feature = "no-gc")]
    cljrs_gc::region::AllocationOwner::for_address(address).read_slot(address, slot);
    #[cfg(not(feature = "no-gc"))]
    let _ = (address, slot);
}

/// Release cached slots before their containing object is destroyed.
pub fn clear_slots(address: usize) {
    #[cfg(feature = "no-gc")]
    cljrs_gc::region::AllocationOwner::for_address(address).clear_slots(address);
    #[cfg(not(feature = "no-gc"))]
    let _ = address;
}

/// A payload snapshot held across a callback or replacement.
pub struct ValueSnapshot {
    pub value: Value,
    #[cfg(feature = "no-gc")]
    pub(crate) lease: cljrs_gc::region::RegionLease,
}
impl ValueSnapshot {
    pub fn protect_current(&self) {
        #[cfg(feature = "no-gc")]
        self.lease.protect_read();
    }
}

/// Cached backing scopes for a replaceable payload. Regional containers use
/// replaceable weak edges, while permanent containers own their current payload.
#[cfg(feature = "no-gc")]
#[derive(Debug, Default)]
pub struct ValueLease {
    owned: Option<cljrs_gc::region::RegionLease>,
    scoped: cljrs_gc::region::RegionDependency,
}
#[cfg(feature = "no-gc")]
impl ValueLease {
    pub fn for_values(values: &[Value], owner: &cljrs_gc::region::AllocationOwner) -> Self {
        if cljrs_gc::alloc_ctx::invocation_is_active() {
            return Self::default();
        }
        Self::from_lease(cljrs_gc::region::RegionLease::for_values(values), owner)
    }
    fn from_lease(
        lease: cljrs_gc::region::RegionLease,
        owner: &cljrs_gc::region::AllocationOwner,
    ) -> Self {
        if owner.is_permanent() {
            Self {
                owned: Some(lease),
                scoped: Default::default(),
            }
        } else {
            Self {
                scoped: owner.replaceable(&lease),
                owned: None,
            }
        }
    }
    pub fn snapshot(&self) -> cljrs_gc::region::RegionLease {
        self.owned
            .as_ref()
            .cloned()
            .unwrap_or_else(|| self.scoped.snapshot())
    }
    pub fn protect_read(&self) {
        if let Some(lease) = &self.owned {
            lease.protect_read();
        } else {
            self.scoped.snapshot().protect_read();
        }
    }
    pub fn protect_current(&self) {
        if let Some(lease) = &self.owned {
            lease.protect_current();
        } else {
            self.scoped.snapshot().protect_current();
        }
    }
}

// Immutable leaf copies retain their logical identity. Collections and mutable
// values keep their allocation identity and structural sharing unchanged.
#[cfg(feature = "no-gc")]
fn compact_scalar(value: Value) -> (Value, Option<cljrs_gc::alloc_ctx::ScratchGuard>) {
    macro_rules! copy_leaf {
        ($pointer:expr, $variant:path) => {{
            let pointer = $pointer;
            if pointer.is_static_alloc() {
                return ($variant(pointer), None);
            }
            let arena = cljrs_gc::alloc_ctx::ScratchGuard::new();
            // SAFETY: each matched type is an immutable scalar with a value-preserving Clone.
            let copy = unsafe { pointer.clone_immutable() };
            return ($variant(copy), Some(arena));
        }};
    }
    match value {
        Value::Str(p) if p.get().len() <= 4096 => copy_leaf!(p, Value::Str),
        Value::Uuid(p) => copy_leaf!(p, Value::Uuid),
        Value::BigInt(p) if p.get().bits() <= 32768 => copy_leaf!(p, Value::BigInt),
        Value::Ratio(p)
            if p.get()
                .numer()
                .bits()
                .saturating_add(p.get().denom().bits())
                <= 32768 =>
        {
            copy_leaf!(p, Value::Ratio)
        }
        other => (other, None),
    }
}

/// Prepare a replaceable payload without copying identity-bearing values.
#[cfg(feature = "no-gc")]
pub fn prepare_for_owner(
    value: Value,
    owner: &cljrs_gc::region::AllocationOwner,
) -> (Value, ValueLease) {
    use cljrs_gc::region::RegionLease;
    if cljrs_gc::alloc_ctx::invocation_is_active() {
        return (value, Default::default());
    }
    let (value, _compact) = compact_scalar(value);
    let lease = RegionLease::for_value(&value);
    let cached = ValueLease::from_lease(lease, owner);
    (value, cached)
}

/// Transfer a result into its caller without retaining unrelated scratch data.
#[cfg(feature = "no-gc")]
pub fn publish_return(value: Value) -> Value {
    if cljrs_gc::alloc_ctx::is_static_context() && !cljrs_gc::region::ReadScope::is_active() {
        publish_value(value)
    } else {
        cljrs_gc::region::RegionLease::for_value(&value).protect_current();
        value
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, not(feature = "no-gc")))]
mod tests {
    use super::*;
    use crate::PersistentVector;
    use cljrs_gc::region::{Region, RegionGuard};

    /// Allocate a vector value whose box lives in `region`.
    fn region_vec(region: &mut Region, items: Vec<Value>) -> Value {
        Value::Vector(region.alloc(PersistentVector::from_iter(items)))
    }

    #[test]
    fn clean_value_passes_through_untouched() {
        let mut region = Region::new();
        let _guard = unsafe { RegionGuard::new(&mut region) };
        let v = Value::Vector(GcPtr::new(PersistentVector::from_iter([Value::Long(1)])));
        let Value::Vector(before) = &v else { panic!() };
        let before = before.clone();
        let out = publish_value(v);
        let Value::Vector(after) = &out else { panic!() };
        assert!(
            GcPtr::ptr_eq(&before, after),
            "heap value must not be copied"
        );
    }

    #[test]
    fn region_value_is_promoted_to_heap() {
        let mut region = Region::new();
        let inner = region_vec(&mut region, vec![Value::Long(1), Value::Long(2)]);
        // A heap map containing a region-allocated vector: the barrier must
        // deep-copy the whole graph so nothing region-tagged is stored.
        let map = Value::Map(crate::MapValue::from_pairs(vec![(Value::Long(0), inner)]));
        let _guard = unsafe { RegionGuard::new(&mut region) };
        let out = publish_value(map);
        assert_eq!(scan(&out), Scan::Clean, "promoted copy must be heap-only");
        // Structure preserved.
        let Value::Map(m) = &out else { panic!() };
        let got = {
            let mut got = None;
            m.for_each(|_, v| got = Some(v.clone()));
            got.unwrap()
        };
        assert_eq!(
            got,
            Value::Vector(GcPtr::new(PersistentVector::from_iter([
                Value::Long(1),
                Value::Long(2)
            ])))
        );
    }

    #[test]
    fn no_active_region_short_circuits() {
        // Even a region-tagged value passes through when no region is active
        // on this thread (the barrier cannot help at that point; the fast
        // path must stay one thread-local read).
        let mut region = Region::new();
        let v = region_vec(&mut region, vec![Value::Long(5)]);
        let out = publish_value(v.clone());
        assert_eq!(out, v);
    }

    #[test]
    fn retained_regions_skip_repeated_publication_and_keep_values_alive() {
        let mut region = Box::new(Region::new());
        let child = region_vec(&mut region, vec![Value::Long(42)]);
        let value = Value::Vector(GcPtr::new(PersistentVector::from_iter([child])));
        unsafe { cljrs_gc::region::push_region_raw(region.as_mut() as *mut Region) };
        cljrs_gc::region::poison_active_regions();
        let out = publish_value(value.clone());
        let (Value::Vector(before), Value::Vector(after)) = (&value, &out) else {
            panic!()
        };
        assert!(
            GcPtr::ptr_eq(before, after),
            "retained graphs need no scan or copy"
        );
        cljrs_gc::region::close_region(region);
        assert_eq!(format!("{out}"), "[[42]]");
    }

    #[test]
    fn opaque_value_poisons_active_regions() {
        use std::sync::{Arc, Mutex};
        #[derive(Debug)]
        struct Tracked(Arc<Mutex<bool>>);
        impl Drop for Tracked {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = true;
            }
        }
        impl Trace for Tracked {
            fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
        }

        let dropped = Arc::new(Mutex::new(false));
        let mut region = Box::new(Region::new());
        region.alloc(Tracked(dropped.clone()));
        // An unrealized lazy seq is opaque to the scan.
        let lazy = {
            struct T;
            impl std::fmt::Debug for T {
                fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    write!(f, "T")
                }
            }
            impl Trace for T {
                fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
            }
            impl crate::types::Thunk for T {
                fn force(&self) -> Result<Value, String> {
                    Ok(Value::Nil)
                }
            }
            Value::LazySeq(GcPtr::new(crate::types::LazySeq::new(Box::new(T))))
        };

        unsafe { cljrs_gc::region::push_region_raw(region.as_mut() as *mut Region) };
        let _ = publish_value(lazy);
        cljrs_gc::region::close_region(region);
        assert!(
            !*dropped.lock().unwrap(),
            "publishing an opaque value must retire the active region"
        );
    }
}
