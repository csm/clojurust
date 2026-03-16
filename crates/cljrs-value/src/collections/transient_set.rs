use crate::hash::hash_combine_unordered;
use crate::{ClojureHash, PersistentHashSet, PersistentVector, Value, ValueError, ValueResult};
use std::hash::Hash;
use std::ops::Deref;
use std::sync::Mutex;

#[derive(Debug)]
pub struct TransientSet {
    set: Mutex<rpds::HashTrieSetSync<Value>>,
    persisted: Mutex<bool>,
}

impl TransientSet {
    pub fn new() -> Self {
        TransientSet {
            set: Mutex::new(Default::default()),
            persisted: Mutex::new(false),
        }
    }

    pub fn new_from_set(set: &rpds::HashTrieSetSync<Value>) -> TransientSet {
        TransientSet {
            set: Mutex::new(set.clone()),
            persisted: Mutex::new(false),
        }
    }

    pub fn conj(&self, value: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        self.set.lock().unwrap().insert_mut(value.clone());
        Ok(())
    }

    pub fn disj(&self, value: &Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        self.set.lock().unwrap().remove_mut(value);
        Ok(())
    }

    pub fn persistent(&self) -> ValueResult<PersistentHashSet> {
        let set = self.set.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        Ok(PersistentHashSet::from_set(set.clone()))
    }
}

impl Clone for TransientSet {
    fn clone(&self) -> Self {
        Self {
            set: Mutex::new(self.set.lock().unwrap().clone()),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
        }
    }
}

impl ClojureHash for TransientSet {
    fn clojure_hash(&self) -> u32 {
        let mut hash: u32 = 0;
        for v in self.set.lock().unwrap().iter() {
            hash = hash_combine_unordered(hash, v.clojure_hash())
        }
        hash
    }
}

impl cljrs_gc::Trace for TransientSet {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        let map = self.set.lock().unwrap();
        for v in map.iter() {
            v.trace(visitor);
        }
    }
}
