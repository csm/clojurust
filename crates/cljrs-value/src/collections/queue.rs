use std::sync::Arc;

use crate::Value;
use crate::collections::list::PersistentList;
use crate::collections::vector::PersistentVector;

/// An immutable FIFO queue.
///
/// Uses a front list (for dequeue) and a rear vector (for enqueue).
/// Amortized O(1) for both operations.
#[derive(Debug, Clone)]
pub struct PersistentQueue {
    front: Arc<PersistentList>,
    rear: PersistentVector,
    count: usize,
}

impl PersistentQueue {
    pub fn empty() -> Self {
        Self {
            front: Arc::new(PersistentList::empty()),
            rear: PersistentVector::empty(),
            count: 0,
        }
    }
    
    pub fn new(front: PersistentList, rear: PersistentVector) -> Self {
        let count = &front.count() + &rear.count();
        Self {
            front: Arc::new(front.clone()),
            rear: rear.clone(),
            count,
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Add an element to the back of the queue.
    pub fn conj(&self, val: Value) -> Self {
        Self {
            front: Arc::clone(&self.front),
            rear: self.rear.conj(val),
            count: self.count + 1,
        }
    }

    /// View the front element without removing it.
    pub fn peek(&self) -> Option<&Value> {
        if self.front.is_empty() {
            self.rear.nth(0)
        } else {
            self.front.first()
        }
    }

    /// Remove the front element, returning the new queue.
    pub fn pop(&self) -> Option<Self> {
        if self.count == 0 {
            return None;
        }
        if !self.front.is_empty() {
            let new_front = Arc::new((*self.front.rest()).clone());
            // If front is now empty, move rear to front.
            if new_front.is_empty() && !self.rear.is_empty() {
                let new_front = Arc::new(PersistentList::from_iter(self.rear.iter().cloned()));
                return Some(Self {
                    front: new_front,
                    rear: PersistentVector::empty(),
                    count: self.count - 1,
                });
            }
            return Some(Self {
                front: new_front,
                rear: self.rear.clone(),
                count: self.count - 1,
            });
        }
        // Front is empty — rear has the items.
        let new_front = Arc::new(PersistentList::from_iter(self.rear.iter().cloned()));
        let new_front_rest = new_front.rest();
        Some(Self {
            front: new_front_rest,
            rear: PersistentVector::empty(),
            count: self.count - 1,
        })
    }

    /// Iterate over elements from front to back.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.front.iter().chain(self.rear.iter())
    }
}

impl PartialEq for PersistentQueue {
    fn eq(&self, other: &Self) -> bool {
        if self.count != other.count {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljrs_gc::Trace for PersistentQueue {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        // front: Arc<PersistentList> — trace through to find embedded GcPtrs
        self.front.trace(visitor);
        // rear: PersistentVector — trace normally
        self.rear.trace(visitor);
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
    fn test_basic_enqueue_dequeue() {
        let q = PersistentQueue::empty()
            .conj(int(1))
            .conj(int(2))
            .conj(int(3));
        assert_eq!(q.count(), 3);
        assert_eq!(q.peek(), Some(&int(1)));

        let q = q.pop().unwrap();
        assert_eq!(q.peek(), Some(&int(2)));
        assert_eq!(q.count(), 2);

        let q = q.pop().unwrap();
        assert_eq!(q.peek(), Some(&int(3)));

        let q = q.pop().unwrap();
        assert!(q.is_empty());
        assert!(q.pop().is_none());
    }

    #[test]
    fn test_fifo_order() {
        let q = PersistentQueue::empty()
            .conj(int(10))
            .conj(int(20))
            .conj(int(30));
        let items: Vec<_> = q.iter().cloned().collect();
        assert_eq!(items, vec![int(10), int(20), int(30)]);
    }
}
