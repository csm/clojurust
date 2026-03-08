//! Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust.
//!
//! Every heap allocation goes through [`GcPtr::new`], which registers the
//! object in the global [`HEAP`].  Memory is freed only during
//! [`GcHeap::collect`]; [`GcPtr::drop`] is a no-op.
//!
//! # Usage
//! ```ignore
//! // Allocate.
//! let p: GcPtr<MyType> = GcPtr::new(MyType::new());
//!
//! // Collect: pass a closure that traces all live roots.
//! cljx_gc::HEAP.collect(|visitor| {
//!     visitor.visit(&root_ptr);
//!     // … visit every other live GcPtr …
//! });
//! ```
//!
//! # Safety contract
//! * `collect` must only be called when no other thread holds or is creating
//!   `GcPtr` values (stop-the-world).
//! * Every live `GcPtr` reachable from the program must be passed to
//!   `visitor.visit` during collection or it will be freed.

#![allow(clippy::missing_safety_doc)]

use std::cell::Cell;
use std::ptr::NonNull;
use std::sync::Mutex;

// ── GcPtr forward declaration ─────────────────────────────────────────────────

// (defined below; we need it to appear in trait signatures)
pub struct GcPtr<T: Trace + 'static>(NonNull<GcBox<T>>);

// ── Trace trait ───────────────────────────────────────────────────────────────

/// Implemented by every type that can be stored behind a [`GcPtr`].
///
/// `trace` must call `visitor.visit(ptr)` for every `GcPtr<_>` directly or
/// indirectly reachable from `self` (including through `Arc`, `Mutex`, etc.).
pub trait Trace: Send + Sync {
    fn trace(&self, visitor: &mut MarkVisitor);
}

// ── Leaf impls for primitives / stdlib types ──────────────────────────────────

impl Trace for String {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for i64 {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for f64 {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for bool {
    fn trace(&self, _: &mut MarkVisitor) {}
}

// Numeric tower types (no GcPtr children).
impl Trace for num_bigint::BigInt {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for bigdecimal::BigDecimal {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for num_rational::Ratio<num_bigint::BigInt> {
    fn trace(&self, _: &mut MarkVisitor) {}
}

// ── GcVisitor convenience trait ───────────────────────────────────────────────

/// Provides typed `visit<T>` sugar over [`MarkVisitor`].
///
/// Implemented by [`MarkVisitor`].  Call `visitor.visit(&ptr)` from within
/// [`Trace::trace`] for every `GcPtr` field.
pub trait GcVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>);
}

// ── GcBoxHeader ───────────────────────────────────────────────────────────────

/// Header prepended to every GC allocation.
///
/// **Layout**: must be the first field of [`GcBox<T>`] (`#[repr(C)]`).
/// The `trace_fn` and `drop_fn` pointers recover the concrete `T` by casting
/// from a `*const GcBoxHeader` to `*const GcBox<T>`.
#[repr(C)]
struct GcBoxHeader {
    /// `true` iff this object was reached during the current mark phase.
    marked: Cell<bool>,
    /// Intrusive singly-linked list: next allocation in [`GcHeapInner::head`].
    next: Cell<*mut GcBoxHeader>,
    /// Type-erased: calls `T::trace` on the enclosing `GcBox<T>`.
    trace_fn: unsafe fn(*const GcBoxHeader, &mut MarkVisitor),
    /// Type-erased: drops the enclosing `GcBox<T>` and frees its memory.
    drop_fn: unsafe fn(*mut GcBoxHeader),
}

// SAFETY: accessed only under heap lock (`next`, `drop`) or during the
// single-threaded mark phase (`marked`, `trace_fn`).
// `Cell` is `!Sync` but our GC protocol guarantees exclusive access.
unsafe impl Send for GcBoxHeader {}
unsafe impl Sync for GcBoxHeader {}

// ── GcBox<T> ─────────────────────────────────────────────────────────────────

#[repr(C)]
struct GcBox<T: Trace + 'static> {
    header: GcBoxHeader,
    value: T,
}

unsafe fn trace_gc_box<T: Trace + 'static>(header: *const GcBoxHeader, visitor: &mut MarkVisitor) {
    // SAFETY: `header` is the first field of `GcBox<T>` (#[repr(C)]).
    unsafe {
        let gc_box = header as *const GcBox<T>;
        (*gc_box).value.trace(visitor);
    }
}

unsafe fn drop_gc_box<T: Trace + 'static>(header: *mut GcBoxHeader) {
    // SAFETY: same cast; `Box::from_raw` takes ownership and runs Drop.
    unsafe {
        let gc_box = header as *mut GcBox<T>;
        drop(Box::from_raw(gc_box));
    }
}

