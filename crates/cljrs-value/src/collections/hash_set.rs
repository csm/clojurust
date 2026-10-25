use crate::Value;
use crate::collections::share_charge::{ShareCharge, hamt_path_bytes, rb_path_bytes};
use rpds::{HashTrieMapSync, RedBlackTreeMapSync};

/// An immutable hash set that preserves insertion order when iterated.
///
/// Uses the same index-plus-ordered-log technique as `PersistentHashMap`:
/// a `rpds::HashTrieMap` from value to insertion sequence number, and a
/// `rpds::RedBlackTreeMap` from sequence number to value that is iterated in
/// order. This keeps iteration deterministic and matching insertion order,
/// rather than depending on hash-bucket layout — `rpds` seeds its hasher
/// randomly per instance, so raw hash-order iteration would otherwise vary
/// from run to run.
///
/// A set derived by `conj` or `disj` from a GC-allocated set reports only its
/// path-copied nodes from `gc_size_extra`; see `collections::share_charge`
/// for the full rule.
#[derive(Debug, Clone)]
pub struct PersistentHashSet {
    index: HashTrieMapSync<Value, u64>,
    order: RedBlackTreeMapSync<u64, Value>,
    next_seq: u64,
    charge: ShareCharge,
}

impl PersistentHashSet {
    pub fn empty() -> Self {
        Self {
            index: HashTrieMapSync::new_sync(),
            order: RedBlackTreeMapSync::new_sync(),
            next_seq: 0,
            charge: ShareCharge::full(),
        }
    }

    pub fn count(&self) -> usize {
        self.index.size()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    pub fn contains(&self, val: &Value) -> bool {
        self.index.contains_key(val)
    }

    /// Return a new set with `val` added.
    ///
    /// If `val` is already present, its original insertion position is kept.
    pub fn conj(&self, val: Value) -> Self {
        if self.index.contains_key(&val) {
            return self.unchanged();
        }
        let seq = self.next_seq;
        let n = self.count();
        Self {
            index: self.index.insert(val.clone(), seq),
            order: self.order.insert(seq, val),
            next_seq: seq + 1,
            charge: self.charge.derive(n, n + 1, Self::path_bytes(n + 1)),
        }
    }

    pub fn conj_mut(&mut self, val: Value) -> &mut Self {
        if !self.index.contains_key(&val) {
            let seq = self.next_seq;
            self.index.insert_mut(val.clone(), seq);
            self.order.insert_mut(seq, val);
            self.next_seq = seq + 1;
            self.charge = ShareCharge::full();
        }
        self
    }

    /// Return a new set with `val` removed.
    pub fn disj(&self, val: &Value) -> Self {
        match self.index.get(val) {
            Some(seq) => {
                let n = self.count();
                Self {
                    index: self.index.remove(val),
                    order: self.order.remove(seq),
                    next_seq: self.next_seq,
                    charge: self.charge.derive(n, n - 1, Self::path_bytes(n)),
                }
            }
            None => self.unchanged(),
        }
    }

    /// A copy returned by an update that changed nothing; it shares every node.
    fn unchanged(&self) -> Self {
        let n = self.count();
        Self {
            index: self.index.clone(),
            order: self.order.clone(),
            next_seq: self.next_seq,
            charge: self.charge.derive(n, n, 0),
        }
    }

    /// Bytes path-copied in both trees by inserting or removing one element.
    fn path_bytes(n: usize) -> usize {
        hamt_path_bytes::<(Value, u64)>(n) + rb_path_bytes::<(u64, Value)>(n)
    }

    /// Estimated bytes owned by this set if it shared no nodes.
    pub(crate) fn full_size_extra(&self) -> usize {
        // Per entry: index HashTrieMap entry (~40) + order RedBlackTree node
        // (~48), value stored inline in the order tree (not behind GcPtr).
        self.count() * (88 + std::mem::size_of::<Value>())
    }

    /// What `gc_size_extra` would report, without marking this version
    /// allocated.
    #[cfg(test)]
    fn charged(&self) -> usize {
        self.charge.size_extra(self.full_size_extra())
    }

    /// Iterate over all elements in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Value> {
        self.order.iter().map(|(_, v)| v)
    }
}

impl FromIterator<Value> for PersistentHashSet {
    fn from_iter<I: IntoIterator<Item = Value>>(iter: I) -> Self {
        let mut s = PersistentHashSet::empty();
        for v in iter {
            s.conj_mut(v);
        }
        s
    }
}

impl PartialEq for PersistentHashSet {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        self.iter().all(|k| other.contains(k))
    }
}

impl cljrs_gc::Trace for PersistentHashSet {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for (_, v) in self.order.iter() {
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

    #[test]
    fn test_iteration_order_matches_insertion_order() {
        let s = PersistentHashSet::empty()
            .conj(int(10))
            .conj(int(3))
            .conj(int(7))
            .conj(int(1));
        let iterated: Vec<Value> = s.iter().cloned().collect();
        assert_eq!(iterated, vec![int(10), int(3), int(7), int(1)]);
    }

    #[test]
    fn conj_on_allocated_set_charges_path_bytes() {
        let _alloc = crate::collections::share_charge::alloc_scope();
        use cljrs_gc::GcPtr;

        let s = GcPtr::new(PersistentHashSet::from_iter((0..10_000).map(int)));
        let added = s.get().conj(int(-1));
        assert_eq!(added.charged(), PersistentHashSet::path_bytes(10_001));
        assert!(added.charged() * 100 < s.get().full_size_extra());
        // A no-op conj shares every node.
        assert_eq!(s.get().conj(int(5)).charged(), 1);
        // A set built by `conj_mut` owns all of its nodes.
        let mut built = PersistentHashSet::empty();
        built.conj_mut(int(1));
        assert_eq!(built.charged(), built.full_size_extra());
    }
}
