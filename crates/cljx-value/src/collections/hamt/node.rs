use std::sync::Arc;

use cljx_gc::{GcPtr, Trace};

use super::bitmap::{BITS, bit_for, fragment, sparse_index};

/// Maximum trie depth (hash is u32 = 32 bits, 5 bits per level → 7 levels).
pub const MAX_DEPTH: u32 = 6; // 0..=6 gives 7 levels × 5 bits = 35 ≥ 32

/// A node in a 32-way HAMT trie.
///
/// `V` is either a `KVPair` (for hash maps) or a `Value` (for vector slots).
/// All nodes are immutable; structural sharing is via `GcPtr` (Arc shim).
#[derive(Debug, Clone)]
pub enum Node<V: Clone + std::fmt::Debug + Trace + 'static> {
    /// A single value stored at a leaf.
    Leaf { hash: u32, value: V },
    /// A sparse internal node: up to 32 children, stored densely.
    Branch {
        bitmap: u32,
        children: Arc<Vec<GcPtr<Node<V>>>>,
    },
    /// A hash-collision bucket: multiple values sharing the same 32-bit hash.
    Collision { hash: u32, entries: Arc<Vec<V>> },
}

impl<V: Clone + std::fmt::Debug + Trace + 'static> Trace for Node<V> {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        use cljx_gc::GcVisitor as _;
        match self {
            Node::Leaf { value, .. } => value.trace(visitor),
            Node::Collision { entries, .. } => {
                for v in entries.iter() {
                    v.trace(visitor);
                }
            }
            Node::Branch { children, .. } => {
                for child in children.iter() {
                    visitor.visit(child);
                }
            }
        }
    }
}

