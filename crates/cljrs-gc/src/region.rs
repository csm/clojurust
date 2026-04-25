//! Region (arena/bump) allocator for short-lived GC objects.
//!
//! A [`Region`] allocates objects via fast bump-pointer allocation without
//! touching the global GC heap's mutex or linked list.  Objects allocated in a
//! region are dropped in bulk when the region is reset or dropped — there is no
//! per-object deallocation.
//!
//! # Safety
//!
//! The caller **must** ensure that no [`GcPtr`] to a region-allocated object
//! outlives the [`Region`].  The GC will never collect region objects (they are
//! not in the heap list).  Accessing a region-allocated pointer after the region
//! is dropped is undefined behaviour.
//!
//! A [`RegionGuard`] provides RAII-based activation of a thread-local region
//! that can be queried by allocation-site code.

use std::alloc::{self, Layout};
use std::cell::RefCell;
use std::ptr::{self, NonNull};

#[cfg(not(feature = "no-gc"))]
use crate::gc_header::GcBoxHeader;
use crate::{GcBox, GcPtr, Trace};

// ── Constants ───────────────────────────────────────────────────────────────

/// Default chunk size (4 KiB).  Chunks grow if a single allocation is larger.
const DEFAULT_CHUNK_SIZE: usize = 4096;

// ── Internal: chunk of raw memory ───────────────────────────────────────────

struct Chunk {
    data: NonNull<u8>,
    layout: Layout,
}

impl Chunk {
    fn new(size: usize, align: usize) -> Self {
        let layout =
            Layout::from_size_align(size, align.max(16)).expect("Region: invalid chunk layout");
        // SAFETY: layout has non-zero size.
        let data =
            unsafe { NonNull::new(alloc::alloc(layout)).expect("Region: chunk allocation failed") };
        Self { data, layout }
    }
}

// ── Internal: drop entry ────────────────────────────────────────────────────

/// Type-erased destructor entry.  Runs `drop_in_place` on the GcBox without
/// freeing memory (the region owns the backing storage).
struct DropEntry {
    ptr: *mut u8,
    drop_fn: unsafe fn(*mut u8),
}

/// Drop the *value* inside a `GcBox<T>` in place, without freeing memory.
///
/// # Safety
/// `ptr` must point to a valid, initialised `GcBox<T>`.
unsafe fn drop_gcbox_in_place<T: Trace + 'static>(ptr: *mut u8) {
    unsafe { ptr::drop_in_place(ptr as *mut GcBox<T>) };
}

// ── Region ──────────────────────────────────────────────────────────────────

/// A bump allocator that produces [`GcPtr`]-compatible objects.
///
/// All memory is freed in bulk on [`reset`](Region::reset) or [`drop`].
pub struct Region {
    /// Allocated chunks, oldest first.
    chunks: Vec<Chunk>,
    /// Current bump pointer (byte offset into the active chunk).
    ptr: usize,
    /// End of the active chunk.
    end: usize,
    /// Drop entries, in allocation order.
    drops: Vec<DropEntry>,
    /// Cumulative bytes consumed by objects (excludes alignment padding).
    bytes_used: usize,
    /// Number of objects allocated.
    object_count: usize,
}

