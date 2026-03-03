use cljx_gc::GcPtr;

use crate::Value;
use crate::collections::array_map::PersistentArrayMap;
use crate::collections::hamt::node::Node;
use crate::hash::ClojureHash;

/// A key-value pair stored in the HAMT.
#[derive(Debug, Clone)]
pub(crate) struct KVPair(pub Value, pub Value);

/// An immutable hash map using a 32-way HAMT trie.
///
/// Small maps (≤8 entries) are represented as `PersistentArrayMap` instead;
/// the two types share the same `Value::Map` variant.  `PersistentHashMap` is
/// used once the entry count exceeds the array-map threshold.
#[derive(Debug, Clone)]
pub struct PersistentHashMap {
    root: Option<GcPtr<Node<KVPair>>>,
    count: usize,
}

impl PersistentHashMap {
    pub fn empty() -> Self {
        Self {
            root: None,
            count: 0,
        }
    }

    pub fn count(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    fn key_eq(key: &Value) -> impl Fn(&KVPair) -> bool + '_ {
        move |pair: &KVPair| pair.0 == *key
    }

    /// Look up a key.
    pub fn get(&self, key: &Value) -> Option<&Value> {
        let root = self.root.as_ref()?;
        let hash = key.clojure_hash();
        root.get().get(hash, 0, &Self::key_eq(key)).map(|p| &p.1)
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.get(key).is_some()
    }

    /// Return a new map with `key` → `value`.
    pub fn assoc(&self, key: Value, value: Value) -> Self {
        let hash = key.clojure_hash();
        let pair = KVPair(key, value);

        let (new_root, new_count) = match &self.root {
            None => {
                let leaf = GcPtr::new(Node::Leaf { hash, value: pair });
                (leaf, 1)
            }
            Some(root) => {
                // Check if the key already exists (for count tracking).
                let existed = root
                    .get()
                    .get(hash, 0, &|p: &KVPair| p.0 == pair.0)
                    .is_some();
                let key_eq = {
                    let k = pair.0.clone();
                    move |p: &KVPair| p.0 == k
                };
                let new_root = root.get().assoc(hash, 0, pair, &key_eq);
                let new_count = if existed { self.count } else { self.count + 1 };
                (new_root, new_count)
            }
        };

        Self {
            root: Some(new_root),
            count: new_count,
        }
    }

    /// Return a new map with `key` removed.
    pub fn dissoc(&self, key: &Value) -> Self {
        let root = match &self.root {
            None => return self.clone(),
            Some(r) => r,
        };
        let hash = key.clojure_hash();
        let existed = root.get().get(hash, 0, &Self::key_eq(key)).is_some();
        if !existed {
            return self.clone();
        }
        let new_root = root.get().dissoc(hash, 0, &Self::key_eq(key));
        Self {
            root: new_root,
            count: self.count - 1,
        }
    }

    /// Iterate over all `(key, value)` pairs in an unspecified order.
    pub fn iter(&self) -> impl Iterator<Item = (&Value, &Value)> {
        let pairs: Vec<(&Value, &Value)> = match &self.root {
            None => vec![],
            Some(root) => {
                // We can't return an iterator that borrows from a local Vec,
                // so collect into a Vec of references pointing into the trie.
                let mut out = Vec::with_capacity(self.count);
                root.get().for_each(&mut |p: &KVPair| {
                    // SAFETY: the references point into Arc-backed nodes that
                    // live at least as long as `self` (which owns the root Arc).
                    let k: &Value = unsafe { &*(&p.0 as *const Value) };
                    let v: &Value = unsafe { &*(&p.1 as *const Value) };
                    out.push((k, v));
                });
                out
            }
        };
        pairs.into_iter()
    }

    /// Collect all keys.
    pub fn keys(&self) -> Vec<Value> {
        match &self.root {
            None => vec![],
            Some(root) => {
                let mut out = Vec::new();
                root.get().for_each(&mut |p: &KVPair| out.push(p.0.clone()));
                out
            }
        }
    }

    /// Collect all values.
    pub fn vals(&self) -> Vec<Value> {
        match &self.root {
            None => vec![],
            Some(root) => {
                let mut out = Vec::new();
                root.get().for_each(&mut |p: &KVPair| out.push(p.1.clone()));
                out
            }
        }
    }

    /// Merge two maps; right-hand side wins on key collision.
    pub fn merge(&self, other: &Self) -> Self {
        let mut result = self.clone();
        match &other.root {
            None => {}
            Some(root) => {
                root.get().for_each(&mut |p: &KVPair| {
                    result = result.assoc(p.0.clone(), p.1.clone());
                });
            }
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
        if self.count != other.count {
            return false;
        }
        // Every key in self must exist in other with the same value.
        match &self.root {
            None => true,
            Some(root) => {
                let mut equal = true;
                root.get().for_each(&mut |p: &KVPair| {
                    if equal {
                        match other.get(&p.0) {
                            Some(v) if v == &p.1 => {}
                            _ => equal = false,
                        }
                    }
                });
                equal
            }
        }
    }
}

impl cljx_gc::Trace for PersistentHashMap {}
impl cljx_gc::Trace for KVPair {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;
    use crate::collections::array_map::AssocResult;

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
