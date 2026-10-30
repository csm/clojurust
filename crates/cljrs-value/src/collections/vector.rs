use crate::Value;
use crate::collections::share_charge::{ShareCharge, vector_path_bytes};

/// An immutable persistent vector backed by `rpds::Vector`.
///
/// A vector may additionally be tagged as a *map entry* (see
/// [`PersistentVector::map_entry`]): a two-element `[key val]` pair produced
/// by seq'ing a map, `find`, or the `map-entry` builtin. Map entries behave
/// exactly like vectors (equality, hashing, printing, indexing) — the tag
/// only answers `map-entry?` — and, as in Clojure, any derived vector
/// (`conj`, `assoc`, `pop`, ...) is a plain vector again.
///
/// A vector derived by `conj`, `assoc_nth`, or `pop` from a GC-allocated
/// vector reports only its path-copied trie nodes from `gc_size_extra`; see
/// `collections::share_charge` for the full rule.
#[derive(Debug, Clone)]
pub struct PersistentVector {
    inner: rpds::VectorSync<Value>,
    is_map_entry: bool,
    charge: ShareCharge,
    // Conservative summary, maintained by updates without scanning shared nodes.
    scalar_only: bool,
}

pub(crate) fn scalar(value: &Value) -> bool {
    matches!(
        value,
        Value::Nil | Value::Bool(_) | Value::Long(_) | Value::Double(_) | Value::Char(_)
    )
}

impl PersistentVector {
    pub fn empty() -> Self {
        Self {
            inner: rpds::VectorSync::new_sync(),
            is_map_entry: false,
            charge: ShareCharge::full(),
            scalar_only: true,
        }
    }

    pub fn from_vector(vector: rpds::VectorSync<Value>) -> Self {
        let scalar_only = vector.iter().all(scalar);
        Self {
            scalar_only,
            inner: vector,
            is_map_entry: false,
            charge: ShareCharge::full(),
        }
    }

    pub(crate) fn from_vector_summary(vector: rpds::VectorSync<Value>, scalar_only: bool) -> Self {
        Self {
            inner: vector,
            scalar_only,
            is_map_entry: false,
            charge: ShareCharge::full(),
        }
    }

    /// Build a `[key val]` pair tagged as a map entry.
    pub fn map_entry(key: Value, val: Value) -> Self {
        let scalar_only = scalar(&key) && scalar(&val);
        let mut inner = rpds::VectorSync::new_sync();
        inner = inner.push_back(key);
        inner = inner.push_back(val);
        Self {
            inner,
            is_map_entry: true,
            scalar_only,
            charge: ShareCharge::full(),
        }
    }

    /// True only for vectors created via [`PersistentVector::map_entry`].
    pub fn is_map_entry(&self) -> bool {
        self.is_map_entry
    }

    pub fn count(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a value. O(log n) amortized.
    pub fn conj(&self, val: Value) -> Self {
        let scalar_only = self.scalar_only && scalar(&val);
        let mut result = self.derived(self.inner.push_back(val));
        result.scalar_only = scalar_only;
        result
    }

    /// Return the element at `idx`, or `None` if out of bounds.
    pub fn nth(&self, idx: usize) -> Option<&Value> {
        self.inner.get(idx)
    }

    /// Last element.
    pub fn peek(&self) -> Option<&Value> {
        self.inner.last()
    }

    /// Return a new vector with element `idx` replaced, or appended if `idx == len`.
    pub fn assoc_nth(&self, idx: usize, val: Value) -> Option<Self> {
        if idx == self.inner.len() {
            Some(self.conj(val))
        } else {
            let scalar_only = self.scalar_only && scalar(&val);
            let mut result = self.derived(self.inner.set(idx, val)?);
            result.scalar_only = scalar_only;
            Some(result)
        }
    }

    /// Remove the last element. Returns `None` if empty.
    pub fn pop(&self) -> Option<Self> {
        Some(self.derived(self.inner.drop_last()?))
    }

    /// Wrap `inner`, produced by one path-copying update of `self.inner`.
    fn derived(&self, inner: rpds::VectorSync<Value>) -> Self {
        let len = inner.len().max(self.inner.len());
        Self {
            charge: self.charge.derive(
                self.inner.len(),
                inner.len(),
                vector_path_bytes::<Value>(len),
            ),
            inner,
            is_map_entry: false,
            scalar_only: self.scalar_only,
        }
    }

    /// Estimated bytes owned by this vector if it shared no trie nodes.
    fn full_size_extra(&self) -> usize {
        // Per element: Arc<T> allocation (16 overhead) + thin ptr in leaf node (8).
        self.inner.len() * (24 + std::mem::size_of::<Value>())
    }

    /// What `gc_size_extra` would report, without marking this version
    /// allocated.
    #[cfg(test)]
    fn charged(&self) -> usize {
        self.charge.size_extra(self.full_size_extra())
    }

    /// Iterate over elements in index order.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.inner.iter()
    }

