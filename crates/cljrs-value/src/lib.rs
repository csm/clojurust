#![allow(clippy::arc_with_non_send_sync)]
pub mod clone;
pub mod collections;
pub mod error;
pub mod hash;
pub mod intern;
pub mod jit_hooks;
pub mod keyword;
pub mod native_object;
pub mod parking;
#[cfg(not(feature = "no-gc"))]
pub mod publish;
pub mod realization;
pub mod regex;
pub mod resource;
pub mod shared;
pub mod symbol;
pub mod type_hint;
pub mod types;
pub mod value;

/// No-gc publication traces backing scopes without evaluating user code.
#[cfg(feature = "no-gc")]
pub mod publish {
    pub fn publish_value(value: crate::Value) -> crate::Value {
        if !cljrs_gc::alloc_ctx::invocation_is_active() {
            cljrs_gc::retain_regions_for(&value);
        }
        value
    }
    pub fn publish_for_owner(value: crate::Value, address: usize) -> crate::Value {
        if !cljrs_gc::alloc_ctx::invocation_is_active() {
            cljrs_gc::region::AllocationOwner::for_address(address).protect(&value);
        }
        value
    }
}

pub use collections::{
    PersistentArrayMap, PersistentHashMap, PersistentHashSet, PersistentList, PersistentQueue,
    PersistentVector, SortedMap, SortedSet,
};
pub use error::{ExceptionInfo, ValueError, ValueResult};
pub use hash::ClojureHash;
pub use intern::{intern_keyword, intern_symbol};
pub use jit_hooks::set_var_rebind_hook;
pub use keyword::Keyword;
pub use native_object::{NativeObject, NativeObjectBox, gc_native_object};
pub use resource::{Resource, ResourceHandle};
pub use shared::{ByteBlob, PromoteError, SharedAtom, SharedSlot, SharedValue, demote, promote};
pub use symbol::Symbol;
pub use type_hint::TypeHint;
pub use types::{
    Agent, Arity, Atom, BoundFn, CljxCons, CljxFn, CljxFnArity, CljxFuture, CljxPromise, Delay,
    DelayState, FUTURE_CANCELLED_MSG, FutureState, LazySeq, MultiFn, Namespace, NativeFn,
    NativeFnFunc, NativeFnPtr, Protocol, ProtocolFn, ProtocolMethod, ReferClojureFilter, Thunk,
    Var, Volatile, bump_protocol_generation, protocol_generation,
};
pub use value::{MapValue, ObjectArray, SetValue, TypeInstance, Value};

#[cfg(all(test, feature = "no-gc"))]
mod publication_review_tests {
    use super::*;
    struct Probe(std::rc::Rc<std::cell::Cell<bool>>);
    impl cljrs_gc::Trace for Probe {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    #[test]
    fn opaque_publication_preserves_identity_and_releases_unrelated_scopes() {
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        let outer = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let probe = cljrs_gc::GcPtr::new(Probe(dropped.clone()));
        let inner = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let array = cljrs_gc::GcPtr::new(ObjectArray(std::sync::Mutex::new(vec![Value::Long(1)])));
        let value = Value::Vector(cljrs_gc::GcPtr::new(PersistentVector::from_iter(vec![
            Value::ObjectArray(array.clone()),
        ])));
        let published = publish::publish_value(value);
        drop(inner);
        drop(probe);
        drop(outer);
        assert!(
            dropped.get(),
            "an unrelated region was permanently retained"
        );
        let Value::Vector(values) = published else {
            panic!("not a vector")
        };
        let Value::ObjectArray(received) = values.get().iter().next().unwrap() else {
            panic!("not an array")
        };
        assert!(cljrs_gc::GcPtr::ptr_eq(&array, received));
        array.get().0.lock().unwrap()[0] = Value::Long(42);
        assert_eq!(received.get().0.lock().unwrap()[0], Value::Long(42));
    }
}
