use crate::Value;

/// An immutable sorted map backed by `rpds::RedBlackTreeMapSync`.
#[derive(Debug, Clone)]
pub struct SortedMap {
    inner: rpds::RedBlackTreeMapSync<Value, Value>,
}

impl SortedMap {
    pub fn empty() -> Self {
        Self {
            inner: rpds::RedBlackTreeMapSync::new_sync(),
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
        Self {
            inner: self.inner.insert(key, value),
        }
    }

    pub fn dissoc(&self, key: &Value) -> Self {
        Self {
            inner: self.inner.remove(key),
        }
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

impl cljx_gc::Trace for SortedMap {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        for (k, v) in self.inner.iter() {
            k.trace(visitor);
            v.trace(visitor);
        }
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
}
