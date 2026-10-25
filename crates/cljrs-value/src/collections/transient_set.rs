use crate::hash::hash_combine_unordered;
use crate::{ClojureHash, PersistentHashSet, Value, ValueError, ValueResult};
use std::sync::Mutex;

#[derive(Debug)]
pub struct TransientSet {
    set: Mutex<PersistentHashSet>,
    persisted: Mutex<bool>,
}

impl TransientSet {
    pub fn new() -> Self {
        TransientSet {
            set: Mutex::new(PersistentHashSet::empty()),
            persisted: Mutex::new(false),
        }
    }

    pub fn new_from_set(set: &PersistentHashSet) -> TransientSet {
        TransientSet {
            set: Mutex::new(set.clone()),
            persisted: Mutex::new(false),
        }
    }

    pub fn conj(&self, value: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let mut set = self.set.lock().unwrap();
        set.conj_mut(value);
        Ok(())
    }

    pub fn disj(&self, value: &Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let mut set = self.set.lock().unwrap();
        *set = set.disj(value);
        Ok(())
    }

    pub fn persistent(&self) -> ValueResult<PersistentHashSet> {
        let set = self.set.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        Ok(set.clone())
    }

    pub fn count(&self) -> usize {
        let set = self.set.lock().unwrap();
        set.count()
    }

    pub fn contains(&self, value: &Value) -> bool {
        self.set.lock().unwrap().contains(value)
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
        {
            let set = self.set.lock().unwrap();
            for v in set.iter() {
                v.trace(visitor);
            }
        }
    }

    fn gc_size_extra(&self) -> usize {
        let set = self.set.lock().unwrap();
        set.full_size_extra()
    }
}

impl Default for TransientSet {
    fn default() -> Self {
        Self::new()
    }
}
