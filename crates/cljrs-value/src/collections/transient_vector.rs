use crate::hash::hash_combine_ordered;
use crate::{ClojureHash, PersistentVector, Value, ValueError, ValueResult};
use std::hash::Hash;
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
            Ok(())
        } else {
            Err(ValueError::OutOfRange)
        }
    }

    pub fn set(&self, index: usize, v: Value) -> ValueResult<()> {
        if *self.persisted.lock().unwrap() {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        let mut vector = self.vector.lock().unwrap();
        if index > vector.len() {
            return Err(ValueError::IndexOutOfBounds {
                idx: index,
                count: vector.len(),
            });
        }
        vector.set_mut(index, v);
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.vector.lock().unwrap().len()
    }

    pub fn persistent(&self) -> ValueResult<PersistentVector> {
        let vector = self.vector.lock().unwrap();
        let mut persisted = self.persisted.lock().unwrap();
        if *persisted {
            return Err(ValueError::TransientAlreadyPersisted);
        }
        *persisted = true;
        Ok(PersistentVector::from_vector(vector.clone()))
    }
}

impl ClojureHash for TransientVector {
    fn clojure_hash(&self) -> u32 {
        let mut hash: u32 = 0;
        for v in self.vector.lock().unwrap().iter() {
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
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        let vec = self.vector.lock().unwrap();
        for v in vec.iter() {
            v.trace(visitor);
        }
    }
}