    pub(crate) fn scalar_only(&self) -> bool {
        self.scalar_only
    }

    pub fn inner(&self) -> &rpds::VectorSync<Value> {
        &self.inner
    }
}

impl std::iter::FromIterator<Value> for PersistentVector {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut v = rpds::VectorSync::new_sync();
        let mut scalar_only = true;
        for item in iter {
            scalar_only &= scalar(&item);
            v = v.push_back(item);
        }
        Self {
            inner: v,
            scalar_only,
            is_map_entry: false,
            charge: ShareCharge::full(),
        }
    }
}

impl PartialEq for PersistentVector {
    fn eq(&self, other: &Self) -> bool {
        if self.inner.len() != other.inner.len() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljrs_gc::Trace for PersistentVector {
    #[cfg(feature = "no-gc")]
    fn immutable_children(&self) -> bool {
        true
    }
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        if self.scalar_only {
            return;
        }
        for v in self.inner.iter() {
            v.trace(visitor);
        }
    }

    fn gc_size_extra(&self) -> usize {
        self.charge.on_alloc(self.full_size_extra())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn int(n: i64) -> Value {
        Value::Long(n)
    }

    #[test]
    fn test_empty() {
        let v = PersistentVector::empty();
        assert!(v.is_empty());
        assert_eq!(v.count(), 0);
        assert!(v.nth(0).is_none());
    }

    #[test]
    fn test_conj_small() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        assert_eq!(v.count(), 3);
        assert_eq!(v.nth(0), Some(&int(1)));
        assert_eq!(v.nth(2), Some(&int(3)));
    }

    #[test]
    fn test_conj_forces_tail_flush() {
        let v = PersistentVector::from_iter((0..33).map(int));
        assert_eq!(v.count(), 33);
        for i in 0..33 {
            assert_eq!(v.nth(i), Some(&int(i as i64)), "nth({i}) wrong");
        }
    }

    #[test]
    fn test_large() {
        let n = 1025;
        let v = PersistentVector::from_iter((0..n).map(|i| int(i as i64)));
        assert_eq!(v.count(), n);
        for i in 0..n {
            assert_eq!(v.nth(i), Some(&int(i as i64)));
        }
    }

    #[test]
    fn test_peek() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        assert_eq!(v.peek(), Some(&int(3)));
    }

