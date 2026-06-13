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
//! Only compiled in GC builds; the `no-gc` build keeps its `StaticCtxGuard`
//! discipline (and debug provenance assertions) instead.

use crate::Value;
use crate::clone::{deserialize, serialize};
use crate::types::{DelayState, LazySeqState};
use crate::value::{MapValue, SetValue};
use cljrs_gc::{GcPtr, Trace};

/// Scan verdict for one value graph.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scan {
    /// No region-allocated box reachable.
    Clean,
    /// At least one region-allocated box found; all of it is copyable.
    Dirty,
    /// Contains something we can neither scan through nor deep-copy.
    Opaque,
}

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
fn tag<T: Trace + 'static>(p: &GcPtr<T>) -> Scan {
    if p.is_region_alloc() {
        Scan::Dirty
    } else {
        Scan::Clean
    }
}

/// Like [`tag`] but for boxes whose pointee cannot be deep-copied (mutable
/// identity, closures, native state): a region-tagged box is unrecoverable.
fn tag_opaque<T: Trace + 'static>(p: &GcPtr<T>) -> Scan {
    if p.is_region_alloc() {
        Scan::Opaque
    } else {
        Scan::Clean
    }
}

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
fn scan(v: &Value) -> Scan {
    use Scan::*;
    match v {
        // Immediates and Arc-backed values: nothing region-allocated.
        Value::Nil
        | Value::Bool(_)
        | Value::Long(_)
        | Value::Double(_)
        | Value::Char(_)
        | Value::Uuid(_)
        | Value::ByteBlob(_)
        | Value::SharedAtom(_)
        | Value::Resource(_) => Clean,

        // Leaf boxes: copyable, no Value children.
        Value::BigInt(p) => tag(p),
        Value::BigDecimal(p) => tag(p),
        Value::Ratio(p) => tag(p),
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
        Value::TypeInstance(p) => tag(p).join(scan_map(&p.get().fields)),

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
                LazySeqState::Pending(_) => Opaque,
            }
        }
        Value::Delay(p) => {
            let t = tag_opaque(p);
            if t == Opaque {
                return Opaque;
            }
            match &*p.get().state.lock().unwrap() {
                DelayState::Forced(v) => t.join(scan(v)),
                DelayState::Pending(_) => Opaque,
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
pub fn publish_value(v: Value) -> Value {
    // Fast path: no region open on this thread ⇒ nothing reachable from `v`
    // can be region-allocated (region values never outlive their region
    // except through exactly the publishes this barrier intercepts).
    if cljrs_gc::region::region_stack_depth() == 0 {
        return v;
    }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
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
