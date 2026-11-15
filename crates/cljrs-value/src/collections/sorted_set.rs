use crate::Value;
use crate::collections::share_charge::{ShareCharge, rb_path_bytes};
use crate::compare::{SortKey, checked};
use crate::error::ValueResult;

/// An immutable sorted set backed by `rpds::RedBlackTreeSet`.
///
/// Elements are ordered by [`crate::compare::try_compare`], the same policy
/// as `clojure.core/compare`.  Adding an element that cannot be compared with
/// the elements on its search path is an error ([`SortedSet::try_conj`]);
/// membership tests and removals of such an element simply find nothing.
///
/// A set derived by `conj` or `disj` from a GC-allocated set reports only its
/// path-copied nodes from `gc_size_extra`; see `collections::share_charge`
/// for the full rule.
#[derive(Debug, Clone)]
pub struct SortedSet {
    inner: rpds::RedBlackTreeSetSync<SortKey>,
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

    /// Membership test; `false` also when `value` is not comparable with the
    /// elements.
    pub fn contains(&self, value: &Value) -> bool {
        checked(|| self.inner.contains(SortKey::from_ref(value))).unwrap_or(false)
    }

    /// Return a new set with `value` added, or an error if `value` cannot be
    /// compared with the existing elements (`self` is unchanged).
    pub fn try_conj(&self, value: Value) -> ValueResult<Self> {
        let inner = checked(|| self.inner.insert(SortKey(value)))?;
        Ok(self.derived(inner))
    }

    /// Infallible [`SortedSet::try_conj`] for elements known to be
    /// comparable; an incomparable element leaves the set unchanged.
    pub fn conj(&self, value: Value) -> Self {
        self.try_conj(value).unwrap_or_else(|_| self.clone())
    }

    /// In-place [`SortedSet::try_conj`]; on error `self` is unchanged.
    pub fn try_conj_mut(&mut self, value: Value) -> ValueResult<&mut Self> {
        let mut inner = self.inner.clone();
        checked(|| inner.insert_mut(SortKey(value)))?;
        self.inner = inner;
        self.charge = ShareCharge::full();
        Ok(self)
    }

    /// Infallible [`SortedSet::try_conj_mut`]; an incomparable element leaves
    /// the set unchanged.
    pub fn conj_mut(&mut self, value: Value) -> &mut Self {
        let _ = self.try_conj_mut(value);
        self
    }

    /// Return a new set with `value` removed.
    pub fn disj(&self, value: &Value) -> Self {
        match checked(|| self.inner.remove(SortKey::from_ref(value))) {
            Ok(inner) => self.derived(inner),
            Err(_) => self.clone(),
        }
    }

    /// Wrap `inner`, produced by one path-copying update of `self.inner`.
    fn derived(&self, inner: rpds::RedBlackTreeSetSync<SortKey>) -> Self {
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

    /// What `gc_size_extra` would report, without marking this version
    /// allocated.
    #[cfg(test)]
    fn charged(&self) -> usize {
        self.charge.size_extra(self.full_size_extra())
    }

    /// Iterate over all elements in sorted order
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.inner.iter().map(|k| &k.0)
    }

    /// Build a set, failing on the first element that cannot be compared
    /// with the elements before it.
    pub fn try_from_iter<I: IntoIterator<Item = Value>>(iter: I) -> ValueResult<Self> {
        let mut inner = rpds::RedBlackTreeSetSync::new_sync();
        for v in iter {
            checked(|| inner.insert_mut(SortKey(v)))?;
        }
        Ok(SortedSet {
            inner,
            charge: ShareCharge::full(),
        })
    }
}

/// Infallible [`SortedSet::try_from_iter`] for elements known to be
/// comparable; incomparable elements are skipped.
impl FromIterator<Value> for SortedSet {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut s = SortedSet::empty();
        for v in iter {
            s.conj_mut(v);
        }
        s
    }
}

impl PartialEq for SortedSet {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        // Same comparator on both sides, so equal sets iterate in step.
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljrs_gc::Trace for SortedSet {
    #[cfg(feature = "no-gc")]
    fn immutable_children(&self) -> bool {
        true
    }
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for v in self.inner.iter() {
            v.0.trace(visitor);
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
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let s = GcPtr::new(SortedSet::from_iter((0..10_000).map(int)));
        let added = s.get().conj(int(-1));
        assert_eq!(added.charged(), rb_path_bytes::<Value>(10_001));
        assert!(added.charged() * 100 < s.get().full_size_extra());
    }
}