impl Region {
    /// Create a new region with the default chunk size (4 KiB).
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_CHUNK_SIZE)
    }

    /// Create a new region whose first chunk is at least `cap` bytes.
    pub fn with_capacity(cap: usize) -> Self {
        let cap = cap.max(64); // minimum sensible size
        let chunk = Chunk::new(cap, 16);
        let base = chunk.data.as_ptr() as usize;
        Self {
            chunks: vec![chunk],
            ptr: base,
            end: base + cap,
            drops: Vec::new(),
            bytes_used: 0,
            object_count: 0,
        }
    }

    /// Allocate a GC-compatible object in this region.
    ///
    /// The returned [`GcPtr`] is valid until this region is reset or dropped.
    /// The object is **not** registered in the global GC heap.
    pub fn alloc<T: Trace + 'static>(&mut self, value: T) -> GcPtr<T> {
        let layout = Layout::new::<GcBox<T>>();
        let raw = self.bump_alloc(layout);

        let gc_box = raw as *mut GcBox<T>;
        // SAFETY: `raw` is properly aligned and sized for GcBox<T>.
        #[cfg(not(feature = "no-gc"))]
        unsafe {
            ptr::write(
                gc_box,
                GcBox {
                    header: GcBoxHeader::new::<T>(),
                    value,
                },
            );
        }
        #[cfg(feature = "no-gc")]
        unsafe {
            ptr::write(gc_box, GcBox { value });
        }

        self.drops.push(DropEntry {
            ptr: raw,
            drop_fn: drop_gcbox_in_place::<T>,
        });
        self.object_count += 1;
        crate::stats::GC_STATS.record_region_alloc(layout.size());

        // SAFETY: `gc_box` is non-null (from bump_alloc).
        GcPtr(unsafe { NonNull::new_unchecked(gc_box) })
    }

    /// Drop all objects and reclaim memory, keeping the first chunk for reuse.
    pub fn reset(&mut self) {
        // Run destructors in reverse (LIFO) order.
        for entry in self.drops.drain(..).rev() {
            unsafe { (entry.drop_fn)(entry.ptr) };
        }

        // Free all chunks except the first.
        while self.chunks.len() > 1 {
            let chunk = self.chunks.pop().unwrap();
            unsafe { alloc::dealloc(chunk.data.as_ptr(), chunk.layout) };
        }

        // Reset bump pointer to the start of the first chunk.
        if let Some(first) = self.chunks.first() {
            let base = first.data.as_ptr() as usize;
            self.ptr = base;
            self.end = base + first.layout.size();
        }

        self.bytes_used = 0;
        self.object_count = 0;
    }

    /// Total bytes consumed by allocated objects (excludes padding).
    pub fn bytes_used(&self) -> usize {
        self.bytes_used
    }

    /// Number of objects currently in the region.
    pub fn object_count(&self) -> usize {
        self.object_count
    }

    // ── internal ────────────────────────────────────────────────────────────

    /// Bump-allocate `layout.size()` bytes with `layout.align()` alignment.
    fn bump_alloc(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();

        // Align the current pointer up.
        let aligned = (self.ptr + align - 1) & !(align - 1);
        let new_ptr = aligned + size;

        if new_ptr <= self.end {
            self.ptr = new_ptr;
            self.bytes_used += size;
            aligned as *mut u8
        } else {
            self.grow_and_alloc(layout)
        }
    }

    /// Allocate a new chunk large enough, then bump-allocate from it.
    fn grow_and_alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let chunk_size = DEFAULT_CHUNK_SIZE.max(size * 2);
        let chunk = Chunk::new(chunk_size, layout.align());
        let base = chunk.data.as_ptr() as usize;

        let aligned = (base + layout.align() - 1) & !(layout.align() - 1);
        self.ptr = aligned + size;
        self.end = base + chunk_size;
        self.bytes_used += size;

        self.chunks.push(chunk);
        aligned as *mut u8
    }
}

impl Default for Region {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // Run destructors in reverse order.
        for entry in self.drops.drain(..).rev() {
            unsafe { (entry.drop_fn)(entry.ptr) };
        }
        // Free all chunks.
        for chunk in self.chunks.drain(..) {
            unsafe { alloc::dealloc(chunk.data.as_ptr(), chunk.layout) };
        }
    }
}

// ── Thread-local region stack ───────────────────────────────────────────────

thread_local! {
    /// Stack of active regions for the current thread.
    ///
    /// [`RegionGuard`] pushes/pops.  Allocation-site code calls
    /// [`try_alloc_in_region`] to opportunistically use the top region.
    static REGION_STACK: RefCell<Vec<*mut Region>> = const { RefCell::new(Vec::new()) };
}

/// RAII guard that activates a [`Region`] on the thread-local stack.
///
/// When dropped, the region is popped from the stack.  The caller still owns
/// the region and is responsible for its lifetime.
pub struct RegionGuard {
    _not_send: std::marker::PhantomData<*mut ()>, // !Send
}

impl RegionGuard {
    /// Push `region` onto the thread-local region stack.
    ///
    /// # Safety
    /// The `Region` must outlive this guard.
    pub unsafe fn new(region: &mut Region) -> Self {
        let ptr = region as *mut Region;
        REGION_STACK.with(|stack| stack.borrow_mut().push(ptr));
        Self {
            _not_send: std::marker::PhantomData,
        }
    }
}