impl<V: Clone + std::fmt::Debug + Trace + 'static> Node<V> {
    // ── Lookup ────────────────────────────────────────────────────────────────

    /// Find a value by hash + equality predicate.  Returns `None` if absent.
    pub fn get<F>(&self, hash: u32, shift: u32, eq: &F) -> Option<&V>
    where
        F: Fn(&V) -> bool,
    {
        match self {
            Node::Leaf { hash: h, value } => {
                if *h == hash && eq(value) {
                    Some(value)
                } else {
                    None
                }
            }
            Node::Collision { hash: h, entries } => {
                if *h != hash {
                    return None;
                }
                entries.iter().find(|v| eq(v))
            }
            Node::Branch { bitmap, children } => {
                let frag = fragment(hash, shift);
                let bit = bit_for(frag);
                if bitmap & bit == 0 {
                    return None;
                }
                let idx = sparse_index(*bitmap, bit);
                children[idx].get().get(hash, shift + BITS, eq)
            }
        }
    }

    // ── Insert / update ───────────────────────────────────────────────────────

    /// Return a new node with `value` inserted/updated.
    /// `eq` identifies the existing entry to replace (same key).
    pub fn assoc<F>(&self, hash: u32, shift: u32, value: V, eq: &F) -> GcPtr<Node<V>>
    where
        F: Fn(&V) -> bool,
        V: 'static,
    {
        match self {
            Node::Leaf {
                hash: h,
                value: existing,
            } => {
                if *h == hash {
                    if eq(existing) {
                        // Replace existing entry.
                        return GcPtr::new(Node::Leaf { hash, value });
                    }
                    // Same hash, different key → collision.
                    return GcPtr::new(Node::Collision {
                        hash,
                        entries: Arc::new(vec![existing.clone(), value]),
                    });
                }
                // Different hash → promote to a branch containing both.
                let new_leaf = GcPtr::new(Node::Leaf { hash, value });
                let old_leaf = GcPtr::new(self.clone());
                merge_leaves(old_leaf, *h, new_leaf, hash, shift)
            }
            Node::Collision { hash: h, entries } => {
                if *h == hash {
                    // Add or replace within the collision bucket.
                    let mut new_entries = entries.as_ref().clone();
                    if let Some(pos) = new_entries.iter().position(eq) {
                        new_entries[pos] = value;
                    } else {
                        new_entries.push(value);
                    }
                    return GcPtr::new(Node::Collision {
                        hash,
                        entries: Arc::new(new_entries),
                    });
                }
                // Different hash → demote collision to a branch.
                let collision = GcPtr::new(self.clone());
                let new_leaf = GcPtr::new(Node::Leaf { hash, value });
                merge_leaves(collision, *h, new_leaf, hash, shift)
            }
            Node::Branch { bitmap, children } => {
                let frag = fragment(hash, shift);
                let bit = bit_for(frag);
                let idx = sparse_index(*bitmap, bit);

                if bitmap & bit == 0 {
                    // Slot is empty → insert a new leaf here.
                    let new_leaf = GcPtr::new(Node::Leaf { hash, value });
                    let mut new_children = children.as_ref().clone();
                    new_children.insert(idx, new_leaf);
                    GcPtr::new(Node::Branch {
                        bitmap: bitmap | bit,
                        children: Arc::new(new_children),
                    })
                } else {
                    // Slot occupied → recurse into the child.
                    let updated = children[idx].get().assoc(hash, shift + BITS, value, eq);
                    let mut new_children = children.as_ref().clone();
                    new_children[idx] = updated;
                    GcPtr::new(Node::Branch {
                        bitmap: *bitmap,
                        children: Arc::new(new_children),
                    })
                }
            }
        }
    }

    // ── Remove ────────────────────────────────────────────────────────────────

    /// Return `None` if the entry was removed (and the node is now empty),
    /// or `Some(new_node)` otherwise.
    pub fn dissoc<F>(&self, hash: u32, shift: u32, eq: &F) -> Option<GcPtr<Node<V>>>
    where
        F: Fn(&V) -> bool,
        V: 'static,
    {
        match self {
            Node::Leaf { hash: h, value } => {
                if *h == hash && eq(value) {
                    None
                } else {
                    Some(GcPtr::new(self.clone()))
                }
            }
            Node::Collision { hash: h, entries } => {
                if *h != hash {
                    return Some(GcPtr::new(self.clone()));
                }
                let new_entries: Vec<V> = entries.iter().filter(|v| !eq(v)).cloned().collect();
                match new_entries.len() {
                    0 => None,
                    1 => Some(GcPtr::new(Node::Leaf {
                        hash,
                        value: new_entries.into_iter().next().unwrap(),
                    })),
                    _ => Some(GcPtr::new(Node::Collision {
                        hash,
                        entries: Arc::new(new_entries),
                    })),
                }
            }
            Node::Branch { bitmap, children } => {
                let frag = fragment(hash, shift);
                let bit = bit_for(frag);
                if bitmap & bit == 0 {
                    return Some(GcPtr::new(self.clone()));
                }
                let idx = sparse_index(*bitmap, bit);
                match children[idx].get().dissoc(hash, shift + BITS, eq) {
                    None => {
                        // Child was removed entirely.
                        let mut new_children = children.as_ref().clone();
                        new_children.remove(idx);
                        let new_bitmap = bitmap & !bit;
                        if new_children.len() == 1 && new_bitmap.count_ones() == 1 {
                            // Collapse single-child branch if the child is a leaf.
                            let only = new_children.remove(0);
                            match only.get() {
                                Node::Leaf { .. } | Node::Collision { .. } => {
                                    return Some(only);
                                }
                                Node::Branch { .. } => {
                                    // Don't collapse branches — keep structure.
                                    return Some(GcPtr::new(Node::Branch {
                                        bitmap: new_bitmap,
                                        children: Arc::new(vec![only]),
                                    }));
                                }
                            }
                        }
                        if new_children.is_empty() {
                            None
                        } else {
                            Some(GcPtr::new(Node::Branch {
                                bitmap: new_bitmap,
                                children: Arc::new(new_children),
                            }))
                        }
                    }
                    Some(updated) => {
                        let mut new_children = children.as_ref().clone();
                        new_children[idx] = updated;
                        Some(GcPtr::new(Node::Branch {
                            bitmap: *bitmap,
                            children: Arc::new(new_children),
                        }))
                    }
                }
            }
        }
    }

    // ── Iteration ─────────────────────────────────────────────────────────────

    /// Visit all leaf values in an unspecified order.
    pub fn for_each<F>(&self, f: &mut F)
    where
        F: FnMut(&V),
    {
        match self {
            Node::Leaf { value, .. } => f(value),
            Node::Collision { entries, .. } => {
                for v in entries.iter() {
                    f(v);
                }
            }
            Node::Branch { children, .. } => {
                for child in children.iter() {
                    child.get().for_each(f);
                }
            }
        }
    }

    /// Collect all leaf values into a `Vec` (for iteration support).
    pub fn collect_values(&self) -> Vec<V> {
        let mut out = Vec::new();
        self.for_each(&mut |v| out.push(v.clone()));
        out
    }
}

