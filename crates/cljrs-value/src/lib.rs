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

/// No-gc publication copies immutable scratch data into the static arena.
#[cfg(feature = "no-gc")]
pub mod publish {
    fn copyable(value: &crate::clone::SerializedValue) -> bool {
        use crate::clone::SerializedValue::*;
        match value {
            Nil
            | Bool(_)
            | Long(_)
            | Double(_)
            | BigInt(_)
            | BigDecimal(_)
            | Ratio(_)
            | Char(_)
            | Str(_)
            | Uuid(_)
            | Pattern(_)
            | Symbol { .. }
            | Keyword { .. }
            | SharedAtom(_)
            | ByteBlob(_) => true,
            List(values) | Vector(values) | HashSet(values) | Queue(values) => {
                values.iter().all(copyable)
            }
            ArrayMap(entries)
            | HashMap(entries)
            | TypeInstance {
                fields: entries, ..
            } => entries
                .iter()
                .all(|(key, value)| copyable(key) && copyable(value)),
            Cons { head, tail } => copyable(head) && copyable(tail),
            WithMeta { value, meta } => copyable(value) && copyable(meta),
            Reduced(value) => copyable(value),
            // Mutable identities and custom sorted comparators stay intact.
            _ => false,
        }
    }

    pub fn publish_value(value: crate::Value) -> crate::Value {
        if cljrs_gc::alloc_ctx::invocation_is_active() {
            return value;
        }
        // Persistent sinks already evaluate their contents in a static context.
        // Opaque callables still need a capture walk even when their box is static.
        if crate::types::value_gcptr_is_static(&value)
            && !matches!(
                value,
                crate::Value::Fn(_)
                    | crate::Value::Macro(_)
                    | crate::Value::NativeFunction(_)
                    | crate::Value::BoundFn(_)
                    | crate::Value::WithMeta(..)
                    | crate::Value::Reduced(_)
            )
        {
            return value;
        }
        if let Ok(wire) = crate::clone::serialize(&value)
            && copyable(&wire)
        {
            let _static_context = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
            return crate::clone::deserialize(wire);
        }
        // Preserve identity for mutable/opaque values. Retain their actual
        // backing regions, rather than every region in the calling computation.
        cljrs_gc::retain_regions_for(&value);
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