    #[test]
    fn test_assoc_nth() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        let v2 = v.assoc_nth(1, int(99)).unwrap();
        assert_eq!(v2.nth(0), Some(&int(1)));
        assert_eq!(v2.nth(1), Some(&int(99)));
        assert_eq!(v2.nth(2), Some(&int(3)));
        // Original unchanged.
        assert_eq!(v.nth(1), Some(&int(2)));
    }

    #[test]
    fn test_pop() {
        let v = PersistentVector::from_iter([int(1), int(2), int(3)]);
        let v2 = v.pop().unwrap();
        assert_eq!(v2.count(), 2);
        assert_eq!(v2.nth(0), Some(&int(1)));
        assert_eq!(v2.nth(1), Some(&int(2)));
    }

    #[test]
    fn test_equality() {
        let a = PersistentVector::from_iter([int(1), int(2)]);
        let b = PersistentVector::from_iter([int(1), int(2)]);
        let c = PersistentVector::from_iter([int(1), int(3)]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_map_entry_flag() {
        let e = PersistentVector::map_entry(int(1), int(2));
        assert!(e.is_map_entry());
        assert_eq!(e.count(), 2);
        assert_eq!(e.nth(0), Some(&int(1)));
        assert_eq!(e.nth(1), Some(&int(2)));
        // Equal to a plain vector with the same elements.
        assert_eq!(e, PersistentVector::from_iter([int(1), int(2)]));
        // Plain constructors never produce map entries.
        assert!(!PersistentVector::from_iter([int(1), int(2)]).is_map_entry());
        // Derived vectors are plain vectors again.
        assert!(!e.conj(int(3)).is_map_entry());
        assert!(!e.assoc_nth(0, int(9)).unwrap().is_map_entry());
        assert!(!e.pop().unwrap().is_map_entry());
    }

    #[test]
    fn test_iter_order() {
        let v = PersistentVector::from_iter((0..10).map(|i| int(i as i64)));
        let items: Vec<_> = v.iter().cloned().collect();
        assert_eq!(items, (0..10).map(|i| int(i as i64)).collect::<Vec<_>>());
    }

    #[test]
    fn conj_on_allocated_vector_charges_log_n() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let path = vector_path_bytes::<Value>;
        let mut charges = Vec::new();
        for n in [1_000usize, 100_000] {
            let base = PersistentVector::from_iter((0..n as i64).map(int));
            let full = base.full_size_extra();
            let mut cur = GcPtr::new(base);
            assert_eq!(cur.get().charged(), full);
            // Stay below 2n so no step re-charges the whole vector.
            for i in 0..500 {
                let next = GcPtr::new(cur.get().conj(int(i)));
                let charge = next.get().charged();
                assert_eq!(charge, path(n + i as usize + 1));
                assert!(charge * 50 < full, "n={n}: {charge} vs full {full}");
                cur = next;
            }
            charges.push(cur.get().charged());
        }
        // 100x the elements costs one or two extra trie levels per step.
        assert!(charges[1] <= 2 * charges[0], "{charges:?}");
    }

    #[test]
    fn assoc_and_pop_on_allocated_vector_charge_log_n() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let v = GcPtr::new(PersistentVector::from_iter((0..10_000).map(int)));
        let set = v.get().assoc_nth(5, int(-1)).unwrap();
        assert_eq!(set.charged(), vector_path_bytes::<Value>(10_000));
        let popped = v.get().pop().unwrap();
        assert_eq!(popped.charged(), vector_path_bytes::<Value>(10_000));
    }

    #[test]
    fn conj_chain_recharges_in_full_on_doubling() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let mut cur = GcPtr::new(PersistentVector::from_iter((0..100).map(int)));
        let mut full_at = Vec::new();
        for i in 100..1_000 {
            let next = GcPtr::new(cur.get().conj(int(i)));
            if next.get().charged() == next.get().full_size_extra() {
                full_at.push(next.get().count());
            }
            cur = next;
        }
        assert_eq!(full_at, vec![200, 400, 800]);
    }

    #[test]
    fn unallocated_builder_chain_is_charged_in_full() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        // A Rust-side loop whose intermediates never reach the GC heap owns
        // every node of its result.
        let mut v = PersistentVector::empty();
        for i in 0..1_000 {
            v = v.conj(int(i));
        }
        assert_eq!(v.charged(), v.full_size_extra());
        // A clone shares its source's nodes and may outlive it.
        let shared = cljrs_gc::GcPtr::new(v);
        let copy = shared.get().clone();
        assert_eq!(copy.charged(), copy.full_size_extra());
    }
}
