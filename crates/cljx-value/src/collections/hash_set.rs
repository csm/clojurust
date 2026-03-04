use crate::Value;
use crate::collections::hash_map::PersistentHashMap;

/// An immutable hash set backed by a `PersistentHashMap` (keys map to themselves).
#[derive(Debug, Clone)]
pub struct PersistentHashSet {
    map: PersistentHashMap,
}

impl PersistentHashSet {
    pub fn empty() -> Self {
        Self {
            map: PersistentHashMap::empty(),
        }
    }

    pub fn count(&self) -> usize {
        self.map.count()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn contains(&self, val: &Value) -> bool {
        self.map.contains_key(val)
    }

    /// Return a new set with `val` added.
    pub fn conj(&self, val: Value) -> Self {
        Self {
            map: self.map.assoc(val.clone(), val),
        }
    }

    /// Return a new set with `val` removed.
    pub fn disj(&self, val: &Value) -> Self {
        Self {
            map: self.map.dissoc(val),
        }
    }

    /// Iterate over all elements in an unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = Value> {
        self.map.keys().into_iter()
    }
}

impl std::iter::FromIterator<Value> for PersistentHashSet {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut s = Self::empty();
        for v in iter {
            s = s.conj(v);
        }
        s
    }
}

impl PartialEq for PersistentHashSet {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        self.map.keys().iter().all(|k| other.contains(k))
        // keys() returns Vec<Value>, so .iter() gives &Value refs
    }
}

impl cljx_gc::Trace for PersistentHashSet {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        self.map.trace(visitor);
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
        let s = PersistentHashSet::empty();
        let s = s.conj(int(1)).conj(int(2)).conj(int(3));
        assert_eq!(s.count(), 3);
        assert!(s.contains(&int(1)));
        assert!(s.contains(&int(2)));
        assert!(!s.contains(&int(99)));
    }

    #[test]
    fn test_idempotent_conj() {
        let s = PersistentHashSet::empty().conj(int(1)).conj(int(1));
        assert_eq!(s.count(), 1);
    }

    #[test]
    fn test_disj() {
        let s = PersistentHashSet::empty().conj(int(1)).conj(int(2));
        let s2 = s.disj(&int(1));
        assert!(!s2.contains(&int(1)));
        assert!(s2.contains(&int(2)));
        assert_eq!(s2.count(), 1);
    }

    #[test]
    fn test_equality_order_independent() {
        let a = PersistentHashSet::from_iter([int(1), int(2), int(3)]);
        let b = PersistentHashSet::from_iter([int(3), int(1), int(2)]);
        assert_eq!(a, b);
    }
}
