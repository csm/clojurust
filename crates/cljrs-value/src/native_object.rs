//! The `NativeObject` trait for opaque Rust structs exposed as Clojure values.
//!
//! Unlike `Resource` (which uses `Arc` for deterministic `Drop`), native objects
//! live behind `GcPtr` and participate in tracing GC.  They are intended for
//! Rust values whose lifetime is managed by the garbage collector.

use std::any::Any;
use std::fmt;

use cljrs_gc::{GcPtr, MarkVisitor, Trace};

/// An opaque Rust value that can be wrapped as a Clojure `Value::NativeObject`.
///
/// Implementors must be `Send + Sync` (shared across threads), `Debug`
/// (for display), and `Trace` (so the GC can walk any `GcPtr`/`Value`
/// references held inside).
///
/// # Example
///
/// ```ignore
/// struct Counter { n: AtomicI64 }
///
/// impl NativeObject for Counter {
///     fn type_tag(&self) -> &str { "Counter" }
///     fn as_any(&self) -> &dyn Any { self }
/// }
///
/// // Trace is a no-op if the struct holds no GcPtr/Value fields.
/// impl Trace for Counter {
///     fn trace(&self, _: &mut MarkVisitor) {}
/// }
/// ```
pub trait NativeObject: Send + Sync + fmt::Debug + Trace + 'static {
    /// Short type name used for `type_tag_of` and protocol dispatch.
    ///
    /// Convention: use the Rust struct name (e.g. `"TcpStream"`, `"Counter"`).
    /// This string is what you pass to `extend-type` on the Clojure side.
    fn type_tag(&self) -> &str;

    /// Downcast support — native functions that know the concrete type can
    /// use `obj.as_any().downcast_ref::<MyType>()` to access it.
    fn as_any(&self) -> &dyn Any;
}

/// A type-erased wrapper around `Box<dyn NativeObject>`.
///
/// This is what `GcPtr` actually points to inside `Value::NativeObject`.
/// The wrapper exists because `GcPtr<dyn NativeObject>` would require
/// unsized coercions; a concrete struct is simpler.
#[derive(Debug)]
pub struct NativeObjectBox {
    inner: Box<dyn NativeObject>,
}

impl NativeObjectBox {
    /// Wrap a concrete `NativeObject` implementor.
    pub fn new(obj: impl NativeObject) -> Self {
        Self {
            inner: Box::new(obj),
        }
    }

    /// The type tag for protocol dispatch.
    pub fn type_tag(&self) -> &str {
        self.inner.type_tag()
    }

    /// Downcast to a concrete type.
    pub fn downcast_ref<T: NativeObject>(&self) -> Option<&T> {
        self.inner.as_any().downcast_ref::<T>()
    }

    /// Access the inner trait object.
    pub fn inner(&self) -> &dyn NativeObject {
        &*self.inner
    }
}

impl Trace for NativeObjectBox {
    fn trace(&self, visitor: &mut MarkVisitor) {
        self.inner.trace(visitor);
    }
}

/// Convenience: allocate a `NativeObject` on the GC heap.
pub fn gc_native_object(obj: impl NativeObject) -> GcPtr<NativeObjectBox> {
    GcPtr::new(NativeObjectBox::new(obj))
}
