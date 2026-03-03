use std::sync::Arc;

use cljx_gc::GcPtr;

use crate::Value;

const BITS: u32 = 5;
const WIDTH: usize = 1 << BITS; // 32

// ── VNode ─────────────────────────────────────────────────────────────────────

/// A node in the vector trie.
#[derive(Debug, Clone)]
enum VNode {
    /// Holds up to WIDTH values at the leaves of the trie.
    Leaf(Arc<Vec<Value>>),
    /// Holds up to WIDTH child nodes.
    Internal(Arc<Vec<GcPtr<VNode>>>),
}

impl cljx_gc::Trace for VNode {}

// ── PersistentVector ──────────────────────────────────────────────────────────

/// An immutable vector using a 32-way trie with a tail buffer.
///
/// Elements 0..tail_offset live in the trie; elements tail_offset..count
/// live in the tail.  The tail holds at most WIDTH elements, giving amortized
/// O(1) `conj`.  All other operations are O(log₃₂ n).
#[derive(Debug, Clone)]
pub struct PersistentVector {
    count: usize,
    /// Shift level of the root: BITS = 1-level trie, 2*BITS = 2-level, etc.
    shift: u32,
    /// Trie root; None when all elements fit in the tail (count ≤ WIDTH).
    root: Option<GcPtr<VNode>>,
    /// Tail buffer: 0..=WIDTH elements not yet flushed to the trie.
    tail: Arc<Vec<Value>>,
}

impl PersistentVector {
    pub fn empty() -> Self {
        Self {
            count: 0,
            shift: BITS,
            root: None,
            tail: Arc::new(Vec::new()),
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Number of elements in the trie (not in the tail).
    fn trie_size(&self) -> usize {
        self.count - self.tail.len()
    }

    /// Index of the first element in the tail.
    fn tail_offset(&self) -> usize {
        self.trie_size()
    }

    /// Append a value.  O(1) amortized.
    pub fn conj(&self, val: Value) -> Self {
        // Room in tail?
        if self.tail.len() < WIDTH {
            let mut new_tail = self.tail.as_ref().clone();
            new_tail.push(val);
            return Self {
                count: self.count + 1,
                shift: self.shift,
                root: self.root.clone(),
                tail: Arc::new(new_tail),
            };
        }

        // Tail is full — push it into the trie as a new leaf.
        let tail_leaf = GcPtr::new(VNode::Leaf(Arc::clone(&self.tail)));
        let trie_sz = self.trie_size(); // elements currently in trie

        let (new_root, new_shift) = match &self.root {
            None => {
                // First trie entry: wrap the leaf in an Internal node.
                (
                    GcPtr::new(VNode::Internal(Arc::new(vec![tail_leaf]))),
                    self.shift,
                )
            }
            Some(root) => {
                // Trie capacity at the current shift level.
                let capacity = 1usize << (self.shift + BITS);
                if trie_sz >= capacity {
                    // Grow by one level.
                    let new_shift = self.shift + BITS;
                    let new_root = GcPtr::new(VNode::Internal(Arc::new(vec![
                        root.clone(),
                        new_path(self.shift, tail_leaf),
                    ])));
                    (new_root, new_shift)
                } else {
                    (
                        push_leaf(root.clone(), self.shift, trie_sz, tail_leaf),
                        self.shift,
                    )
                }
            }
        };

        Self {
            count: self.count + 1,
            shift: new_shift,
            root: Some(new_root),
            tail: Arc::new(vec![val]),
        }
    }

    /// Return the element at `idx`, or `None` if out of bounds.  O(log₃₂ n).
    pub fn nth(&self, idx: usize) -> Option<&Value> {
        if idx >= self.count {
            return None;
        }
        if idx >= self.tail_offset() {
            return self.tail.get(idx - self.tail_offset());
        }
        let root = self.root.as_ref()?;
        get_in_node(root, self.shift, idx)
    }

    /// Last element.
    pub fn peek(&self) -> Option<&Value> {
        if self.count == 0 {
            None
        } else {
            self.nth(self.count - 1)
        }
    }

    /// Return a new vector with element `idx` replaced.  O(log₃₂ n).
    pub fn assoc_nth(&self, idx: usize, val: Value) -> Option<Self> {
        if idx >= self.count {
            return None;
        }
        if idx >= self.tail_offset() {
            let mut new_tail = self.tail.as_ref().clone();
            new_tail[idx - self.tail_offset()] = val;
            return Some(Self {
                count: self.count,
                shift: self.shift,
                root: self.root.clone(),
                tail: Arc::new(new_tail),
            });
        }
        let root = self.root.as_ref()?;
        Some(Self {
            count: self.count,
            shift: self.shift,
            root: Some(assoc_in_node(root.clone(), self.shift, idx, val)),
            tail: Arc::clone(&self.tail),
        })
    }

    /// Remove the last element.  Returns `None` if empty.
    pub fn pop(&self) -> Option<Self> {
        if self.count == 0 {
            return None;
        }
        if self.count == 1 {
            return Some(Self::empty());
        }
        if self.tail.len() > 1 {
            let mut new_tail = self.tail.as_ref().clone();
            new_tail.pop();
            return Some(Self {
                count: self.count - 1,
                shift: self.shift,
                root: self.root.clone(),
                tail: Arc::new(new_tail),
            });
        }
        // Tail has exactly one element — pull the rightmost leaf from the trie.
        let root = self.root.as_ref().unwrap();
        let (new_root_opt, new_tail_vals) = pop_leaf(root.clone(), self.shift);
        let (final_root, final_shift) = match new_root_opt {
            None => (None, BITS),
            Some(r) => {
                // Collapse a single-child root to reduce trie depth.
                if self.shift > BITS
                    && let VNode::Internal(ch) = r.get()
                    && ch.len() == 1
                {
                    return Some(Self {
                        count: self.count - 1,
                        shift: self.shift - BITS,
                        root: Some(ch[0].clone()),
                        tail: Arc::new(new_tail_vals),
                    });
                }
                (Some(r), self.shift)
            }
        };

        Some(Self {
            count: self.count - 1,
            shift: final_shift,
            root: final_root,
            tail: Arc::new(new_tail_vals),
        })
    }

    /// Iterate over elements in index order.
    pub fn iter(&self) -> VectorIter<'_> {
        VectorIter { vec: self, idx: 0 }
    }
}

impl std::iter::FromIterator<Value> for PersistentVector {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut v = Self::empty();
        for item in iter {
            v = v.conj(item);
        }
        v
    }
}

