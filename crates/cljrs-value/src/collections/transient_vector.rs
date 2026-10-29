use crate::hash::hash_combine_ordered;
use crate::{ClojureHash, PersistentVector, Value, ValueError, ValueResult};
use std::sync::Mutex;

#[derive(Debug)]
pub struct TransientVector {
    vector: Mutex<rpds::VectorSync<Value>>,
    persisted: Mutex<bool>,
}

impl TransientVector {
    pub fn new() -> TransientVector {
        TransientVector {
            vector: Mutex::new(Default::default()),
            persisted: Mutex::new(false),
        }
    }

    pub fn new_from_vector(vector: &rpds::VectorSync<Value>) -> TransientVector {
        TransientVector {
            vector: Mutex::new(vector.clone()),
            persisted: Mutex::new(false),
        }
    }

    pub fn append(&self, v: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let v = crate::publish::publish_for_slot(v, self as *const _ as usize, self.count());
        let mut vector = self.vector.lock().unwrap();
        vector.push_back_mut(v);
        Ok(())
    }

    pub fn pop(&self) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let mut vector = self.vector.lock().unwrap();
        if vector.drop_last_mut() {
            let index = vector.len();
            drop(vector);
            crate::publish::publish_for_slot(Value::Nil, self as *const _ as usize, index);
            Ok(())
        } else {
            Err(ValueError::OutOfRange)
        }
    }

    pub fn set(&self, index: usize, v: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let count = self.count();
        if index > count {
            return Err(ValueError::IndexOutOfBounds { idx: index, count });
        }
        let v = crate::publish::publish_for_slot(v, self as *const _ as usize, index);
        let mut vector = self.vector.lock().unwrap();
        if index == count {
            vector.push_back_mut(v);
        } else {
            vector.set_mut(index, v);
        }
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.vector.lock().unwrap().len()
    }

    pub fn get(&self, index: usize) -> Option<Value> {
        crate::publish::read_slot(self as *const _ as usize, index);
        self.vector.lock().unwrap().get(index).cloned()
    }

    pub fn persistent(&self) -> ValueResult<PersistentVector> {
        let vector = self.vector.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        let result = PersistentVector::from_vector(vector.clone());
        #[cfg(feature = "no-gc")]
        cljrs_gc::region::RegionLease::for_value(&result).protect_read();
        Ok(result)
    }
}

impl ClojureHash for TransientVector {
    fn clojure_hash(&self) -> u32 {
        let mut hash: u32 = 0;
        let values = self.vector.lock().unwrap().clone();
        let snapshot = super::Snapshot::new(crate::PersistentVector::from_vector(values));
        for v in snapshot.get().iter() {
            hash = hash_combine_ordered(hash, v.clojure_hash());
        }
        hash
    }
}

impl Clone for TransientVector {
    fn clone(&self) -> Self {
        Self {
            vector: Mutex::new(self.vector.lock().unwrap().clone()),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
        }
    }
}

impl cljrs_gc::Trace for TransientVector {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        for (index, value) in self.vector.lock().unwrap().iter().enumerate() {
            let lease = cljrs_gc::region::RegionLease::for_value(value);
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                index,
                &lease,
            );
        }
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let vec = self.vector.lock().unwrap();
            for v in vec.iter() {
                v.trace(visitor);
            }
        }
    }

    fn gc_size_extra(&self) -> usize {
        let n = self.vector.lock().unwrap().len();
        n * (24 + std::mem::size_of::<crate::Value>())
    }
}

impl Default for TransientVector {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for TransientVector {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}
