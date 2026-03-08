use crate::Value;

/// An immutable sorted set backed by `rpds::RedBlackTreeSet`.
#[derive(Debug, Clone)]
pub struct SortedSet {
    inner: rpds::RedBlackTreeSetSync<Value>,
}

impl SortedSet {
    pub fn empty() -> Self {
        Self {
            inner: rpds::RedBlackTreeSetSync::new_sync(),
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
        Self {
            inner: self.inner.insert(value),
        }
    }

    pub fn conj_mut(&mut self, value: Value) -> &mut Self {
        self.inner.insert_mut(value);
        self
    }

    /// Return a new set with `val` removed.
    pub fn disj(&self, value: &Value) -> Self {
        Self {
            inner: self.inner.remove(value),
        }
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
        SortedSet { inner }
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

impl cljx_gc::Trace for SortedSet {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        for v in self.inner.iter() {
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
}