// ── Trie helpers ──────────────────────────────────────────────────────────────

/// Build a single-child path from level `shift` down to `leaf`.
///
/// At `shift == BITS` (the level directly above leaves), `leaf` is wrapped
/// in an `Internal` with one child.  At higher levels, more wrappers are added.
fn new_path(shift: u32, leaf: GcPtr<VNode>) -> GcPtr<VNode> {
    if shift == BITS {
        GcPtr::new(VNode::Internal(Arc::new(vec![leaf])))
    } else {
        GcPtr::new(VNode::Internal(Arc::new(vec![new_path(
            shift - BITS,
            leaf,
        )])))
    }
}

/// Insert `leaf` at the position for `trie_sz` elements already in the trie.
fn push_leaf(node: GcPtr<VNode>, shift: u32, trie_sz: usize, leaf: GcPtr<VNode>) -> GcPtr<VNode> {
    let VNode::Internal(children) = node.get() else {
        return node.clone();
    };
    let mut new_children = children.as_ref().clone();
    if shift == BITS {
        // Direct parent of leaf nodes — just append.
        new_children.push(leaf);
    } else {
        let child_idx = (trie_sz >> shift) & (WIDTH - 1);
        if child_idx < new_children.len() {
            let updated = push_leaf(new_children[child_idx].clone(), shift - BITS, trie_sz, leaf);
            new_children[child_idx] = updated;
        } else {
            new_children.push(new_path(shift - BITS, leaf));
        }
    }
    GcPtr::new(VNode::Internal(Arc::new(new_children)))
}