impl Drop for RegionGuard {
    fn drop(&mut self) {
        REGION_STACK.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

/// Allocate in the currently active thread-local region, if one exists.
///
/// Returns `Some(GcPtr<T>)` if a region is active, `None` otherwise (caller
/// should fall back to [`GcPtr::new`]).
///
/// # Safety
/// The returned `GcPtr` is only valid while the region is alive.  The caller
/// must ensure the pointer does not outlive the region.
pub unsafe fn try_alloc_in_region<T: Trace + 'static>(value: T) -> Option<GcPtr<T>> {
    REGION_STACK.with(|stack| {
        let stack = stack.borrow();
        if let Some(&region_ptr) = stack.last() {
            // SAFETY: RegionGuard guarantees the pointer is valid.
            let region = unsafe { &mut *region_ptr };
            Some(region.alloc(value))
        } else {
            None
        }
    })
}

/// Explicitly pop the top region from the thread-local stack.
///
/// This is used by the AOT runtime ABI where [`RegionGuard`]'s RAII `Drop`
/// cannot be used across `extern "C"` boundaries.
///
/// # Safety
/// The caller must ensure that the corresponding [`Region`] is cleaned up
/// after this call.  No `GcPtr` allocated in that region may be used
/// afterwards.
pub fn pop_region_guard() {
    REGION_STACK.with(|stack| {
        stack.borrow_mut().pop();
    });
}

/// Returns `true` if a region is currently active on this thread.
pub fn region_is_active() -> bool {
    REGION_STACK.with(|stack| !stack.borrow().is_empty())
}

/// Returns the current depth of the region stack (number of active regions).
///
/// Used by exception handling to save/restore region state on throw.
pub fn region_stack_depth() -> usize {
    REGION_STACK.with(|stack| stack.borrow().len())
}

/// Pop regions until the stack depth matches `target_depth`.
///
/// Used by exception handling to unwind region scopes on throw.
///
/// # Safety
/// The caller must ensure that the corresponding `Region` objects are
/// also cleaned up (dropped/reset) for each popped entry.
pub fn unwind_region_stack_to(target_depth: usize) {
    REGION_STACK.with(|stack| {
        let mut stack = stack.borrow_mut();
        while stack.len() > target_depth {
            stack.pop();
        }
    });
}

/// Push a raw region pointer onto the thread-local stack.
///
/// This is the non-RAII equivalent of [`RegionGuard::new`], used by the
/// AOT runtime ABI where the region's lifetime is managed explicitly.
///
/// # Safety
/// The `Region` must remain valid until the corresponding
/// [`pop_region_guard`] call.
pub unsafe fn push_region_raw(region: *mut Region) {
    REGION_STACK.with(|stack| stack.borrow_mut().push(region));
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, not(feature = "no-gc")))]
mod tests {
    use super::*;
    use crate::MarkVisitor;
    use std::sync::{Arc, Mutex};

