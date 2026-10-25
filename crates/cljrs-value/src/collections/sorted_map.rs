use crate::Value;
use crate::collections::share_charge::{ShareCharge, rb_path_bytes};

/// An immutable sorted map backed by `rpds::RedBlackTreeMapSync`.
///
/// A map derived by `assoc` or `dissoc` from a GC-allocated map reports only
/// its path-copied nodes from `gc_size_extra`; see
/// `collections::share_charge` for the full rule.
#[derive(Debug, Clone)]
pub struct SortedMap {
    inner: rpds::RedBlackTreeMapSync<Value, Value>,
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

    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.inner.get(key)
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.inner.contains_key(key)
    }

    pub fn assoc(&self, key: Value, value: Value) -> Self {
        self.derived(self.inner.insert(key, value))
    }

    pub fn dissoc(&self, key: &Value) -> Self {
        self.derived(self.inner.remove(key))
    }

    /// Wrap `inner`, produced by one path-copying update of `self.inner`.
    fn derived(&self, inner: rpds::RedBlackTreeMapSync<Value, Value>) -> Self {
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
        self.inner.iter()
    }

    pub fn keys(&self) -> Vec<Value> {
        self.inner.keys().cloned().collect()
    }

    pub fn vals(&self) -> Vec<Value> {
        self.inner.values().cloned().collect()
    }

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
        self.inner.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl cljrs_gc::Trace for SortedMap {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for (k, v) in self.inner.iter() {
            k.trace(visitor);
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