// ── GcHeapInner ───────────────────────────────────────────────────────────────

struct GcHeapInner {
    head: *mut GcBoxHeader,
    count: usize,
    total_allocated: usize,
    total_freed: usize,
}

// SAFETY: protected by the outer `Mutex`.
unsafe impl Send for GcHeapInner {}

impl GcHeapInner {
    const fn new() -> Self {
        Self {
            head: std::ptr::null_mut(),
            count: 0,
            total_allocated: 0,
            total_freed: 0,
        }
    }
}

// ── GcHeap ────────────────────────────────────────────────────────────────────

/// The global GC heap: allocates and collects GC-managed objects.
pub struct GcHeap {
    inner: Mutex<GcHeapInner>,
}

// SAFETY: `Mutex<GcHeapInner>` is `Sync` because `GcHeapInner: Send`.
unsafe impl Sync for GcHeap {}

impl Default for GcHeap {
    fn default() -> Self {
        Self::new()
    }
}

impl GcHeap {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(GcHeapInner::new()),
        }
    }

    /// Allocate a new GC-managed value and register it in the heap.
    pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T> {
        let gc_box = Box::new(GcBox {
            header: GcBoxHeader {
                marked: Cell::new(false),
                next: Cell::new(std::ptr::null_mut()),
                trace_fn: trace_gc_box::<T>,
                drop_fn: drop_gc_box::<T>,
            },
            value,
        });
        let raw: *mut GcBox<T> = Box::into_raw(gc_box);
        {
            let mut inner = self.inner.lock().unwrap();
            // SAFETY: `raw` is non-null and freshly owned.
            // `GcBox<T>` is `#[repr(C)]` with `header` first, so
            // `raw as *mut GcBoxHeader` is a valid pointer to the header.
            unsafe {
                (*raw).header.next.set(inner.head);
                inner.head = raw as *mut GcBoxHeader;
            }
            inner.count += 1;
            inner.total_allocated += 1;
        }
        // SAFETY: `raw` is non-null (from Box).
        GcPtr(unsafe { NonNull::new_unchecked(raw) })
    }

    /// Mark all objects reachable from `trace_roots`, then sweep unreachable.
    ///
    /// # Safety
    /// Must only be called when no other thread is creating or dereferencing
    /// `GcPtr` values.  `trace_roots` must visit every live root.
    pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, trace_roots: F) {
        // Mark phase: populate grey set from roots, then drain it.
        let mut visitor = MarkVisitor::new();
        trace_roots(&mut visitor);
        visitor.drain();

        // Sweep phase: partition into live and dead, then free dead objects.
        let mut inner = self.inner.lock().unwrap();
        let mut live: Vec<*mut GcBoxHeader> = Vec::with_capacity(inner.count);
        let mut dead: Vec<*mut GcBoxHeader> = Vec::new();

        let mut current = inner.head;
        while !current.is_null() {
            // SAFETY: every pointer in our list is a valid `GcBoxHeader`.
            let header = unsafe { &*current };
            let next = header.next.get();
            if header.marked.get() {
                header.marked.set(false); // reset for next collection
                live.push(current);
            } else {
                dead.push(current);
            }
            current = next;
        }

        // Free unreachable objects.
        for ptr in dead {
            let header = unsafe { &*ptr };
            unsafe { (header.drop_fn)(ptr) };
            inner.count -= 1;
            inner.total_freed += 1;
        }

        // Rebuild linked list from surviving objects.
        inner.head = std::ptr::null_mut();
        for ptr in live {
            let header = unsafe { &*ptr };
            header.next.set(inner.head);
            inner.head = ptr;
        }
    }

    /// Number of currently live GC allocations.
    pub fn count(&self) -> usize {
        self.inner.lock().unwrap().count
    }

    /// Total allocations made since startup.
    pub fn total_allocated(&self) -> usize {
        self.inner.lock().unwrap().total_allocated
    }

    /// Total objects freed by collection since startup.
    pub fn total_freed(&self) -> usize {
        self.inner.lock().unwrap().total_freed
    }
}