/// Promote two leaves with different hashes into a branch (or deeper branches).
fn merge_leaves<V: Clone + std::fmt::Debug + Trace + 'static>(
    left: GcPtr<Node<V>>,
    left_hash: u32,
    right: GcPtr<Node<V>>,
    right_hash: u32,
    shift: u32,
) -> GcPtr<Node<V>> {
    if shift > 32 {
        // Should be unreachable for well-formed hashes, but guard anyway.
        panic!("HAMT shift exceeded 32 bits — hash collision or implementation bug");
    }

    let left_frag = fragment(left_hash, shift);
    let right_frag = fragment(right_hash, shift);

    if left_frag == right_frag {
        // Same fragment at this level → recurse deeper.
        let merged = merge_leaves(left, left_hash, right, right_hash, shift + BITS);
        let bit = bit_for(left_frag);
        GcPtr::new(Node::Branch {
            bitmap: bit,
            children: Arc::new(vec![merged]),
        })
    } else {
        let (first, second, first_bit, second_bit) = if left_frag < right_frag {
            (left, right, bit_for(left_frag), bit_for(right_frag))
        } else {
            (right, left, bit_for(right_frag), bit_for(left_frag))
        };
        GcPtr::new(Node::Branch {
            bitmap: first_bit | second_bit,
            children: Arc::new(vec![first, second]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Simple test entry: (hash, key_id, val)
    #[derive(Debug, Clone, PartialEq)]
    struct Entry(u32, u32, i32); // (hash, key, value)
    impl cljx_gc::Trace for Entry {
        fn trace(&self, _: &mut cljx_gc::MarkVisitor) {}
    }

    fn key_eq(key: u32) -> impl Fn(&Entry) -> bool {
        move |e: &Entry| e.1 == key
    }

    fn leaf(hash: u32, key: u32, val: i32) -> GcPtr<Node<Entry>> {
        GcPtr::new(Node::Leaf {
            hash,
            value: Entry(hash, key, val),
        })
    }

    #[test]
    fn test_insert_and_get() {
        let n = leaf(1, 1, 10);
        let n = n.get().assoc(2, 0, Entry(2, 2, 20), &key_eq(2));
        assert_eq!(n.get().get(1, 0, &key_eq(1)).map(|e| e.2), Some(10));
        assert_eq!(n.get().get(2, 0, &key_eq(2)).map(|e| e.2), Some(20));
        assert_eq!(n.get().get(3, 0, &key_eq(3)), None);
    }

    #[test]
    fn test_update() {
        let n = leaf(1, 1, 10);
        let n = n.get().assoc(1, 0, Entry(1, 1, 99), &key_eq(1));
        assert_eq!(n.get().get(1, 0, &key_eq(1)).map(|e| e.2), Some(99));
    }

    #[test]
    fn test_remove() {
        let n = leaf(1, 1, 10);
        let n = n.get().assoc(2, 0, Entry(2, 2, 20), &key_eq(2));
        let n = n.get().dissoc(1, 0, &key_eq(1)).unwrap();
        assert_eq!(n.get().get(1, 0, &key_eq(1)), None);
        assert_eq!(n.get().get(2, 0, &key_eq(2)).map(|e| e.2), Some(20));
    }

    #[test]
    fn test_collision() {
        // Two different keys with the same hash.
        let n = leaf(42, 1, 10);
        let n = n.get().assoc(42, 0, Entry(42, 2, 20), &key_eq(2));
        assert!(matches!(n.get(), Node::Collision { .. }));
        assert_eq!(n.get().get(42, 0, &key_eq(1)).map(|e| e.2), Some(10));
        assert_eq!(n.get().get(42, 0, &key_eq(2)).map(|e| e.2), Some(20));
    }

    #[test]
    fn test_many_inserts() {
        let mut root: GcPtr<Node<Entry>> = GcPtr::new(Node::Leaf {
            hash: 0,
            value: Entry(0, 0, 0),
        });
        for i in 1u32..=100 {
            root = root.get().assoc(i, 0, Entry(i, i, i as i32), &key_eq(i));
        }
        for i in 1u32..=100 {
            assert_eq!(
                root.get().get(i, 0, &key_eq(i)).map(|e| e.2),
                Some(i as i32)
            );
        }
    }
}
