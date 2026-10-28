use super::Snapshot;
use crate::hash::hash_combine_unordered;
use crate::{ClojureHash, PersistentHashSet, Value, ValueError, ValueResult};
use std::sync::{
    Mutex,
    atomic::{AtomicU64, Ordering},
};

#[derive(Debug)]
pub struct TransientSet {
    set: Mutex<PersistentHashSet>,
    persisted: Mutex<bool>,
    revision: AtomicU64,
}

impl TransientSet {
    pub fn new() -> Self {
        TransientSet {
            set: Mutex::new(PersistentHashSet::empty()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
        }
    }

    pub fn new_from_set(set: &PersistentHashSet) -> TransientSet {
        TransientSet {
            set: Mutex::new(set.clone()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
        }
    }

    fn update(
        &self,
        operation: impl Fn(&PersistentHashSet) -> PersistentHashSet,
    ) -> ValueResult<()> {
        loop {
            let (snapshot, revision) = {
                let set = self.set.lock().unwrap();
                if *self.persisted.lock().unwrap() {
                    return Err(ValueError::TransientAlreadyPersisted);
                }
                (
                    Snapshot::new(set.clone()),
                    self.revision.load(Ordering::Relaxed),
                )
            };
            let candidate = operation(snapshot.get());
            let mut set = self.set.lock().unwrap();
            if *self.persisted.lock().unwrap() {
                return Err(ValueError::TransientAlreadyPersisted);
            }
            if self.revision.load(Ordering::Relaxed) != revision {
                continue;
            }
            *set = candidate;
            self.revision.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
    }
    pub fn conj(&self, value: Value) -> ValueResult<()> {
        self.update(|set| set.conj(value.clone()))
    }
    pub fn disj(&self, value: &Value) -> ValueResult<()> {
        self.update(|set| set.disj(value))
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
        let snapshot = Snapshot::new(self.set.lock().unwrap().clone());
        snapshot.get().contains(value)
    }
}

impl Clone for TransientSet {
    fn clone(&self) -> Self {
        Self {
            set: Mutex::new(self.set.lock().unwrap().clone()),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
            revision: AtomicU64::new(0),
        }
    }
}

impl ClojureHash for TransientSet {
    fn clojure_hash(&self) -> u32 {
        let mut hash: u32 = 0;
        let snapshot = Snapshot::new(self.set.lock().unwrap().clone());
        for v in snapshot.get().iter() {
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