// ── MarkVisitor ───────────────────────────────────────────────────────────────

/// Marks GC objects reachable from roots during a collection.
///
/// Uses a grey stack to avoid recursion stack overflow on deep structures.
/// Add objects as grey via [`GcVisitor::visit`]; then call [`drain`] to
/// process all pending objects.
pub struct MarkVisitor {
    grey: Vec<*mut GcBoxHeader>,
}

// SAFETY: raw pointers are only used during stop-the-world collection.
unsafe impl Send for MarkVisitor {}
unsafe impl Sync for MarkVisitor {}

impl MarkVisitor {
    fn new() -> Self {
        Self { grey: Vec::new() }
    }

    /// Process all grey objects (their children are discovered and added to
    /// grey), repeating until the grey set is empty.
    fn drain(&mut self) {
        while let Some(header) = self.grey.pop() {
            // SAFETY: grey objects are always valid live allocations.
            let h = unsafe { &*header };
            unsafe { (h.trace_fn)(header as *const GcBoxHeader, self) };
        }
    }
}

impl GcVisitor for MarkVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>) {
        // SAFETY: `GcPtr` is always a valid live pointer (stop-the-world).
        let header = unsafe { &(*ptr.0.as_ptr()).header };
        if !header.marked.get() {
            header.marked.set(true);
            self.grey.push(ptr.0.as_ptr() as *mut GcBoxHeader);
        }
    }
}

// ── Global heap singleton ─────────────────────────────────────────────────────

/// The global GC heap.  All `GcPtr::new` calls allocate here.
pub static HEAP: GcHeap = GcHeap::new();

// ── GcPtr ─────────────────────────────────────────────────────────────────────

// (struct declared at top for trait signature availability)

// SAFETY: `T: Trace: Send + Sync`.  `GcBoxHeader` internals are accessed only
// under the heap lock or during stop-the-world marking.
unsafe impl<T: Trace + 'static> Send for GcPtr<T> {}
unsafe impl<T: Trace + 'static> Sync for GcPtr<T> {}

impl<T: Trace + 'static> GcPtr<T> {
    /// Allocate a new GC-managed value.
    pub fn new(value: T) -> Self {
        HEAP.alloc(value)
    }

    /// Borrow the contained value.
    ///
    /// The reference is valid as long as no `collect()` runs and frees this
    /// object.  Never hold it across a GC safepoint.
    pub fn get(&self) -> &T {
        // SAFETY: valid live pointer (stop-the-world invariant).
        unsafe { &(*self.0.as_ptr()).value }
    }

    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut (*self.0.as_ptr()).value }
    }

    /// Identity comparison: `true` iff both pointers point to the same object.
    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        a.0 == b.0
    }
}

/// O(1): copies the raw pointer without touching the heap.
impl<T: Trace + 'static> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        GcPtr(self.0)
    }
}

impl<T: Trace + 'static + std::fmt::Debug> std::fmt::Debug for GcPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SAFETY: valid live pointer.
        unsafe { (*self.0.as_ptr()).value.fmt(f) }
    }
}

