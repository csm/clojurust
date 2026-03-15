//! The `Resource` trait for I/O handles and other closeable resources.
//!
//! Resources are ref-counted with `Arc` (NOT `GcPtr`) so that they get
//! deterministic cleanup when the last reference drops.  The GC has no
//! finalizers, so `GcPtr` would leak file descriptors.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::error::ValueResult;

/// A closeable, ref-counted resource (file handle, socket, etc.).
///
/// All methods take `&self` because the inner state is behind a `Mutex`.
pub trait Resource: Send + Sync + fmt::Debug + 'static {
    /// Close the resource.  Idempotent — calling on an already-closed
    /// resource is a no-op.
    fn close(&self) -> ValueResult<()>;

    /// Whether this resource has been closed.
    fn is_closed(&self) -> bool;

    /// A short tag for display, e.g. `"reader"`, `"writer"`.
    fn resource_type(&self) -> &'static str;

    /// Downcast support so native fns can get the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// A wrapper around `Arc<dyn Resource>` that provides `Clone` and `Debug`.
#[derive(Clone)]
pub struct ResourceHandle(pub Arc<dyn Resource>);

impl fmt::Debug for ResourceHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Resource({})", self.0.resource_type())
    }
}

impl ResourceHandle {
    pub fn new(r: impl Resource) -> Self {
        Self(Arc::new(r))
    }

    pub fn close(&self) -> ValueResult<()> {
        self.0.close()
    }

    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }

    pub fn resource_type(&self) -> &'static str {
        self.0.resource_type()
    }

    /// Attempt to downcast to a concrete resource type.
    pub fn downcast<T: Resource>(&self) -> Option<&T> {
        self.0.as_any().downcast_ref::<T>()
    }
}
