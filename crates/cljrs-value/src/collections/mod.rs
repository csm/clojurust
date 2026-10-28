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

// A stable, traced snapshot while a collection operation realizes user code.
// no-gc needs no extra allocation: its active regions stay with the fiber.
pub(crate) struct Snapshot<T: cljrs_gc::Trace + 'static> {
    #[cfg(not(feature = "no-gc"))]
    data: cljrs_gc::GcPtr<T>,
    #[cfg(not(feature = "no-gc"))]
    _root: cljrs_gc::AllocRootGuard,
    #[cfg(feature = "no-gc")]
    data: T,
}
impl<T: cljrs_gc::Trace + 'static> Snapshot<T> {
    pub(crate) fn new(data: T) -> Self {
        #[cfg(not(feature = "no-gc"))]
        {
            let root = cljrs_gc::push_alloc_frame();
            Self {
                data: cljrs_gc::GcPtr::new(data),
                _root: root,
            }
        }
        #[cfg(feature = "no-gc")]
        {
            Self { data }
        }
    }
    pub(crate) fn get(&self) -> &T {
        #[cfg(not(feature = "no-gc"))]
        {
            self.data.get()
        }
        #[cfg(feature = "no-gc")]
        {
            &self.data
        }
    }
}