/// Drop is intentionally a no-op: the GC heap owns all memory.
impl<T: Trace + 'static> Drop for GcPtr<T> {
    fn drop(&mut self) {}
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    // A Trace type that records when it is dropped.
    #[derive(Debug)]
    struct Tracked {
        value: i32,
        dropped: Arc<Mutex<bool>>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            *self.dropped.lock().unwrap() = true;
        }
    }

    impl Trace for Tracked {
        fn trace(&self, _: &mut MarkVisitor) {}
    }

    // A Trace type that holds a child GcPtr.
    #[derive(Debug)]
    struct Parent {
        child: GcPtr<Tracked>,
    }

    impl Trace for Parent {
        fn trace(&self, visitor: &mut MarkVisitor) {
            visitor.visit(&self.child);
        }
    }

    fn fresh_heap() -> GcHeap {
        GcHeap::new()
    }

    #[test]
    fn alloc_and_get() {
        let heap = fresh_heap();
        let p = heap.alloc(42i64);
        assert_eq!(*p.get(), 42);
        assert_eq!(heap.count(), 1);
    }

    #[test]
    fn clone_is_same_ptr() {
        let heap = fresh_heap();
        let p = heap.alloc(99i64);
        let q = p.clone();
        assert!(GcPtr::ptr_eq(&p, &q));
    }

    #[test]
    fn collect_frees_unreachable() {
        let heap = fresh_heap();
        let dropped = Arc::new(Mutex::new(false));
        let _p = heap.alloc(Tracked {
            value: 1,
            dropped: dropped.clone(),
        });
        assert_eq!(heap.count(), 1);
        heap.collect(|_| {});
        assert_eq!(heap.count(), 0);
        assert!(*dropped.lock().unwrap(), "object should have been dropped");
    }

    #[test]
    fn collect_keeps_reachable() {
        let heap = fresh_heap();
        let dropped = Arc::new(Mutex::new(false));
        let p = heap.alloc(Tracked {
            value: 2,
            dropped: dropped.clone(),
        });
        heap.collect(|vis| vis.visit(&p));
        assert_eq!(heap.count(), 1);
        assert!(!*dropped.lock().unwrap(), "reachable object must survive");
    }

    #[test]
    fn collect_traces_children() {
        let heap = fresh_heap();
        let child_dropped = Arc::new(Mutex::new(false));
        let child = heap.alloc(Tracked {
            value: 10,
            dropped: child_dropped.clone(),
        });
        let parent = heap.alloc(Parent {
            child: child.clone(),
        });
        assert_eq!(heap.count(), 2);
        // Trace only the parent root; child must survive via Parent::trace.
        heap.collect(|vis| vis.visit(&parent));
        assert_eq!(heap.count(), 2);
        assert!(!*child_dropped.lock().unwrap());
    }

    #[test]
    fn collect_frees_two_unreachable() {
        let heap = fresh_heap();
        let d1 = Arc::new(Mutex::new(false));
        let d2 = Arc::new(Mutex::new(false));
        let _a = heap.alloc(Tracked {
            value: 1,
            dropped: d1.clone(),
        });
        let _b = heap.alloc(Tracked {
            value: 2,
            dropped: d2.clone(),
        });
        heap.collect(|_| {});
        assert!(*d1.lock().unwrap());
        assert!(*d2.lock().unwrap());
        assert_eq!(heap.count(), 0);
    }

    #[test]
    fn total_stats() {
        let heap = fresh_heap();
        let p = heap.alloc(1i64);
        let _q = heap.alloc(2i64);
        assert_eq!(heap.total_allocated(), 2);
        heap.collect(|vis| vis.visit(&p));
        assert_eq!(heap.count(), 1);
        assert_eq!(heap.total_freed(), 1);
    }
}

// Impls for vectors (backs "arrays" in clojure).

impl Trace for std::sync::Mutex<Vec<i32>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<i64>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<i16>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<i8>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<char>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<f64>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<f32>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}

impl Trace for std::sync::Mutex<Vec<bool>> {
    fn trace(&self, _visitor: &mut MarkVisitor) {}
}