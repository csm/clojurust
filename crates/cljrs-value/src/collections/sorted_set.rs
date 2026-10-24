use crate::Value;
use crate::collections::share_charge::{ShareCharge, rb_path_bytes};

/// An immutable sorted set backed by `rpds::RedBlackTreeSet`.
///
/// A set derived by `conj` or `disj` from a GC-allocated set reports only its
/// path-copied nodes from `gc_size_extra`; see `collections::share_charge`
/// for the full rule.
#[derive(Debug, Clone)]
pub struct SortedSet {
    inner: rpds::RedBlackTreeSetSync<Value>,
    charge: ShareCharge,
}

impl SortedSet {
    pub fn empty() -> Self {
        Self {
            inner: rpds::RedBlackTreeSetSync::new_sync(),
            charge: ShareCharge::full(),
        }
    }

    pub fn count(&self) -> usize {
        self.inner.size()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn contains(&self, value: &Value) -> bool {
        self.inner.contains(value)
    }

    /// Return a new set with `val` added.
    pub fn conj(&self, value: Value) -> Self {
        self.derived(self.inner.insert(value))
    }

    pub fn conj_mut(&mut self, value: Value) -> &mut Self {
        self.inner.insert_mut(value);
        self.charge = ShareCharge::full();
        self
    }

    /// Return a new set with `val` removed.
    pub fn disj(&self, value: &Value) -> Self {
        self.derived(self.inner.remove(value))
    }

    /// Wrap `inner`, produced by one path-copying update of `self.inner`.
    fn derived(&self, inner: rpds::RedBlackTreeSetSync<Value>) -> Self {
        let n = inner.size().max(self.inner.size());
        Self {
            charge: self
                .charge
                .derive(self.inner.size(), inner.size(), rb_path_bytes::<Value>(n)),
            inner,
        }
    }

    /// Estimated bytes owned by this set if it shared no nodes.
    fn full_size_extra(&self) -> usize {
        self.inner.size() * (40 + std::mem::size_of::<Value>())
    }

    /// Iterate over all elements in sorted order
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.inner.iter()
    }
}

impl FromIterator<Value> for SortedSet {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut inner = rpds::RedBlackTreeSetSync::new_sync();
        for v in iter {
            inner.insert_mut(v);
        }
        SortedSet {
            inner,
            charge: ShareCharge::full(),
        }
    }
}

impl PartialEq for SortedSet {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        self.inner.iter().all(|v| other.contains(v))
    }
}

impl cljrs_gc::Trace for SortedSet {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
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
    fn test_basic() {
        let s = SortedSet::empty();
        let s = s.conj(int(1)).conj(int(2)).conj(int(3));
        assert_eq!(s.count(), 3);
        assert!(s.contains(&int(1)));
        assert!(s.contains(&int(2)));
        assert!(!s.contains(&int(99)));
    }

    #[test]
    fn test_idempotent_conj() {
        let s = SortedSet::empty().conj(int(1)).conj(int(1));
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn test_disj() {
        let s = SortedSet::empty().conj(int(1)).conj(int(2));
        let s2 = s.disj(&int(1));
        assert!(!s2.contains(&int(1)));
        assert!(s2.contains(&int(2)));
        assert_eq!(s2.count(), 1);
    }

    #[test]
    fn test_equality_order_independent() {
        let a = SortedSet::from_iter([int(1), int(2), int(3)]);
        let b = SortedSet::from_iter([int(3), int(1), int(2)]);
        assert_eq!(a, b);
    }

    #[test]
    fn test_sorted_set_sorted() {
        let mut a = SortedSet::empty();
        a = a.conj(int(3));
        a = a.conj(int(1));
        a = a.conj(int(2));
        let v: Vec<Value> = a.iter().cloned().collect();
        assert!(matches!(&v[0], Value::Long(1)));
        assert!(matches!(&v[1], Value::Long(2)));
        assert!(matches!(&v[2], Value::Long(3)));
    }

    #[test]
    fn conj_on_allocated_set_charges_path_bytes() {
        use cljrs_gc::{GcPtr, Trace};

        let s = GcPtr::new(SortedSet::from_iter((0..10_000).map(int)));
        let added = s.get().conj(int(-1));
        assert_eq!(added.gc_size_extra(), rb_path_bytes::<Value>(10_001));
        assert!(added.gc_size_extra() * 100 < s.get().full_size_extra());
    }
}
