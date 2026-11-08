use crate::hash::hash_combine_ordered;
use crate::{ClojureHash, PersistentVector, Value, ValueError, ValueResult};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug)]
pub struct TransientVector {
    vector: Mutex<rpds::VectorSync<Value>>,
    persisted: Mutex<bool>,
    scalar_only: AtomicBool,
    #[cfg(feature = "no-gc")]
    initial: std::cell::RefCell<crate::publish::ValueLease>,
}

impl TransientVector {
    pub fn new() -> TransientVector {
        TransientVector {
            vector: Mutex::new(Default::default()),
            #[cfg(feature = "no-gc")]
            initial: Default::default(),
            persisted: Mutex::new(false),
            scalar_only: AtomicBool::new(true),
        }
    }

    pub fn new_from_vector(vector: &rpds::VectorSync<Value>) -> TransientVector {
        TransientVector {
            vector: Mutex::new(vector.clone()),
            #[cfg(feature = "no-gc")]
            initial: std::cell::RefCell::new(crate::publish::ValueLease::for_trace(
                &PersistentVector::from_vector_summary(vector.clone(), false),
                &Default::default(),
            )),
            persisted: Mutex::new(false),
            scalar_only: AtomicBool::new(false),
        }
    }

    /// Share a persistent vector and its cached scalar summary.
    pub fn from_persistent(vector: &PersistentVector) -> Self {
        let result = Self::new_from_vector(vector.inner());
        result
            .scalar_only
            .store(vector.scalar_only(), Ordering::Relaxed);
        result
    }

    pub fn append(&self, v: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        if !super::vector::scalar(&v) {
            self.scalar_only.store(false, Ordering::Relaxed);
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
        if !super::vector::scalar(&v) {
            self.scalar_only.store(false, Ordering::Relaxed);
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
        #[cfg(feature = "no-gc")]
        self.initial.borrow().protect_read();
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
        let result = PersistentVector::from_vector_summary(
            vector.clone(),
            self.scalar_only.load(Ordering::Relaxed),
        );
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
            #[cfg(feature = "no-gc")]
            initial: std::cell::RefCell::new(crate::publish::ValueLease::for_trace(
                &PersistentVector::from_vector_summary(self.vector.lock().unwrap().clone(), false),
                &Default::default(),
            )),
            persisted: Mutex::new(*self.persisted.lock().unwrap()),
            scalar_only: AtomicBool::new(self.scalar_only.load(Ordering::Relaxed)),
        }
    }
}

impl cljrs_gc::Trace for TransientVector {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let address = self as *const _ as usize;
        let owner = cljrs_gc::region::AllocationOwner::for_address(address);
        let lease = self.initial.borrow().reparent(&owner);
        *self.initial.borrow_mut() = lease;
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
