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
    use cljrs_gc::GcPtr;
    struct Probe(std::rc::Rc<std::cell::Cell<bool>>);
    impl cljrs_gc::Trace for Probe {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    #[derive(Debug)]
    struct Counted {
        traces: std::rc::Rc<std::cell::Cell<usize>>,
        dropped: std::rc::Rc<std::cell::Cell<bool>>,
    }
    impl cljrs_gc::Trace for Counted {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {
            self.traces.set(self.traces.get() + 1);
        }
    }
    impl NativeObject for Counted {
        fn type_tag(&self) -> &str {
            "Counted"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    impl Drop for Counted {
        fn drop(&mut self) {
            self.dropped.set(true);
        }
    }
    #[test]
    fn cached_cell_snapshots_do_not_retrace_or_permanently_pin_payloads() {
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let traces = std::rc::Rc::new(std::cell::Cell::new(0));
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let counted = Value::NativeObject(gc_native_object(Counted {
                traces: traces.clone(),
                dropped: dropped.clone(),
            }));
            let map = Value::Map(MapValue::from_pairs(
                (0..1000)
                    .map(|i| (Value::Long(i), counted.clone()))
                    .collect(),
            ));
            atom.get().reset(map);
        }
        let initial = traces.get();
        assert_eq!(initial, 1);
        for _ in 0..1000 {
            let scope = cljrs_gc::region::ReadScope::enter().unwrap();
            let snapshot = atom.get().snapshot();
            assert!(matches!(snapshot.value, Value::Map(_)));
            atom.get().deref();
            drop(scope);
        }
        assert_eq!(traces.get(), initial, "reads walked the full map graph");
        atom.get().reset(Value::Nil);
        assert!(
            dropped.get(),
            "top-level reads permanently pinned the old payload"
        );
    }

    #[test]
    fn stored_scalar_boxes_preserve_identity_and_release_with_the_last_alias() {
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let _temporary = GcPtr::new(Probe(dropped.clone()));
            let uuid = GcPtr::new(crate::value::UuidValue(42));
            let returned = atom.get().reset(Value::Uuid(uuid.clone()));
            let Value::Uuid(returned) = returned else {
                unreachable!()
            };
            assert!(!GcPtr::ptr_eq(&uuid, &returned));
            assert!(GcPtr::identity_eq(&uuid, &returned));
        }
        assert!(
            dropped.get(),
            "a scalar kept unrelated source temporaries alive"
        );
        let snapshot = atom.get().snapshot();
        let Value::Uuid(uuid) = &snapshot.value else {
            unreachable!()
        };
        assert_eq!(uuid.get().0, 42);
        atom.get().reset(Value::Nil);
        assert_eq!(uuid.get().0, 42);
        drop(snapshot);
        assert!(dropped.get());
    }

    #[test]
    fn returned_updates_protect_shared_children_after_the_cell_is_replaced() {
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let object = Value::NativeObject(gc_native_object(Counted {
                traces: Default::default(),
                dropped: dropped.clone(),
            }));
            atom.get().reset(Value::Map(MapValue::from_pairs(vec![(
                Value::Long(0),
                object,
            )])));
        }
        {
            let _consumer = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let previous = atom.get().snapshot();
            let Value::Map(old) = &previous.value else {
                unreachable!()
            };
            let returned = atom
                .get()
                .reset(Value::Map(old.assoc(Value::Long(1), Value::Long(2))));
            drop(previous);
            atom.get().reset(Value::Nil);
            assert!(
                !dropped.get(),
                "replacing the cell freed a child of its returned value"
            );
            let Value::Map(returned) = returned else {
                unreachable!()
            };
            let Value::NativeObject(child) = returned.get(&Value::Long(0)).unwrap() else {
                unreachable!()
            };
            assert_eq!(child.get().type_tag(), "Counted");
        }
        assert!(dropped.get());
    }

    #[test]
    fn metadata_validators_and_watches_release_replaced_callbacks() {
        let _container = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let atom = GcPtr::new(Atom::new(Value::Nil));
        for _ in 0..50 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let callback = Value::NativeFunction(GcPtr::new(NativeFn::with_captures(
                    "owned-callback",
                    Arity::Fixed(1),
                    vec![Value::NativeObject(gc_native_object(Counted {
                        traces: Default::default(),
                        dropped: dropped.clone(),
                    }))],
                    |captures, _| Ok(captures[0].clone()),
                )));
                atom.get().set_validator(Some(callback.clone()));
                atom.get().set_meta(Some(callback.clone()));
                atom.get().add_watch(Value::Long(1), callback);
            }
            assert!(!dropped.get());
            {
                let _reader = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let validator = atom.get().get_validator().unwrap();
                assert!(matches!(validator, Value::NativeFunction(_)));
                assert!(atom.get().get_meta().is_some());
                atom.get().set_validator(None);
                atom.get().set_meta(None);
                atom.get().remove_watch(&Value::Long(1));
                assert!(
                    !dropped.get(),
                    "a borrowed callback was freed during replacement"
                );
                let Value::NativeFunction(f) = validator else {
                    unreachable!()
                };
                assert!(matches!(
                    (f.get().func)(&[Value::Nil]).unwrap(),
                    Value::NativeObject(_)
                ));
            }
            assert!(
                dropped.get(),
                "removed auxiliary slots retained their source"
            );
        }
    }

    #[test]
    fn durable_stores_outlive_an_active_read_scope() {
        std::thread::spawn(|| {
            let destination = cljrs_gc::region::AllocationSnapshot::capture();
            let read = cljrs_gc::region::ReadScope::enter().unwrap();
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            let value = {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let value = Value::NativeObject(gc_native_object(Counted {
                    traces: Default::default(),
                    dropped: dropped.clone(),
                }));
                destination.protect(&value);
                value
            };
            drop(read);
            assert!(
                !dropped.get(),
                "a store was shortened to the reader lifetime"
            );
            let Value::NativeObject(value) = value else {
                unreachable!()
            };
            assert_eq!(value.get().type_tag(), "Counted");
        })
        .join()
        .unwrap();
    }

    #[test]
    fn var_rebinding_releases_opaque_roots_after_the_last_reader() {
        let var = GcPtr::new(Var::new("user", "root"));
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let _temporary = GcPtr::new(Probe(dropped.clone()));
                var.get()
                    .bind(Value::NativeFunction(GcPtr::new(NativeFn::with_captures(
                        "root",
                        Arity::Fixed(0),
                        vec![Value::string("alive")],
                        |captures, _| Ok(captures[0].clone()),
                    ))));
            }
            let scope = cljrs_gc::region::ReadScope::enter().unwrap();
            let alias = var.get().deref().unwrap();
            var.get().bind(Value::Nil);
            assert!(!dropped.get());
            let Value::NativeFunction(f) = alias else {
                unreachable!()
            };
            assert_eq!((f.get().func)(&[]).unwrap().to_string(), "\"alive\"");
            drop(scope);
            assert!(dropped.get(), "an obsolete Var root remained permanent");
        }
    }

    #[test]
    fn unscoped_reads_require_an_owning_snapshot_without_pinning_sources() {
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let var = GcPtr::new(Var::new("user", "scoped"));
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let value = Value::NativeObject(gc_native_object(Counted {
                traces: Default::default(),
                dropped: dropped.clone(),
            }));
            atom.get().reset(value.clone());
            var.get().bind(value);
        }
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| atom.get().deref())).is_err()
        );
        let snapshot = var.get().snapshot().unwrap();
        atom.get().reset(Value::Nil);
        var.get().bind(Value::Nil);
        assert!(!dropped.get());
        assert!(matches!(snapshot.value, Value::NativeObject(_)));
        drop(snapshot);
        assert!(
            dropped.get(),
            "an unscoped read permanently pinned the payload"
        );
    }

    #[test]
    fn indexed_storage_releases_overwritten_sources_and_failed_deliveries() {
        let _container = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let array = GcPtr::new(ObjectArray::new(vec![Value::Nil; 2]));
        let mut previous: Option<std::rc::Rc<std::cell::Cell<bool>>> = None;
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let value = Value::NativeObject(gc_native_object(Counted {
                    traces: Default::default(),
                    dropped: dropped.clone(),
                }));
                let value = publish::publish_for_slot(value, array.get() as *const _ as usize, 0);
                array.get().0.lock().unwrap()[0] = value;
            }
            if let Some(previous) = previous {
                assert!(previous.get());
            }
            previous = Some(dropped);
        }
        let value = publish::publish_for_slot(Value::Nil, array.get() as *const _ as usize, 0);
        array.get().0.lock().unwrap()[0] = value;
        assert!(previous.unwrap().get());
        let promise = CljxPromise::new();
        promise.deliver(Value::Long(1));
        let discarded = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            promise.deliver(Value::NativeObject(gc_native_object(Counted {
                traces: Default::default(),
                dropped: discarded.clone(),
            })));
        }
        assert!(
            discarded.get(),
            "an unsuccessful promise delivery retained its candidate"
        );
        assert_eq!(promise.deref_blocking(), Value::Long(1));
    }

    #[test]
    fn regional_cells_release_each_replaced_payload_without_changing_identity() {
        let _container = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let other = GcPtr::new(Atom::new(Value::Nil));
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let _temporary = GcPtr::new(Probe(dropped.clone()));
                let vector = GcPtr::new(PersistentVector::from_iter([Value::Long(1)]));
                let returned = atom.get().reset(Value::Vector(vector.clone()));
                let Value::Vector(returned) = returned else {
                    unreachable!()
                };
                assert!(GcPtr::ptr_eq(&vector, &returned));
                other.get().reset(Value::Vector(vector));
            }
            assert!(!dropped.get());
            atom.get().reset(Value::Nil);
            assert!(!dropped.get(), "another slot still owns the same source");
            other.get().reset(Value::Nil);
            assert!(
                dropped.get(),
                "obsolete regional payload kept its source alive"
            );
        }
    }

    #[test]
    fn replacement_does_not_inherit_the_previous_payloads_read_lease() {
        let _container = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let mut previous: Option<std::rc::Rc<std::cell::Cell<bool>>> = None;
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _iteration = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let _old = atom.get().deref();
                atom.get()
                    .reset(Value::NativeObject(gc_native_object(Counted {
                        traces: Default::default(),
                        dropped: dropped.clone(),
                    })));
            }
            if let Some(old) = previous {
                assert!(
                    old.get(),
                    "a read became a durable dependency of the replacement"
                );
            }
            previous = Some(dropped);
        }
        atom.get().reset(Value::Nil);
        assert!(previous.unwrap().get());
    }

    #[test]
    fn static_cells_release_replaced_opaque_payloads_and_preserve_borrowed_aliases() {
        let atom = GcPtr::new(Atom::new(Value::Nil));
        let volatile = GcPtr::new(Volatile::new(Value::Nil));
        for _ in 0..100 {
            let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _scope = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let _temporary = GcPtr::new(Probe(dropped.clone()));
                let payload = Value::NativeFunction(GcPtr::new(NativeFn::with_captures(
                    "payload",
                    Arity::Fixed(0),
                    vec![Value::string("alive")],
                    |captures, _| Ok(captures[0].clone()),
                )));
                atom.get().reset(payload.clone());
                volatile.get().reset(payload);
            }
            assert!(!dropped.get());
            {
                let _consumer = cljrs_gc::alloc_ctx::ScratchGuard::new();
                let alias = atom.get().deref();
                atom.get().reset(Value::Nil);
                volatile.get().reset(Value::Nil);
                assert!(!dropped.get(), "a live alias lost its backing scope");
                let Value::NativeFunction(f) = alias else {
                    unreachable!()
                };
                assert_eq!((f.get().func)(&[]).unwrap().to_string(), "\"alive\"");
            }
            assert!(
                dropped.get(),
                "an obsolete opaque payload retained its scope"
            );
        }
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _scope = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let _temporary = GcPtr::new(Probe(dropped.clone()));
            let rejected =
                Value::ObjectArray(GcPtr::new(ObjectArray(std::sync::Mutex::new(vec![]))));
            assert!(!atom.get().compare_and_set(&Value::Long(1), rejected));
        }
        assert!(dropped.get(), "a failed CAS retained its candidate");
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
