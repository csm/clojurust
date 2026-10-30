/*
 * transient_map.rs -- implementation of transient maps.
 * Copyright (C) 2026 Casey Marshall
 *
 * This file is licensed under the Eclipse Public License, Version 1,
 * the same license as Clojure.
 */

use super::Snapshot;
use crate::hash::{hash_combine_ordered, hash_combine_unordered};
use crate::{ClojureHash, PersistentHashMap, Value, ValueError, ValueResult};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[derive(Debug)]
pub struct TransientMap {
    map: Mutex<PersistentHashMap>,
    persisted: Mutex<bool>,
    revision: AtomicU64,
    keys_may_realize: AtomicBool,
    #[cfg(feature = "no-gc")]
    initial: PersistentHashMap,
}

impl TransientMap {
    pub fn new() -> TransientMap {
        TransientMap {
            map: Mutex::new(PersistentHashMap::empty()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            initial: PersistentHashMap::empty(),
            keys_may_realize: AtomicBool::new(false),
        }
    }

    pub fn new_from_map(map: &PersistentHashMap) -> TransientMap {
        TransientMap {
            map: Mutex::new(map.clone()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            initial: map.clone(),
            keys_may_realize: AtomicBool::new(!map.keys_inert()),
        }
    }

    fn update<T>(
        &self,
        key: &Value,
        fast: impl FnOnce(&mut PersistentHashMap) -> T,
        operation: impl Fn(&PersistentHashMap) -> (PersistentHashMap, T),
    ) -> ValueResult<T> {
        {
            let mut collection = self.map.lock().unwrap();
            if *self.persisted.lock().unwrap() {
                return Err(ValueError::TransientAlreadyPersisted);
            }
            if !self.keys_may_realize.load(Ordering::Relaxed) && crate::realization::is_inert(key) {
                let result = fast(&mut collection);
                self.revision.fetch_add(1, Ordering::Relaxed);
                return Ok(result);
            }
        }
        loop {
            let (snapshot, revision) = {
                let map = self.map.lock().unwrap();
                if *self.persisted.lock().unwrap() {
                    return Err(ValueError::TransientAlreadyPersisted);
                }
                (
                    Snapshot::new(map.clone()),
                    self.revision.load(Ordering::Relaxed),
                )
            };
            let (candidate, result) = operation(snapshot.get());
            let mut map = self.map.lock().unwrap();
            if *self.persisted.lock().unwrap() {
                return Err(ValueError::TransientAlreadyPersisted);
            }
            if self.revision.load(Ordering::Relaxed) != revision {
                continue;
            }
            *map = candidate;
            if !crate::realization::is_inert(key) {
                self.keys_may_realize.store(true, Ordering::Relaxed);
            }
            self.revision.fetch_add(1, Ordering::Relaxed);
            return Ok(result);
        }
    }
    pub fn assoc(&self, key: Value, value: Value) -> ValueResult<()> {
        #[cfg(feature = "no-gc")]
        let (key, _key_owner) = crate::publish::prepare_for_owner(key, &Default::default());
        #[cfg(feature = "no-gc")]
        let (value, _value_owner) = crate::publish::prepare_for_owner(value, &Default::default());
        let entry = self.update(
            &key,
            |map| {
                map.assoc_mut(key.clone(), value.clone());
                map.storage_entry(&key).unwrap()
            },
            |map| {
                let candidate = map.assoc(key.clone(), value.clone());
                let entry = candidate.storage_entry(&key).unwrap();
                (candidate, entry)
            },
        )?;
        #[cfg(feature = "no-gc")]
        if !cljrs_gc::alloc_ctx::invocation_is_active() {
            let lease = cljrs_gc::region::RegionLease::for_values(&[entry.1, value]);
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                entry.0,
                &lease,
            );
        }
        #[cfg(not(feature = "no-gc"))]
        let _ = entry;
        Ok(())
    }
    pub fn dissoc(&self, key: &Value) -> ValueResult<()> {
        let entry = self.update(
            key,
            |map| {
                let entry = map.storage_entry(key);
                map.dissoc_mut(key);
                entry
            },
            |map| (map.dissoc(key), map.storage_entry(key)),
        )?;
        #[cfg(feature = "no-gc")]
        if let Some((slot, _)) = entry {
            crate::publish::publish_for_slot(Value::Nil, self as *const _ as usize, slot);
        }
        #[cfg(not(feature = "no-gc"))]
        let _ = entry;
        Ok(())
    }

    pub fn persistent(&self) -> ValueResult<PersistentHashMap> {
        let map = self.map.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        let result = map.clone();
        #[cfg(feature = "no-gc")]
        {
            let address = self as *const _ as usize;
            if cljrs_gc::region::AllocationOwner::for_address(address).is_permanent() {
                // Rust stack containers do not have a containing region to lease.
                cljrs_gc::region::RegionLease::for_value(&result).protect_read();
            } else {
                cljrs_gc::region::RegionLease::for_address(address).protect_read();
            }
        }
        Ok(result)
    }

    fn lookup<T>(&self, key: &Value, operation: impl Fn(&PersistentHashMap) -> T) -> T {
        let map = self.map.lock().unwrap();
        if !self.keys_may_realize.load(Ordering::Relaxed) && crate::realization::is_inert(key) {
            return operation(&map);
        }
        let snapshot = Snapshot::new(map.clone());
        drop(map);
        operation(snapshot.get())
    }
    pub fn find(&self, key: &Value) -> Option<(Value, Value)> {
        self.lookup(key, |map| {
            map.get(key).map(|value| {
                #[cfg(feature = "no-gc")]
                cljrs_gc::region::RegionLease::for_values(&[key.clone(), value.clone()])
                    .protect_read();
                (key.clone(), value.clone())
            })
        })
    }
    pub fn get(&self, key: &Value) -> Option<Value> {
        self.lookup(key, |map| {
            let value = map.get(key).cloned();
            #[cfg(feature = "no-gc")]
            if let Some(value) = &value {
                cljrs_gc::region::RegionLease::for_value(value).protect_read();
            }
            value
        })
    }
    pub fn contains_key(&self, key: &Value) -> bool {
        self.lookup(key, |map| map.contains_key(key))
    }

    pub fn count(&self) -> usize {
        let map = self.map.lock().unwrap();
        map.count()
    }
}

impl Clone for TransientMap {
    fn clone(&self) -> Self {
        Self {
            map: Mutex::new(self.map.lock().unwrap().clone()),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
            revision: AtomicU64::new(0),
            #[cfg(feature = "no-gc")]
            initial: self.map.lock().unwrap().clone(),
            keys_may_realize: AtomicBool::new(self.keys_may_realize.load(Ordering::Relaxed)),
        }
    }
}

impl ClojureHash for TransientMap {
    fn clojure_hash(&self) -> u32 {
        let mut hash: u32 = 0;
        let snapshot = Snapshot::new(self.map.lock().unwrap().clone());
        for (k, v) in snapshot.get().iter() {
            hash = hash_combine_unordered(
                hash,
                hash_combine_ordered(k.clojure_hash(), v.clojure_hash()),
            );
        }
        hash
    }
}

impl cljrs_gc::Trace for TransientMap {
    #[cfg(feature = "no-gc")]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.initial.trace(visitor);
    }
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let map = self.map.lock().unwrap();
            for (k, v) in map.iter() {
                k.trace(visitor);
                v.trace(visitor);
            }
        }
    }

    fn gc_size_extra(&self) -> usize {
        let map = self.map.lock().unwrap();
        map.full_size_extra()
    }
}

impl Default for TransientMap {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TransientMap {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}
