//! Garbage collector for clojurust.
//!
//! Phase 8 will replace this stub with a real tracing GC.  Until then,
//! `GcPtr<T>` is a thin `Arc<T>` wrapper with the same public API that
//! Phase 8 will expose, so callers need no changes when the real GC lands.

use std::sync::Arc;

// ─── Trace marker ─────────────────────────────────────────────────────────────

/// Marker trait for types that can be stored in a `GcPtr`.
///
/// Phase 8 will add a real `fn trace(&self, visitor: &mut dyn GcVisitor)`
/// method used by the collector to walk the object graph.  For now the
/// default no-op implementation satisfies the bound without requiring
/// boilerplate in every implementor.
pub trait Trace: Send + Sync {
    fn trace(&self) {}
}

// Blanket impls for common leaf types that need no tracing.
impl Trace for String {}
impl Trace for i64 {}
impl Trace for f64 {}
impl Trace for bool {}

// ─── GcPtr ────────────────────────────────────────────────────────────────────

/// A GC-managed smart pointer.
///
/// Currently an `Arc<T>` shim.  Phase 8 replaces the internals with a real
/// GC handle while keeping this public API identical.
pub struct GcPtr<T: ?Sized>(Arc<T>);

impl<T> GcPtr<T> {
    /// Allocate a new GC-managed value.
    ///
    /// Phase 8 will add a `T: Trace` bound here so the GC can walk the
    /// object graph.  For now `Arc<T>` has no such requirement.
    pub fn new(value: T) -> Self {
        GcPtr(Arc::new(value))
    }
}

impl<T: ?Sized> GcPtr<T> {
    /// Obtain a reference to the contained value.
    ///
    /// In Phase 8 this reference will be valid until the next GC safepoint;
    /// callers must not hold it across a potential allocation.
    pub fn get(&self) -> &T {
        &self.0
    }

    /// Identity comparison: true iff both pointers refer to the same allocation.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

impl<T: ?Sized> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        GcPtr(Arc::clone(&self.0))
    }
}

impl<T: ?Sized + std::fmt::Debug> std::fmt::Debug for GcPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

// Allow constructing a GcPtr<dyn Trait> from a GcPtr<T> where T: Trait.
impl<T: Trace + 'static> From<GcPtr<T>> for GcPtr<dyn Trace> {
    fn from(p: GcPtr<T>) -> Self {
        GcPtr(p.0 as Arc<dyn Trace>)
    }
}
