use std::sync::Arc;

use crate::Value;

/// An immutable singly-linked list.  Structural sharing via `Arc`-backed tails.
///
/// Clojure's `PersistentList` is the primary `seq` type; it supports O(1) `cons`,
/// `first`, and `rest`.  `count` is cached on each node so it is also O(1).
#[derive(Debug, Clone)]
pub enum PersistentList {
    Empty,
    Cons {
        head: Value,
        tail: Arc<PersistentList>,
        count: usize,
    },
}

impl PersistentList {
    /// The canonical empty list.
    pub fn empty() -> Self {
        PersistentList::Empty
    }

    /// Prepend `head` to `tail`.  O(1).
    pub fn cons(head: Value, tail: Arc<PersistentList>) -> Self {
        let count = tail.count() + 1;
        PersistentList::Cons { head, tail, count }
    }

    /// Number of elements.  O(1).
    pub fn count(&self) -> usize {
        match self {
            PersistentList::Empty => 0,
            PersistentList::Cons { count, .. } => *count,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, PersistentList::Empty)
    }

    /// First element, or `None` if empty.
    pub fn first(&self) -> Option<&Value> {
        match self {
            PersistentList::Empty => None,
            PersistentList::Cons { head, .. } => Some(head),
        }
    }

    /// The rest of the list after the first element.  Returns an empty list
    /// if called on an empty or single-element list.
    pub fn rest(&self) -> Arc<PersistentList> {
        match self {
            PersistentList::Empty => Arc::new(PersistentList::Empty),
            PersistentList::Cons { tail, .. } => Arc::clone(tail),
        }
    }

    /// Iterate over elements from head to tail.
    pub fn iter(&self) -> ListIter<'_> {
        ListIter { current: self }
    }
}

impl cljx_gc::Trace for PersistentList {}

impl std::iter::FromIterator<Value> for PersistentList {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let items: Vec<Value> = iter.into_iter().collect();
        let mut list = PersistentList::Empty;
        for item in items.into_iter().rev() {
            list = PersistentList::cons(item, Arc::new(list));
        }
        list
    }
}

/// Iterator over `PersistentList`.
pub struct ListIter<'a> {
    current: &'a PersistentList,
}

impl<'a> Iterator for ListIter<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        match self.current {
            PersistentList::Empty => None,
            PersistentList::Cons { head, tail, .. } => {
                self.current = tail;
                Some(head)
            }
        }
    }
}

impl PartialEq for PersistentList {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
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
        let l = PersistentList::empty();
        assert!(l.is_empty());
        assert_eq!(l.count(), 0);
        assert!(l.first().is_none());
    }

    #[test]
    fn test_cons_and_first() {
        let l = PersistentList::cons(int(1), Arc::new(PersistentList::empty()));
        assert_eq!(l.count(), 1);
        assert_eq!(l.first(), Some(&int(1)));
    }

    #[test]
    fn test_from_iter() {
        let l = PersistentList::from_iter([int(1), int(2), int(3)]);
        assert_eq!(l.count(), 3);
        let items: Vec<_> = l.iter().cloned().collect();
        assert_eq!(items, vec![int(1), int(2), int(3)]);
    }

    #[test]
    fn test_rest() {
        let l = PersistentList::from_iter([int(1), int(2)]);
        let rest = l.rest();
        assert_eq!(rest.count(), 1);
        assert_eq!(rest.first(), Some(&int(2)));
    }

    #[test]
    fn test_equality() {
        let a = PersistentList::from_iter([int(1), int(2)]);
        let b = PersistentList::from_iter([int(1), int(2)]);
        let c = PersistentList::from_iter([int(1), int(3)]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_structural_sharing() {
        let tail = Arc::new(PersistentList::from_iter([int(2), int(3)]));
        let a = PersistentList::cons(int(1), Arc::clone(&tail));
        let b = PersistentList::cons(int(10), Arc::clone(&tail));
        // Both lists share the same tail allocation.
        assert!(Arc::ptr_eq(&a.rest(), &b.rest()));
    }
}
