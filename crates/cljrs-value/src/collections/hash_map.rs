use crate::Value;
use crate::collections::array_map::PersistentArrayMap;
use crate::collections::share_charge::{ShareCharge, hamt_path_bytes, rb_path_bytes};
use rpds::{HashTrieMapSync, RedBlackTreeMapSync};

/// An immutable hash map that preserves insertion order when iterated.
///
/// Lookups go through a `rpds::HashTrieMap` from key to insertion sequence
/// number; the sequence number then indexes a `rpds::RedBlackTreeMap` that
/// holds the actual `(key, value)` pairs in insertion order. Re-associating
/// an existing key keeps its original position (matching `PersistentArrayMap`
/// and how most ordered-map implementations behave), so iteration order is
/// deterministic and matches the order keys were first written, rather than
/// depending on hash-bucket layout (which — since `rpds` seeds its hasher
/// randomly per instance — would otherwise vary from run to run).
///
/// Small maps (≤8 entries) are represented as `PersistentArrayMap` instead;
/// the two types share the same `Value::Map` variant.  `PersistentHashMap` is
/// used once the entry count exceeds the array-map threshold.
///
/// A map derived by `assoc` or `dissoc` from a GC-allocated map reports only
/// its path-copied nodes from `gc_size_extra`; see
/// `collections::share_charge` for the full rule.
#[derive(Debug, Clone)]
pub struct PersistentHashMap {
    index: HashTrieMapSync<Value, u64>,
    order: RedBlackTreeMapSync<u64, (Value, Value)>,
    next_seq: u64,
    charge: ShareCharge,
}

/// An entry of the insertion-order tree.
type OrderEntry = (u64, (Value, Value));

impl PersistentHashMap {
    pub fn empty() -> Self {
        Self {
            index: HashTrieMapSync::new_sync(),
            next_seq: 0,
            order: RedBlackTreeMapSync::new_sync(),
            charge: ShareCharge::full(),
        }
    }

    /// Build from a raw `HashTrieMap`, in its (arbitrary) iteration order.
    ///
    /// Prefer `from_pairs`/`assoc` when the caller has a meaningful source
    /// order to preserve.
    pub fn new(map: HashTrieMapSync<Value, Value>) -> Self {
        Self::from_pairs(map.iter().map(|(k, v)| (k.clone(), v.clone())))
    }

