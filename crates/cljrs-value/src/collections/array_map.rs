use std::sync::Arc;

use crate::Value;

/// Maximum number of key-value pairs before promoting to `PersistentHashMap`.
pub const THRESHOLD: usize = 8;

/// A small immutable map stored as a flat key/value vector.
///
/// Linear-scan lookup is fast for small maps (≤8 entries) and avoids the
/// overhead of hashing.  Once the map exceeds `THRESHOLD` entries an `assoc`
/// returns a `PersistentHashMap` instead.
#[derive(Debug, Clone)]
pub struct PersistentArrayMap {
    /// Flat: `[k0, v0, k1, v1, …]`.  Always even-length; at most 16 elements.
    entries: Arc<Vec<Value>>,
}

/// Result of `assoc` — stays an array-map or promotes to a hash-map.
pub enum AssocResult {
    Array(PersistentArrayMap),
    /// The caller must construct the hash-map; we return the raw entries to
    /// avoid a circular dependency (HashMapincludes ArrayMap but not vice-versa).
    Promote(Vec<(Value, Value)>),
}

impl PersistentArrayMap {
    /// The canonical empty array-map.
    pub fn empty() -> Self {
        Self {
            entries: Arc::new(Vec::new()),
        }
    }

    pub fn count(&self) -> usize {
        self.entries.len() / 2
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Look up `key` using Clojure value equality.  O(n).
    pub fn get(&self, key: &Value) -> Option<&Value> {
        let e = &self.entries;
        let mut i = 0;
        while i < e.len() {
            if &e[i] == key {
                return Some(&e[i + 1]);
            }
            i += 2;
        }
        None
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        self.get(key).is_some()
    }

    /// Return a new map with `key` associated to `value`.
    /// Promotes to `Promote` when the count would exceed `THRESHOLD`.
    pub fn assoc(&self, key: Value, value: Value) -> AssocResult {
        // Check for existing key to replace.
        let mut new_entries = (*self.entries).clone();
        let mut i = 0;
        while i < new_entries.len() {
            if new_entries[i] == key {
                new_entries[i + 1] = value;
                return AssocResult::Array(Self {
                    entries: Arc::new(new_entries),
                });
            }
            i += 2;
        }

        // New key.
        new_entries.push(key);
        new_entries.push(value);

        if new_entries.len() / 2 > THRESHOLD {
            // Promote: collect pairs and let the caller build a hash-map.
            let mut pairs = Vec::with_capacity(new_entries.len() / 2);
            let mut j = 0;
            while j < new_entries.len() {
                pairs.push((new_entries[j].clone(), new_entries[j + 1].clone()));
                j += 2;
            }
            AssocResult::Promote(pairs)
        } else {
            AssocResult::Array(Self {
                entries: Arc::new(new_entries),
            })
        }
    }

    /// Return a new map with `key` removed.  O(n).
    pub fn dissoc(&self, key: &Value) -> Self {
        let mut new_entries = (*self.entries).clone();
        let mut i = 0;
        while i < new_entries.len() {
            if &new_entries[i] == key {
                new_entries.remove(i); // value
                new_entries.remove(i); // key
                return Self {
                    entries: Arc::new(new_entries),
                };
            }
            i += 2;
        }
        // Key not present → return a clone (new Arc, same data).
        self.clone()
    }

    /// Iterate over `(key, value)` pairs.
    pub fn iter(&self) -> ArrayMapIter<'_> {
        ArrayMapIter {
            entries: &self.entries,
            pos: 0,
        }
    }

    /// Build directly from a pre-evaluated flat entries vector `[k0, v0, k1, v1, ...]`.
    ///
    /// This avoids intermediate allocations when the entries are already known.
    /// Caller must ensure even length. Does NOT check for duplicate keys —
    /// if duplicates are possible, use `from_pairs` instead.
    pub fn from_flat_entries(entries: Vec<Value>) -> AssocResult {
        debug_assert!(entries.len().is_multiple_of(2));
        if entries.len() / 2 > THRESHOLD {
            let mut pairs = Vec::with_capacity(entries.len() / 2);
            let mut i = 0;
            while i < entries.len() {
                pairs.push((entries[i].clone(), entries[i + 1].clone()));
                i += 2;
            }
            AssocResult::Promote(pairs)
        } else {
            AssocResult::Array(Self {
                entries: Arc::new(entries),
            })
        }
    }

