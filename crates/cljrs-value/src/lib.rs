#![allow(clippy::arc_with_non_send_sync)]
pub mod clone;
pub mod collections;
pub mod error;
pub mod hash;
pub mod intern;
pub mod jit_hooks;
pub mod keyword;
pub mod native_object;
#[cfg(not(feature = "no-gc"))]
pub mod publish;
pub mod regex;
pub mod resource;
pub mod shared;
pub mod symbol;
pub mod types;
pub mod value;

/// `no-gc` builds keep their `StaticCtxGuard` discipline; the publish barrier
/// is an identity function there.
#[cfg(feature = "no-gc")]
pub mod publish {
    pub fn publish_value(v: crate::Value) -> crate::Value {
        v
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
pub use shared::{PromoteError, SharedAtom, SharedValue, demote, promote};
pub use symbol::Symbol;
pub use types::{
    Agent, Arity, Atom, BoundFn, CljxCons, CljxFn, CljxFnArity, CljxFuture, CljxPromise, Delay,
    DelayState, FutureState, LazySeq, MultiFn, Namespace, NativeFn, NativeFnFunc, NativeFnPtr,
    Protocol, ProtocolFn, ProtocolMethod, Thunk, Var, Volatile,
};
pub use value::{MapValue, ObjectArray, SetValue, TypeInstance, Value};