    // A traceable type that records when it's dropped.
    #[derive(Debug)]
    struct Tracked {
        id: i32,
        dropped: Arc<Mutex<Vec<i32>>>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.dropped.lock().unwrap().push(self.id);
        }
    }

    impl Trace for Tracked {
        fn trace(&self, _: &mut MarkVisitor) {}
    }

    // A traceable type with a child GcPtr.
    #[derive(Debug)]
    struct Parent {
        child: GcPtr<Tracked>,
    }

    impl Trace for Parent {
        fn trace(&self, visitor: &mut MarkVisitor) {
            use crate::GcVisitor as _;
            visitor.visit(&self.child);
        }
    }

    #[test]
    fn basic_alloc_and_read() {
        let mut region = Region::new();
        let p = region.alloc(42i64);
        assert_eq!(*p.get(), 42);
        assert_eq!(region.object_count(), 1);
    }

    #[test]
    fn multiple_allocs() {
        let mut region = Region::new();
        let a = region.alloc(10i64);
        let b = region.alloc(20i64);
        let c = region.alloc(30i64);
        assert_eq!(*a.get(), 10);
        assert_eq!(*b.get(), 20);
        assert_eq!(*c.get(), 30);
        assert_eq!(region.object_count(), 3);
    }

    #[test]
    fn drop_runs_on_region_drop() {
        let dropped = Arc::new(Mutex::new(Vec::new()));
        {
            let mut region = Region::new();
            region.alloc(Tracked {
                id: 1,
                dropped: dropped.clone(),
            });
            region.alloc(Tracked {
                id: 2,
                dropped: dropped.clone(),
            });
            region.alloc(Tracked {
                id: 3,
                dropped: dropped.clone(),
            });
            // Region drops here.
        }
        let order = dropped.lock().unwrap();
        // Dropped in reverse (LIFO) order.
        assert_eq!(*order, vec![3, 2, 1]);
    }

    #[test]
    fn reset_drops_and_reuses() {
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Region::new();

        region.alloc(Tracked {
            id: 1,
            dropped: dropped.clone(),
        });
        region.alloc(Tracked {
            id: 2,
            dropped: dropped.clone(),
        });

        region.reset();

        {
            let order = dropped.lock().unwrap();
            assert_eq!(*order, vec![2, 1]);
        }
        assert_eq!(region.object_count(), 0);

        // Allocate again after reset.
        let p = region.alloc(99i64);
        assert_eq!(*p.get(), 99);
        assert_eq!(region.object_count(), 1);
    }

    #[test]
    fn large_alloc_triggers_new_chunk() {
        // Allocate many objects to exceed the default chunk size.
        let mut region = Region::with_capacity(128);
        for i in 0..100 {
            let p = region.alloc(i as i64);
            assert_eq!(*p.get(), i);
        }
        assert_eq!(region.object_count(), 100);
        assert!(region.chunks.len() > 1);
    }

    #[test]
    fn region_objects_not_in_gc_heap() {
        let heap = crate::GcHeap::new();
        let heap_before = heap.count();

        let mut region = Region::new();
        let _p = region.alloc(42i64);
        let _q = region.alloc(99i64);

        // Region allocations should NOT increase the GC heap count.
        assert_eq!(heap.count(), heap_before);
    }

    #[test]
    fn gc_can_trace_through_region_objects() {
        // A GC-heap parent holds a GcPtr to a region-allocated child.
        // The GC marker should be able to trace through the region object.
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Region::new();
        let child = region.alloc(Tracked {
            id: 1,
            dropped: dropped.clone(),
        });

        // The parent is on the GC heap, pointing to a region-allocated child.
        let heap = crate::GcHeap::new();
        let parent = heap.alloc(Parent {
            child: child.clone(),
        });

        // Marking should succeed without crashing.
        heap.collect(|vis| {
            use crate::GcVisitor as _;
            vis.visit(&parent);
        });

        // Parent should survive.
        assert_eq!(heap.count(), 1);
        // Child is region-managed, not in heap — still alive.
        assert!(dropped.lock().unwrap().is_empty());
    }

    #[test]
    fn thread_local_region_guard() {
        assert!(!region_is_active());

        let mut region = Region::new();
        {
            let _guard = unsafe { RegionGuard::new(&mut region) };
            assert!(region_is_active());

            // Allocate through the thread-local API.
            let p: GcPtr<i64> = unsafe { try_alloc_in_region(42i64) }.unwrap();
            assert_eq!(*p.get(), 42);
        }

        assert!(!region_is_active());
    }

    #[test]
    fn try_alloc_returns_none_without_region() {
        assert!(!region_is_active());
        let result: Option<GcPtr<i64>> = unsafe { try_alloc_in_region(42i64) };
        assert!(result.is_none());
    }

    #[test]
    fn nested_region_guards() {
        let mut r1 = Region::new();
        let mut r2 = Region::new();

        let _g1 = unsafe { RegionGuard::new(&mut r1) };
        assert!(region_is_active());

        {
            let _g2 = unsafe { RegionGuard::new(&mut r2) };
            assert!(region_is_active());

            // Allocations go to r2 (innermost).
            unsafe { try_alloc_in_region(1i64) };
            assert_eq!(r2.object_count(), 1);
            assert_eq!(r1.object_count(), 0);
        }

        // After g2 drops, allocations go to r1.
        unsafe { try_alloc_in_region(2i64) };
        assert_eq!(r1.object_count(), 1);
    }

    #[test]
    fn bytes_used_tracking() {
        let mut region = Region::new();
        let size = std::mem::size_of::<GcBox<i64>>();
        region.alloc(1i64);
        region.alloc(2i64);
        // At least 2 * size_of::<GcBox<i64>> bytes used.
        assert!(region.bytes_used() >= size * 2);
    }

    #[test]
    fn alloc_throughput_region_vs_heap() {
        const N: usize = 10_000;

        // Region allocation (bump pointer, no mutex).
        let region_start = std::time::Instant::now();
        let mut region = Region::with_capacity(N * std::mem::size_of::<GcBox<i64>>() + 1024);
        for i in 0..N as i64 {
            let p = region.alloc(i);
            std::hint::black_box(p.get());
        }
        let region_dur = region_start.elapsed();
        drop(region);

        // GC heap allocation (Box + mutex lock per allocation).
        let heap = crate::GcHeap::new();
        let heap_start = std::time::Instant::now();
        for i in 0..N as i64 {
            let p = heap.alloc(i);
            std::hint::black_box(p.get());
        }
        let heap_dur = heap_start.elapsed();

        // Region should be faster — bump allocation avoids mutex contention
        // and individual Box::new calls.
        eprintln!(
            "Region: {:?} ({:.0} ns/alloc), Heap: {:?} ({:.0} ns/alloc), speedup: {:.1}x",
            region_dur,
            region_dur.as_nanos() as f64 / N as f64,
            heap_dur,
            heap_dur.as_nanos() as f64 / N as f64,
            heap_dur.as_nanos() as f64 / region_dur.as_nanos().max(1) as f64,
        );

        // We don't assert a specific speedup (CI machines vary), but
        // the region should not be slower.
        // If this assert fires, something is wrong with the region allocator.
        assert!(
            region_dur <= heap_dur.mul_f64(2.0),
            "Region should not be significantly slower than heap"
        );
    }
}
