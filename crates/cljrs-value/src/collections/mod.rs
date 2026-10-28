pub mod array_map;
pub mod hash_map;
pub mod hash_set;
pub mod list;
pub mod queue;
mod share_charge;
pub mod sorted_map;
pub mod sorted_set;
pub mod transient_map;
pub mod transient_set;
pub mod transient_vector;
pub mod vector;

pub use array_map::PersistentArrayMap;
pub use hash_map::PersistentHashMap;
pub use hash_set::PersistentHashSet;
pub use list::PersistentList;
pub use queue::PersistentQueue;
pub use sorted_map::SortedMap;
pub use sorted_set::SortedSet;
pub use transient_map::TransientMap;
pub use transient_set::TransientSet;
pub use transient_vector::TransientVector;
pub use vector::PersistentVector;

// Stable host storage, traced through a per-mutator shadow stack. Snapshots
// do not create managed boxes or keep whole allocation frames alive.
#[cfg(not(feature = "no-gc"))]
thread_local! {
    static SNAPSHOT_ROOTS: std::cell::RefCell<Vec<*const dyn cljrs_gc::Trace>> = const { std::cell::RefCell::new(Vec::new()) };
}
#[cfg(not(feature = "no-gc"))]
cljrs_gc::mutator_slot!("cljrs-value/SNAPSHOT_ROOTS", refcell SNAPSHOT_ROOTS: Vec<*const dyn cljrs_gc::Trace>, trace trace_saved_snapshots);
#[cfg(not(feature = "no-gc"))]
fn trace_saved_snapshots(
    roots: &Vec<*const dyn cljrs_gc::Trace>,
    visitor: &mut cljrs_gc::MarkVisitor,
) {
    for &root in roots {
        // SAFETY: private Snapshot guards own stable boxed data. The owning
        // fiber retains that storage until it restores its state and unwinds.
        unsafe { (&*root).trace(visitor) };
    }
}
/// Trace active collection snapshots. Runtime collectors call this in addition
/// to tracing detached mutator states.
#[doc(hidden)]
pub fn trace_snapshots(visitor: &mut cljrs_gc::MarkVisitor) {
    #[cfg(not(feature = "no-gc"))]
    SNAPSHOT_ROOTS.with(|roots| trace_saved_snapshots(&roots.borrow(), visitor));
    #[cfg(feature = "no-gc")]
    let _ = visitor;
}

pub(crate) struct Snapshot<T: cljrs_gc::Trace + 'static> {
    #[cfg(not(feature = "no-gc"))]
    data: Box<T>,
    #[cfg(feature = "no-gc")]
    data: T,
    _not_send: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl<T: cljrs_gc::Trace + 'static> Snapshot<T> {
    pub(crate) fn new(data: T) -> Self {
        #[cfg(not(feature = "no-gc"))]
        let data = {
            let data = Box::new(data);
            SNAPSHOT_ROOTS.with(|roots| {
                roots
                    .borrow_mut()
                    .push(data.as_ref() as &dyn cljrs_gc::Trace)
            });
            data
        };
        Self {
            data,
            _not_send: std::marker::PhantomData,
        }
    }
    pub(crate) fn get(&self) -> &T {
        &self.data
    }
}
impl<T: cljrs_gc::Trace + 'static> Drop for Snapshot<T> {
    fn drop(&mut self) {
        #[cfg(not(feature = "no-gc"))]
        SNAPSHOT_ROOTS.with(|roots| {
            let root = roots
                .borrow_mut()
                .pop()
                .expect("missing collection snapshot root");
            assert!(std::ptr::addr_eq(
                root,
                self.data.as_ref() as &dyn cljrs_gc::Trace
            ));
        });
    }
}

struct ValueSnapshot(Vec<crate::Value>);
impl cljrs_gc::Trace for ValueSnapshot {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for value in &self.0 {
            value.trace(visitor);
        }
    }
    fn gc_size_extra(&self) -> usize {
        self.0.capacity() * std::mem::size_of::<crate::Value>()
    }
}
pub(crate) fn hash_values(values: Vec<crate::Value>) -> u32 {
    use crate::ClojureHash;
    let snapshot = Snapshot::new(ValueSnapshot(values));
    snapshot.get().0.iter().fold(0, |hash, value| {
        crate::hash::hash_combine_ordered(hash, value.clojure_hash())
    })
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::Value;

    #[test]
    fn host_snapshot_roots_survive_collection_without_a_managed_box() {
        let frame = cljrs_gc::push_alloc_frame();
        let values = ValueSnapshot(vec![Value::string("retained host snapshot")]);
        let before = cljrs_gc::HEAP.total_allocated();
        let snapshot = Snapshot::new(values);
        assert_eq!(cljrs_gc::HEAP.total_allocated(), before);
        drop(frame);
        #[cfg(not(feature = "no-gc"))]
        for _ in 0..3 {
            cljrs_gc::HEAP.collect(trace_snapshots);
        }
        assert_eq!(snapshot.get().0[0], Value::string("retained host snapshot"));
    }

    #[test]
    fn scalar_transient_operations_do_not_allocate_managed_snapshots() {
        let map = TransientMap::new();
        let set = TransientSet::new();
        let before = cljrs_gc::HEAP.total_allocated();
        for i in 0..1000 {
            let key = Value::Long(i);
            map.assoc(key.clone(), Value::Long(i + 1)).unwrap();
            set.conj(key.clone()).unwrap();
            assert_eq!(map.get(&key), Some(Value::Long(i + 1)));
            assert_eq!(map.find(&key), Some((key.clone(), Value::Long(i + 1))));
            assert!(map.contains_key(&key));
            assert!(set.contains(&key));
        }
        for i in 0..500 {
            map.dissoc(&Value::Long(i)).unwrap();
            set.disj(&Value::Long(i)).unwrap();
        }
        assert_eq!(cljrs_gc::HEAP.total_allocated(), before);
        assert_eq!(map.count(), 500);
        assert_eq!(set.count(), 500);
        // Mutating a transient derived from a persistent collection must
        // preserve the source version despite the in-place fast path.
        let original = map.persistent().unwrap();
        let derived = TransientMap::new_from_map(&original);
        derived.assoc(Value::Long(501), Value::Long(-1)).unwrap();
        assert_eq!(original.get(&Value::Long(501)), Some(&Value::Long(502)));
        let original = set.persistent().unwrap();
        let derived = TransientSet::new_from_set(&original);
        derived.disj(&Value::Long(501)).unwrap();
        assert!(original.contains(&Value::Long(501)));
    }
}