    pub fn count(&self) -> usize {
        self.index.size()
    }

    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Look up a key.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        let seq = self.index.get(key)?;
        self.order.get(seq).map(|(_, v)| v)
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.index.contains_key(key)
    }

    /// Return a new map with `key` → `value`.
    ///
    /// Re-associating a key that is already present keeps its original
    /// insertion position; a brand-new key is appended at the end.
    pub fn assoc(&self, key: Value, value: Value) -> Self {
        if let Some(seq) = self.index.get(&key) {
            let seq = *seq;
            let n = self.count();
            Self {
                index: self.index.clone(),
                order: self.order.insert(seq, (key, value)),
                next_seq: self.next_seq,
                charge: self.charge.derive(n, n, rb_path_bytes::<OrderEntry>(n)),
            }
        } else {
            let seq = self.next_seq;
            let n = self.count();
            Self {
                index: self.index.insert(key.clone(), seq),
                order: self.order.insert(seq, (key, value)),
                next_seq: seq + 1,
                charge: self.charge.derive(n, n + 1, Self::path_bytes(n + 1)),
            }
        }
    }

    /// Return a new map with `key` removed.
    pub fn dissoc(&self, key: &Value) -> Self {
        match self.index.get(key) {
            Some(seq) => {
                let n = self.count();
                Self {
                    index: self.index.remove(key),
                    order: self.order.remove(seq),
                    next_seq: self.next_seq,
                    charge: self.charge.derive(n, n - 1, Self::path_bytes(n)),
                }
            }
            None => {
                let n = self.count();
                Self {
                    index: self.index.clone(),
                    order: self.order.clone(),
                    next_seq: self.next_seq,
                    charge: self.charge.derive(n, n, 0),
                }
            }
        }
    }

    /// Bytes path-copied in both trees by inserting or removing one entry.
    fn path_bytes(n: usize) -> usize {
        hamt_path_bytes::<(Value, u64)>(n) + rb_path_bytes::<OrderEntry>(n)
    }

    /// Estimated bytes owned by this map if it shared no nodes.
    pub(crate) fn full_size_extra(&self) -> usize {
        // Per entry: index HashTrieMap entry (~40) + order RedBlackTree node
        // (~48) + values stored inline in the order tree (not behind GcPtr).
        self.count() * (88 + 2 * std::mem::size_of::<Value>())
    }

    /// Iterate over all `(key, value)` pairs in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        self.order.iter().map(|(_, (k, v))| (k, v))
    }

    /// Collect all keys, in insertion order.
    pub fn keys(&self) -> Vec<Value> {
        self.iter().map(|(k, _)| k.clone()).collect()
    }

    /// Collect all values, in insertion order.
    pub fn vals(&self) -> Vec<Value> {
        self.iter().map(|(_, v)| v.clone()).collect()
    }

    /// Merge two maps; right-hand side wins on key collision.
    pub fn merge(&self, other: &Self) -> Self {
        let mut result = self.clone();
        for (k, v) in other.iter() {
            result = result.assoc(k.clone(), v.clone());
        }
        result
    }

    /// Build from an iterator of `(key, value)` pairs, in the given order.
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
        self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl cljrs_gc::Trace for PersistentHashMap {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for (_, (k, v)) in self.order.iter() {
            k.trace(visitor);
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
    use crate::collections::array_map::AssocResult;
    use cljrs_gc::GcPtr;

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

    #[test]
    fn test_iteration_order_matches_insertion_order() {
        let mut m = PersistentHashMap::empty();
        let keys = ["j", "i", "h", "g", "f", "e", "d", "c", "b", "a", "z", "y"];
        for k in keys {
            m = m.assoc(kw(k), int(0));
        }
        let iterated: Vec<Value> = m.keys();
        let expected: Vec<Value> = keys.iter().map(|k| kw(k)).collect();
        assert_eq!(iterated, expected);
    }

    #[test]
    fn test_reassoc_keeps_original_position() {
        let m = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("b"), int(2))
            .assoc(kw("c"), int(3))
            .assoc(kw("b"), int(99));
        let keys = m.keys();
        assert_eq!(keys, vec![kw("a"), kw("b"), kw("c")]);
        assert_eq!(m.get(&kw("b")), Some(&int(99)));
    }

    #[test]
    fn test_dissoc_preserves_remaining_order() {
        let m = PersistentHashMap::empty()
            .assoc(kw("a"), int(1))
            .assoc(kw("b"), int(2))
            .assoc(kw("c"), int(3))
            .dissoc(&kw("b"));
        assert_eq!(m.keys(), vec![kw("a"), kw("c")]);
    }

    #[test]
    fn assoc_on_allocated_map_charges_path_bytes() {
        use cljrs_gc::Trace;

        let m = GcPtr::new(PersistentHashMap::from_pairs(
            (0..10_000).map(|i| (int(i), int(i))),
        ));
        let added = m.get().assoc(int(-1), int(0));
        assert_eq!(added.gc_size_extra(), PersistentHashMap::path_bytes(10_001));
        let replaced = m.get().assoc(int(5), int(0));
        assert_eq!(
            replaced.gc_size_extra(),
            rb_path_bytes::<OrderEntry>(10_000)
        );
        let removed = m.get().dissoc(&int(5));
        assert_eq!(
            removed.gc_size_extra(),
            PersistentHashMap::path_bytes(10_000)
        );
        assert!(removed.gc_size_extra() * 100 < m.get().full_size_extra());
    }
}
