use super::Snapshot;
use crate::hash::hash_combine_unordered;
use crate::{ClojureHash, PersistentHashSet, Value, ValueError, ValueResult};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Debug)]
pub struct TransientSet {
    set: Mutex<PersistentHashSet>,
    persisted: Mutex<bool>,
    revision: AtomicU64,
    keys_may_realize: AtomicBool,
    #[cfg(feature = "no-gc")]
    ownership: super::OwnershipSlots,
}

impl TransientSet {
    pub fn new() -> Self {
        TransientSet {
            set: Mutex::new(PersistentHashSet::empty()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            ownership: Default::default(),
            keys_may_realize: AtomicBool::new(false),
        }
    }

    pub fn new_from_set(set: &PersistentHashSet) -> TransientSet {
        TransientSet {
            set: Mutex::new(set.clone()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            ownership: Default::default(),
            keys_may_realize: AtomicBool::new(!(set.iter().all(crate::realization::is_inert))),
        }
    }

    fn update(
        &self,
        key: &Value,
        fast: impl FnOnce(&mut PersistentHashSet),
        operation: impl Fn(&PersistentHashSet) -> PersistentHashSet,
    ) -> ValueResult<()> {
        {
            let mut collection = self.set.lock().unwrap();
            if *self.persisted.lock().unwrap() {
                return Err(ValueError::TransientAlreadyPersisted);
            }
            if !self.keys_may_realize.load(Ordering::Relaxed) && crate::realization::is_inert(key) {
                fast(&mut collection);
                self.revision.fetch_add(1, Ordering::Relaxed);
                return Ok(());
            }
        }
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
            if !crate::realization::is_inert(key) {
                self.keys_may_realize.store(true, Ordering::Relaxed);
            }
            self.revision.fetch_add(1, Ordering::Relaxed);
            return Ok(());
        }
    }
    pub fn conj(&self, value: Value) -> ValueResult<()> {
        self.update(
            &value,
            |set| {
                set.conj_mut(value.clone());
            },
            |set| set.conj(value.clone()),
        )?;
        #[cfg(feature = "no-gc")]
        {
            let value = self
                .set
                .lock()
                .unwrap()
                .canonical_key(&value)
                .expect("conjoined key");
            let slot = self.ownership.id(&value);
            let lease = cljrs_gc::region::RegionLease::for_value(&value);
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                slot,
                &lease,
            );
        }
        Ok(())
    }
    pub fn disj(&self, value: &Value) -> ValueResult<()> {
        self.update(value, |set| set.disj_mut(value), |set| set.disj(value))?;
        #[cfg(feature = "no-gc")]
        if let Some(slot) = self.ownership.remove(value) {
            crate::publish::publish_for_slot(Value::Nil, self as *const _ as usize, slot);
        }
        Ok(())
    }

    pub fn persistent(&self) -> ValueResult<PersistentHashSet> {
        let set = self.set.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        let result = set.clone();
        #[cfg(feature = "no-gc")]
        cljrs_gc::region::RegionLease::for_value(&result).protect_read();
        Ok(result)
    }

    pub fn count(&self) -> usize {
        let set = self.set.lock().unwrap();
        set.count()
    }

    pub fn contains(&self, value: &Value) -> bool {
        let set = self.set.lock().unwrap();
        if !self.keys_may_realize.load(Ordering::Relaxed) && crate::realization::is_inert(value) {
            return set.contains(value);
        }
        let snapshot = Snapshot::new(set.clone());
        drop(set);
        snapshot.get().contains(value)
    }
}

impl Clone for TransientSet {
    fn clone(&self) -> Self {
        Self {
            set: Mutex::new(self.set.lock().unwrap().clone()),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            ownership: Default::default(),
            keys_may_realize: AtomicBool::new(self.keys_may_realize.load(Ordering::Relaxed)),
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
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        for value in self.set.lock().unwrap().iter() {
            let slot = self.ownership.id(value);
            let lease = cljrs_gc::region::RegionLease::for_value(value);
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                slot,
                &lease,
            );
        }
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
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

impl Drop for TransientSet {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}
