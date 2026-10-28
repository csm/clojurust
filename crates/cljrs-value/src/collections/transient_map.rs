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
}

impl TransientMap {
    pub fn new() -> TransientMap {
        TransientMap {
            map: Mutex::new(PersistentHashMap::empty()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            keys_may_realize: AtomicBool::new(false),
        }
    }

    pub fn new_from_map(map: &PersistentHashMap) -> TransientMap {
        TransientMap {
            map: Mutex::new(map.clone()),
            persisted: Mutex::new(false),
            revision: AtomicU64::new(0),
            keys_may_realize: AtomicBool::new(
                !(map.iter().all(|(key, _)| crate::realization::is_inert(key))),
            ),
        }
    }

    fn update(
        &self,
        key: &Value,
        fast: impl FnOnce(&mut PersistentHashMap),
        operation: impl Fn(&PersistentHashMap) -> PersistentHashMap,
    ) -> ValueResult<()> {
        {
            let mut collection = self.map.lock().unwrap();
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
                let map = self.map.lock().unwrap();
                if *self.persisted.lock().unwrap() {
                    return Err(ValueError::TransientAlreadyPersisted);
                }
                (
                    Snapshot::new(map.clone()),
                    self.revision.load(Ordering::Relaxed),
                )
            };
            let candidate = operation(snapshot.get());
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
            return Ok(());
        }
    }
    pub fn assoc(&self, key: Value, value: Value) -> ValueResult<()> {
        self.update(
            &key,
            |map| map.assoc_mut(key.clone(), value.clone()),
            |map| map.assoc(key.clone(), value.clone()),
        )
    }
    pub fn dissoc(&self, key: &Value) -> ValueResult<()> {
        self.update(key, |map| map.dissoc_mut(key), |map| map.dissoc(key))
    }

    pub fn persistent(&self) -> ValueResult<PersistentHashMap> {
        let map = self.map.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        Ok(map.clone())
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
            map.get(key).map(|value| (key.clone(), value.clone()))
        })
    }
    pub fn get(&self, key: &Value) -> Option<Value> {
        self.lookup(key, |map| map.get(key).cloned())
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