/// Traverse the trie to find the value at `idx`.
fn get_in_node(node: &GcPtr<VNode>, shift: u32, idx: usize) -> Option<&Value> {
    match node.get() {
        VNode::Leaf(vals) => vals.get(idx & (WIDTH - 1)),
        VNode::Internal(children) => {
            let child_idx = (idx >> shift) & (WIDTH - 1);
            children
                .get(child_idx)
                .and_then(|c| get_in_node(c, shift - BITS, idx))
        }
    }
}

/// Return a new node with the value at `idx` replaced.
fn assoc_in_node(node: GcPtr<VNode>, shift: u32, idx: usize, val: Value) -> GcPtr<VNode> {
    match node.get() {
        VNode::Leaf(vals) => {
            let mut new_vals = vals.as_ref().clone();
            new_vals[idx & (WIDTH - 1)] = val;
            GcPtr::new(VNode::Leaf(Arc::new(new_vals)))
        }
        VNode::Internal(children) => {
            let child_idx = (idx >> shift) & (WIDTH - 1);
            let mut new_children = children.as_ref().clone();
            new_children[child_idx] =
                assoc_in_node(new_children[child_idx].clone(), shift - BITS, idx, val);
            GcPtr::new(VNode::Internal(Arc::new(new_children)))
        }
    }
}

/// Remove the rightmost leaf from the trie; return the (possibly-pruned) trie
/// and the values that were in the removed leaf.
fn pop_leaf(node: GcPtr<VNode>, shift: u32) -> (Option<GcPtr<VNode>>, Vec<Value>) {
    let VNode::Internal(children) = node.get() else {
        return (None, vec![]);
    };
    if shift == BITS {
        // Last child is the rightmost leaf.
        let tail_vals = if let VNode::Leaf(vals) = children.last().unwrap().get() {
            vals.as_ref().clone()
        } else {
            vec![]
        };
        if children.len() == 1 {
            (None, tail_vals)
        } else {
            let mut new_children = children.as_ref().clone();
            new_children.pop();
            (
                Some(GcPtr::new(VNode::Internal(Arc::new(new_children)))),
                tail_vals,
            )
        }
    } else {
        let last_idx = children.len() - 1;
        let (updated, tail_vals) = pop_leaf(children[last_idx].clone(), shift - BITS);
        let mut new_children = children.as_ref().clone();
        match updated {
            None => {
                new_children.pop();
            }
            Some(c) => {
                new_children[last_idx] = c;
            }
        }
        if new_children.is_empty() {
            (None, tail_vals)
        } else {
            (
                Some(GcPtr::new(VNode::Internal(Arc::new(new_children)))),
                tail_vals,
            )
        }
    }
}

// ── Iterator ──────────────────────────────────────────────────────────────────

pub struct VectorIter<'a> {
    vec: &'a PersistentVector,
    idx: usize,
}

impl<'a> Iterator for VectorIter<'a> {
    type Item = &'a Value;

    fn next(&mut self) -> Option<Self::Item> {
        if self.idx >= self.vec.count() {
            return None;
        }
        let v = self.vec.nth(self.idx)?;
        self.idx += 1;
        Some(v)
    }
}

impl PartialEq for PersistentVector {
    fn eq(&self, other: &Self) -> bool {
        if self.count != other.count {
            return false;
        }
        self.iter().zip(other.iter()).all(|(a, b)| a == b)
    }
}

impl cljx_gc::Trace for PersistentVector {}

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
        // 33 elements: tail fills at 32 and is pushed to the trie.
        let v = PersistentVector::from_iter((0..33).map(int));
        assert_eq!(v.count(), 33);
        for i in 0..33 {
            assert_eq!(v.nth(i), Some(&int(i as i64)), "nth({i}) wrong");
        }
    }

    #[test]
    fn test_large() {
        let n = 1025; // 32 leaf pages fills one trie level
        let v = PersistentVector::from_iter((0..n).map(|i| int(i as i64)));
        assert_eq!(v.count(), n);
        for i in 0..n {
            assert_eq!(v.nth(i), Some(&int(i as i64)));
        }
    }

    #[test]
    fn test_two_level_trie() {
        // 1025 + 32 = 1057 forces a two-level trie.
        let n = 1057;
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
