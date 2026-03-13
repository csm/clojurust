use rpds::HashTrieMapSync;
use crate::Value;
use crate::collections::array_map::PersistentArrayMap;

/// An immutable hash map backed by `rpds::HashTrieMap`.
///
/// Small maps (≤8 entries) are represented as `PersistentArrayMap` instead;
/// the two types share the same `Value::Map` variant.  `PersistentHashMap` is
/// used once the entry count exceeds the array-map threshold.
#[derive(Debug, Clone)]
pub struct PersistentHashMap {
    inner: rpds::HashTrieMapSync<Value, Value>,
}

impl PersistentHashMap {
    pub fn empty() -> Self {
        Self {
            inner: rpds::HashTrieMapSync::new_sync(),
        }
    }

    pub fn new(map: HashTrieMapSync<Value, Value>) -> Self {
        Self { inner: map }
    }

    pub fn count(&self) -> usize {
        self.inner.size()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Look up a key.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        self.inner.get(key)
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.inner.contains_key(key)
    }

    /// Return a new map with `key` → `value`.
    pub fn assoc(&self, key: Value, value: Value) -> Self {
        Self {
            inner: self.inner.insert(key, value),
        }
    }

    /// Return a new map with `key` removed.
    pub fn dissoc(&self, key: &Value) -> Self {
        Self {
            inner: self.inner.remove(key),
        }
    }

    /// Iterate over all `(key, value)` pairs in an unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.inner.iter()
    }

    /// Collect all keys.
    pub fn keys(&self) -> Vec<Value> {
        self.inner.keys().cloned().collect()
    }

    /// Collect all values.
    pub fn vals(&self) -> Vec<Value> {
        self.inner.values().cloned().collect()
    }

    /// Merge two maps; right-hand side wins on key collision.
    pub fn merge(&self, other: &Self) -> Self {
        let mut result = self.clone();
        for (k, v) in other.inner.iter() {
            result = result.assoc(k.clone(), v.clone());
        }
        result
    }

    /// Build from an iterator of `(key, value)` pairs.
    pub fn from_pairs<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> Self {
        let mut m = Self::empty();
        for (k, v) in iter {
            m = m.assoc(k, v);
        }
        m
    }

    /// Promote from a `PersistentArrayMap` when the threshold is exceeded.
    pub fn from_array_map(am: &PersistentArrayMap) -> Self {
        Self::from_pairs(am.iter().map(|(k, v)| (k.clone(), v.clone())))
    }
}

impl PartialEq for PersistentHashMap {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        self.inner.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl cljx_gc::Trace for PersistentHashMap {
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
    use crate::collections::array_map::AssocResult;
    use cljx_gc::GcPtr;

    fn kw(s: &str) -> Value {
        Value::Keyword(GcPtr::new(crate::keyword::Keyword::simple(s)))
    }
    fn int(n: i64) -> Value {
        Value::Long(n)
    }

    #[test]
    fn test_basic_ops() {
        let m = PersistentHashMap::empty();
        let m = m.assoc(kw("a"), int(1));
        let m = m.assoc(kw("b"), int(2));
        assert_eq!(m.count(), 2);
        assert_eq!(m.get(&kw("a")), Some(&int(1)));
        assert_eq!(m.get(&kw("b")), Some(&int(2)));
        assert_eq!(m.get(&kw("c")), None);
    }

    #[test]
    fn test_update() {
        let m = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("a"), int(99));
        assert_eq!(m.count(), 1);
        assert_eq!(m.get(&kw("a")), Some(&int(99)));
    }

    #[test]
    fn test_dissoc() {
        let m = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("b"), int(2));
        let m2 = m.dissoc(&kw("a"));
        assert_eq!(m2.count(), 1);
        assert_eq!(m2.get(&kw("a")), None);
        assert_eq!(m2.get(&kw("b")), Some(&int(2)));
    }

    #[test]
    fn test_many_entries() {
        let mut m = PersistentHashMap::empty();
        for i in 0i64..200 {
            m = m.assoc(int(i), int(i * 10));
        }
        assert_eq!(m.count(), 200);
        for i in 0i64..200 {
            assert_eq!(m.get(&int(i)), Some(&int(i * 10)));
        }
    }

    #[test]
    fn test_merge() {
        let a = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("b"), int(2));
        let b = PersistentHashMap::empty()
            .assoc(kw("b"), int(99))
            .assoc(kw("c"), int(3));
        let merged = a.merge(&b);
        assert_eq!(merged.count(), 3);
        assert_eq!(merged.get(&kw("a")), Some(&int(1)));
        assert_eq!(merged.get(&kw("b")), Some(&int(99))); // right wins
        assert_eq!(merged.get(&kw("c")), Some(&int(3)));
    }

    #[test]
    fn test_equality() {
        let a = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("b"), int(2));
        let b = PersistentHashMap::empty()
            .assoc(kw("b"), int(2))
            .assoc(kw("a"), int(1));
        assert_eq!(a, b);
    }

    #[test]
    fn test_from_array_map() {
        let mut am = PersistentArrayMap::empty();
        for i in 0..3i64 {
            let AssocResult::Array(next) = am.assoc(int(i), int(i * 2)) else {
                panic!()
            };
            am = next;
        }
        let hm = PersistentHashMap::from_array_map(&am);
        assert_eq!(hm.count(), 3);
        for i in 0..3i64 {
            assert_eq!(hm.get(&int(i)), Some(&int(i * 2)));
        }
    }
}
