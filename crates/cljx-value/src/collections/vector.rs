use crate::Value;

/// An immutable persistent vector backed by `rpds::Vector`.
#[derive(Debug, Clone)]
pub struct PersistentVector {
    inner: rpds::VectorSync<Value>,
}

impl PersistentVector {
    pub fn empty() -> Self {
        Self {
            inner: rpds::VectorSync::new_sync(),
        }
    }

    pub fn count(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Append a value. O(log n) amortized.
    pub fn conj(&self, val: Value) -> Self {
        Self {
            inner: self.inner.push_back(val),
        }
    }

    /// Return the element at `idx`, or `None` if out of bounds.
    pub fn nth(&self, idx: usize) -> Option<&Value> {
        self.inner.get(idx)
    }

    /// Last element.
    pub fn peek(&self) -> Option<&Value> {
        self.inner.last()
    }

    /// Return a new vector with element `idx` replaced.
    pub fn assoc_nth(&self, idx: usize, val: Value) -> Option<Self> {
        Some(Self {
            inner: self.inner.set(idx, val)?,
        })
    }

    /// Remove the last element. Returns `None` if empty.
    pub fn pop(&self) -> Option<Self> {
        Some(Self {
            inner: self.inner.drop_last()?,
        })
    }

    /// Iterate over elements in index order.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.inner.iter()
    }
}

impl std::iter::FromIterator<Value> for PersistentVector {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut v = rpds::VectorSync::new_sync();
        for item in iter {
            v = v.push_back(item);
        }
        Self { inner: v }
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

impl cljx_gc::Trace for PersistentVector {
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
    fn test_iter_order() {
        let v = PersistentVector::from_iter((0..10).map(|i| int(i as i64)));
        let items: Vec<_> = v.iter().cloned().collect();
        assert_eq!(items, (0..10).map(|i| int(i as i64)).collect::<Vec<_>>());
    }
}