    /// Build from an iterator of `(key, value)` pairs.
    /// Pairs are inserted left-to-right; later values win on duplicate keys.
    pub fn from_pairs<I: IntoIterator<Item = (Value, Value)>>(iter: I) -> AssocResult {
        let mut map = Self::empty();
        for (k, v) in iter {
            match map.assoc(k, v) {
                AssocResult::Array(m) => map = m,
                p @ AssocResult::Promote(_) => return p,
            }
        }
        AssocResult::Array(map)
    }
}

pub struct ArrayMapIter<'a> {
    entries: &'a Vec<Value>,
    pos: usize,
}

impl<'a> Iterator for ArrayMapIter<'a> {
    type Item = (&'a Value, &'a Value);

    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.entries.len() {
            return None;
        }
        let k = &self.entries[self.pos];
        let v = &self.entries[self.pos + 1];
        self.pos += 2;
        Some((k, v))
    }
}

impl PartialEq for PersistentArrayMap {
    fn eq(&self, other: &Self) -> bool {
        if self.count() != other.count() {
            return false;
        }
        // Every key in self must map to the same value in other.
        self.iter().all(|(k, v)| other.get(k) == Some(v))
    }
}

impl cljrs_gc::Trace for PersistentArrayMap {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for v in self.entries.iter() {
            v.trace(visitor);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Value;

    fn kw(s: &str) -> Value {
        Value::Keyword(cljrs_gc::GcPtr::new(crate::keyword::Keyword::simple(s)))
    }

    fn int(n: i64) -> Value {
        Value::Long(n)
    }

    #[test]
    fn test_empty() {
        let m = PersistentArrayMap::empty();
        assert!(m.is_empty());
        assert_eq!(m.count(), 0);
    }

    #[test]
    fn test_assoc_and_get() {
        let m = PersistentArrayMap::empty();
        let AssocResult::Array(m) = m.assoc(kw("a"), int(1)) else {
            panic!()
        };
        let AssocResult::Array(m) = m.assoc(kw("b"), int(2)) else {
            panic!()
        };
        assert_eq!(m.get(&kw("a")), Some(&int(1)));
        assert_eq!(m.get(&kw("b")), Some(&int(2)));
        assert_eq!(m.get(&kw("c")), None);
    }

    #[test]
    fn test_update() {
        let m = PersistentArrayMap::empty();
        let AssocResult::Array(m) = m.assoc(kw("a"), int(1)) else {
            panic!()
        };
        let AssocResult::Array(m) = m.assoc(kw("a"), int(99)) else {
            panic!()
        };
        assert_eq!(m.get(&kw("a")), Some(&int(99)));
        assert_eq!(m.count(), 1);
    }

    #[test]
    fn test_dissoc() {
        let m = PersistentArrayMap::empty();
        let AssocResult::Array(m) = m.assoc(kw("a"), int(1)) else {
            panic!()
        };
        let AssocResult::Array(m) = m.assoc(kw("b"), int(2)) else {
            panic!()
        };
        let m2 = m.dissoc(&kw("a"));
        assert_eq!(m2.get(&kw("a")), None);
        assert_eq!(m2.get(&kw("b")), Some(&int(2)));
        assert_eq!(m2.count(), 1);
    }

    #[test]
    fn test_promotes_at_threshold() {
        let mut m = PersistentArrayMap::empty();
        for i in 0..THRESHOLD as i64 {
            let AssocResult::Array(next) = m.assoc(int(i), int(i * 10)) else {
                panic!()
            };
            m = next;
        }
        // Adding one more should trigger promotion.
        let result = m.assoc(int(THRESHOLD as i64), int(0));
        assert!(matches!(result, AssocResult::Promote(_)));
    }

    #[test]
    fn test_equality() {
        let m1 = PersistentArrayMap::empty();
        let AssocResult::Array(m1) = m1.assoc(kw("a"), int(1)) else {
            panic!()
        };
        let m2 = PersistentArrayMap::empty();
        let AssocResult::Array(m2) = m2.assoc(kw("a"), int(1)) else {
            panic!()
        };
        assert_eq!(m1, m2);
    }
}
