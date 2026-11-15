use crate::Value;
use crate::collections::share_charge::{ShareCharge, rb_path_bytes};
use crate::compare::{SortKey, checked};
use crate::error::ValueResult;

/// An immutable sorted map backed by `rpds::RedBlackTreeMapSync`.
///
/// Keys are ordered by [`crate::compare::try_compare`], the same policy as
/// `clojure.core/compare`.  Inserting a key that cannot be compared with the
/// keys on its search path is an error ([`SortedMap::try_assoc`]); lookups
/// and removals of such a key simply find nothing.
///
/// A map derived by `assoc` or `dissoc` from a GC-allocated map reports only
/// its path-copied nodes from `gc_size_extra`; see
/// `collections::share_charge` for the full rule.
#[derive(Debug, Clone)]
pub struct SortedMap {
    inner: rpds::RedBlackTreeMapSync<SortKey, Value>,
    charge: ShareCharge,
}

impl SortedMap {
    pub fn empty() -> Self {
        Self {
            inner: rpds::RedBlackTreeMapSync::new_sync(),
            charge: ShareCharge::full(),
        }
    }

    pub fn count(&self) -> usize {
        self.inner.size()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up `key`; `None` also when `key` is not comparable with the keys.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        checked(|| self.inner.get(SortKey::from_ref(key)))
            .ok()
            .flatten()
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.get(key).is_some()
    }

    /// Return a new map with `key` bound to `value`, or an error if `key`
    /// cannot be compared with the existing keys (`self` is unchanged).
    pub fn try_assoc(&self, key: Value, value: Value) -> ValueResult<Self> {
        let inner = checked(|| self.inner.insert(SortKey(key), value))?;
        Ok(self.derived(inner))
    }

    /// Infallible [`SortedMap::try_assoc`] for keys known to be comparable;
    /// an incomparable key leaves the map unchanged.
    pub fn assoc(&self, key: Value, value: Value) -> Self {
        self.try_assoc(key, value).unwrap_or_else(|_| self.clone())
    }

    /// Return a new map without `key`; a key that is not comparable with the
    /// existing keys is not present, so the result equals `self`.
    pub fn dissoc(&self, key: &Value) -> Self {
        match checked(|| self.inner.remove(SortKey::from_ref(key))) {
            Ok(inner) => self.derived(inner),
            Err(_) => self.clone(),
        }
    }

    /// Wrap `inner`, produced by one path-copying update of `self.inner`.
    fn derived(&self, inner: rpds::RedBlackTreeMapSync<SortKey, Value>) -> Self {
        let n = inner.size().max(self.inner.size());
        Self {
            charge: self.charge.derive(
                self.inner.size(),
                inner.size(),
                rb_path_bytes::<(Value, Value)>(n),
            ),
            inner,
        }
    }

    /// Estimated bytes owned by this map if it shared no nodes.
    fn full_size_extra(&self) -> usize {
        // Per entry: Arc<Node<K,V>> (16 overhead) + left/right ptrs (16) + color+pad (8).
        self.inner.size() * (40 + 2 * std::mem::size_of::<Value>())
    }

    /// What `gc_size_extra` would report, without marking this version
    /// allocated.
    #[cfg(test)]
    fn charged(&self) -> usize {
        self.charge.size_extra(self.full_size_extra())
    }

    /// Iterate over all `(key, value)` pairs in sorted key order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.inner.iter().map(|(k, v)| (&k.0, v))
    }

    pub fn keys(&self) -> Vec<Value> {
        self.inner.keys().map(|k| k.0.clone()).collect()
    }

    pub fn vals(&self) -> Vec<Value> {
        self.inner.values().cloned().collect()
    }

    /// Build a map from pairs (last binding wins), failing on the first key
    /// that cannot be compared with the keys before it.
    pub fn try_from_pairs<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> ValueResult<Self> {
        let mut inner = rpds::RedBlackTreeMapSync::new_sync();
        for (k, v) in iter {
            checked(|| inner.insert_mut(SortKey(k), v))?;
        }
        Ok(Self {
            inner,
            charge: ShareCharge::full(),
        })
    }

    /// Infallible [`SortedMap::try_from_pairs`] for keys known to be
    /// comparable; incomparable keys are skipped.
    pub fn from_pairs<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> Self {
        let mut m = Self::empty();
        for (k, v) in iter {
            m = m.assoc(k, v);
        }
        m
    }
}

impl PartialEq for SortedMap {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        // Same comparator on both sides, so equal maps iterate in step.
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljrs_gc::Trace for SortedMap {
    #[cfg(feature = "no-gc")]
    fn immutable_children(&self) -> bool {
        true
    }
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for (k, v) in self.inner.iter() {
            k.0.trace(visitor);
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
    fn test_basic_ops() {
        let m = SortedMap::empty();
        let m = m.assoc(int(3), int(30));
        let m = m.assoc(int(1), int(10));
        let m = m.assoc(int(2), int(20));
        assert_eq!(m.count(), 3);
        assert_eq!(m.get(&int(1)), Some(&int(10)));
        assert_eq!(m.get(&int(2)), Some(&int(20)));
        assert_eq!(m.get(&int(3)), Some(&int(30)));
    }

    #[test]
    fn test_sorted_iteration_order() {
        let m = SortedMap::empty()
            .assoc(int(3), int(30))
            .assoc(int(1), int(10))
            .assoc(int(2), int(20));
        let keys: Vec<Value> = m.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys, vec![int(1), int(2), int(3)]);
    }

    #[test]
    fn test_dissoc() {
        let m = SortedMap::empty()
            .assoc(int(1), int(10))
            .assoc(int(2), int(20));
        let m2 = m.dissoc(&int(1));
        assert_eq!(m2.count(), 1);
        assert_eq!(m2.get(&int(1)), None);
        assert_eq!(m2.get(&int(2)), Some(&int(20)));
    }

    #[test]
    fn assoc_on_allocated_map_charges_path_bytes() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let m = GcPtr::new(SortedMap::from_pairs((0..10_000).map(|i| (int(i), int(i)))));
        let added = m.get().assoc(int(-1), int(0));
        assert_eq!(added.charged(), rb_path_bytes::<(Value, Value)>(10_001));
        assert!(added.charged() * 100 < m.get().full_size_extra());
    }
}
