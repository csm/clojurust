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
pub mod publish;
pub mod realization;
pub mod regex;
pub mod resource;
pub mod shared;
pub mod symbol;
pub mod type_hint;
pub mod types;
pub mod value;

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
    fn static_publication_copies_finite_data_and_releases_temporary_storage() {
        let atom = cljrs_gc::GcPtr::new(Atom::new(Value::Nil));
        // A captured allocation destination can itself have been published.
        let permanent = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let identity = Value::ObjectArray(cljrs_gc::GcPtr::new(ObjectArray(
            std::sync::Mutex::new(vec![]),
        )));
        publish::publish_value(identity);
        assert!(cljrs_gc::alloc_ctx::is_static_context());
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            let mut scope = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let _temporary = cljrs_gc::GcPtr::new(Probe(dropped.clone()));
            let value = Value::Vector(cljrs_gc::GcPtr::new(PersistentVector::from_iter([
                Value::Str(cljrs_gc::GcPtr::new("published".to_string())),
            ])));
            atom.get().reset(value.clone());
            scope.pop_for_return();
            let returned = publish::publish_return(value);
            drop(scope);
            assert!(
                dropped.get(),
                "a static publication retained unrelated temporaries"
            );
            assert_eq!(returned.to_string(), "[\"published\"]");
            assert_eq!(atom.get().deref().to_string(), "[\"published\"]");
        }
        drop(permanent);
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
