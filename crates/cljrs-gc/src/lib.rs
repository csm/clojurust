//! Garbage collector (default) or region-based allocator (`no-gc` feature) for clojurust.

#![allow(clippy::missing_safety_doc)]
#![allow(private_interfaces)]

use std::ptr::NonNull;

pub mod region;
pub mod stats;

#[cfg(not(feature = "no-gc"))]
pub mod cancellation;
#[cfg(not(feature = "no-gc"))]
pub mod config;

#[cfg(feature = "no-gc")]
pub mod alloc_ctx;
#[cfg(feature = "no-gc")]
pub mod static_arena;

pub use stats::{CLJRS_GC_STATS_ENV, GC_STATS, GcStats, GcStatsSnapshot, dump_stats_from_env};

// ── Re-exports from active implementation ─────────────────────────────────────

#[cfg(not(feature = "no-gc"))]
pub use cancellation::{
    CancellableGuard, MutatorGuard, StwGuard, begin_stw, check_cancellation, gc_requested,
    park_thread, register_mutator, registered_threads, request_gc, safepoint, take_gc_request,
    unpark_thread, wait_for_threads_to_park,
};
#[cfg(not(feature = "no-gc"))]
pub use config::{GC_CANCELLATION as CONFIG_CANCELLATION, GcConfig, GcParked};

#[cfg(not(feature = "no-gc"))]
pub use gc_full::{AllocRootGuard, GcHeap, HEAP, push_alloc_frame, trace_thread_alloc_roots};
#[cfg(feature = "no-gc")]
pub use nogc_stubs::{
    AllocRootGuard, CONFIG_CANCELLATION, CancellableGuard, GcConfig, GcHeap, GcParked, HEAP,
    MutatorGuard, StwGuard, begin_stw, check_cancellation, gc_requested, park_thread,
    push_alloc_frame, register_mutator, registered_threads, request_gc, safepoint, take_gc_request,
    unpark_thread, wait_for_threads_to_park,
};

/// Return `true` if `addr` was allocated by the global `StaticArena`.
///
/// Available only in `no-gc` debug builds.  Downstream crates (`cljrs-value`)
/// use this to implement write-site provenance assertions.
#[cfg(all(feature = "no-gc", debug_assertions))]
pub fn is_static_addr(addr: usize) -> bool {
    static_arena::is_static_addr(addr)
}

// ── Trace trait ───────────────────────────────────────────────────────────────

/// Implemented by every type that can be stored behind a [`GcPtr`].
///
/// The `gc_size_extra` method accounts for heap bytes owned by the value that
/// are NOT captured by `size_of::<GcBox<T>>()` (e.g. `Vec` buffers, `String`
/// capacity, `Form` AST trees stored inline).  The default returns 0, which is
/// correct for primitives and types with no out-of-line heap.
///
/// Rules for implementors of `gc_size_extra`:
/// - Count only bytes THIS value owns and will free when dropped.
/// - Do NOT cross `GcPtr` boundaries — each pointed-to box is counted
///   separately when it is allocated.
pub trait Trace {
    fn trace(&self, visitor: &mut MarkVisitor);

    fn gc_size_extra(&self) -> usize {
        0
    }
}

// ── Leaf Trace impls ──────────────────────────────────────────────────────────

impl Trace for String {
    fn trace(&self, _: &mut MarkVisitor) {}

    fn gc_size_extra(&self) -> usize {
        self.capacity()
    }
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
impl Trace for num_bigint::BigInt {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for bigdecimal::BigDecimal {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for num_rational::Ratio<num_bigint::BigInt> {
    fn trace(&self, _: &mut MarkVisitor) {}
}
impl Trace for regex::Regex {
    fn trace(&self, _: &mut MarkVisitor) {}
}
macro_rules! impl_trace_prim_array {
    ($t:ty) => {
        impl Trace for std::sync::Mutex<Vec<$t>> {
            fn trace(&self, _: &mut MarkVisitor) {}
            fn gc_size_extra(&self) -> usize {
                self.lock().unwrap().capacity() * std::mem::size_of::<$t>()
            }
        }
    };
}
impl_trace_prim_array!(i32);
impl_trace_prim_array!(i64);
impl_trace_prim_array!(i16);
impl_trace_prim_array!(i8);
impl_trace_prim_array!(char);
impl_trace_prim_array!(f64);
impl_trace_prim_array!(f32);
impl_trace_prim_array!(bool);

// ── GcVisitor ─────────────────────────────────────────────────────────────────

pub trait GcVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>);
}

// =============================================================================
// GC build: GcBox with header, MarkVisitor with grey stack
// =============================================================================

#[cfg(not(feature = "no-gc"))]
pub use self::gc_header::{GcBox, GcBoxHeader};

#[cfg(not(feature = "no-gc"))]
mod gc_header {
    use crate::{MarkVisitor, Trace};
    use std::cell::Cell;

    // Objects start at lives = GC_INITIAL_LIVES - 1.  The mark phase sets
    // lives = GC_INITIAL_LIVES for reachable objects; sweep frees objects
    // whose lives reach 0.  A value of 2 gives exactly one cycle of grace:
    // enough to cover the window between an alloc frame dropping and the
    // next GC safepoint (where VALUE_ROOTS or a new alloc frame re-roots it).
    // 10 was chosen conservatively but keeps 9× more garbage in RAM than
    // necessary, worsening OOM pressure under test suites with many forms.
    pub(crate) const GC_INITIAL_LIVES: u8 = 2;

    #[cfg(debug_assertions)]
    pub(crate) const GC_MAGIC_ALIVE: u64 = 0xCAFE_BABE_DEAD_BEEF;
    #[cfg(debug_assertions)]
    pub(crate) const GC_MAGIC_FREED: u64 = 0xDEAD_DEAD_DEAD_DEAD;

    #[repr(C)]
    pub struct GcBoxHeader {
        #[cfg(debug_assertions)]
        pub(crate) magic: Cell<u64>,
        /// Exact size of the GcBox<T> allocation in bytes.
        pub(crate) size: usize,
        pub(crate) lives: Cell<u8>,
        pub(crate) next: Cell<*mut GcBoxHeader>,
        pub(crate) trace_fn: unsafe fn(*const GcBoxHeader, &mut MarkVisitor),
        pub(crate) drop_fn: unsafe fn(*mut GcBoxHeader),
    }

    impl GcBoxHeader {
        pub(crate) fn new<T: Trace + 'static>(heap_extra: usize) -> Self {
            Self {
                #[cfg(debug_assertions)]
                magic: Cell::new(GC_MAGIC_ALIVE),
                size: std::mem::size_of::<GcBox<T>>() + heap_extra,
                lives: Cell::new(GC_INITIAL_LIVES - 1),
                next: Cell::new(std::ptr::null_mut()),
                trace_fn: trace_gc_box::<T>,
                drop_fn: drop_gc_box::<T>,
            }
        }
    }

    unsafe impl Send for GcBoxHeader {}
    unsafe impl Sync for GcBoxHeader {}

    #[repr(C)]
    pub struct GcBox<T: Trace + 'static> {
        pub(crate) header: GcBoxHeader,
        pub value: T,
    }

    pub(crate) unsafe fn trace_gc_box<T: Trace + 'static>(
        header: *const GcBoxHeader,
        visitor: &mut MarkVisitor,
    ) {
        unsafe {
            let gc_box = header as *const GcBox<T>;
            (*gc_box).value.trace(visitor);
        }
    }

    pub(crate) unsafe fn drop_gc_box<T: Trace + 'static>(header: *mut GcBoxHeader) {
        unsafe {
            #[cfg(debug_assertions)]
            {
                (*header).magic.set(GC_MAGIC_FREED);
            }
            let gc_box = header as *mut GcBox<T>;
            drop(Box::from_raw(gc_box));
        }
    }
}

// =============================================================================
// no-gc build: GcBox without header
// =============================================================================

#[cfg(feature = "no-gc")]
pub use self::nogc_box::GcBox;

#[cfg(feature = "no-gc")]
mod nogc_box {
    use crate::Trace;

    pub struct GcBox<T: Trace + 'static> {
        pub value: T,
    }
}

// =============================================================================
// MarkVisitor: full under GC, stub under no-gc
// =============================================================================

#[cfg(not(feature = "no-gc"))]
#[derive(Default)]
pub struct MarkVisitor {
    pub(crate) grey: Vec<*mut GcBoxHeader>,
}

#[cfg(feature = "no-gc")]
pub struct MarkVisitor;

#[cfg(not(feature = "no-gc"))]
impl MarkVisitor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn grey_len(&self) -> usize {
        self.grey.len()
    }

    pub unsafe fn mark_header(&mut self, header: *mut GcBoxHeader) {
        use gc_header::GC_INITIAL_LIVES;
        let h = unsafe { &*header };
        if h.lives.get() < GC_INITIAL_LIVES {
            h.lives.set(GC_INITIAL_LIVES);
            self.grey.push(header);
        }
    }

    pub(crate) fn drain(&mut self) {
        let mut visited = 0usize;
        while let Some(header) = self.grey.pop() {
            visited += 1;
            let h = unsafe { &*header };
            unsafe { (h.trace_fn)(header as *const GcBoxHeader, self) };
        }
        cljrs_logging::feat_debug!("gc", "drain visited {} objects", visited);
    }
}

#[cfg(not(feature = "no-gc"))]
impl GcVisitor for MarkVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>) {
        use gc_header::GC_INITIAL_LIVES;
        let header = unsafe { &(*ptr.0.as_ptr()).header };
        if header.lives.get() < GC_INITIAL_LIVES {
            header.lives.set(GC_INITIAL_LIVES);
            self.grey.push(ptr.0.as_ptr() as *mut GcBoxHeader);
        }
    }
}

#[cfg(feature = "no-gc")]
impl MarkVisitor {
    pub fn grey_len(&self) -> usize {
        0
    }
    pub unsafe fn mark_header(&mut self, _: *mut u8) {}
}

#[cfg(feature = "no-gc")]
impl GcVisitor for MarkVisitor {
    fn visit<T: Trace + 'static>(&mut self, _: &GcPtr<T>) {}
}

// =============================================================================
// GcPtr — always present
// =============================================================================

pub struct GcPtr<T: Trace + 'static>(NonNull<GcBox<T>>);

impl<T: Trace + 'static> GcPtr<T> {
    #[cfg(not(feature = "no-gc"))]
    pub fn new(value: T) -> Self {
        gc_full::HEAP.alloc(value)
    }

    #[cfg(feature = "no-gc")]
    pub fn new(value: T) -> Self {
        alloc_ctx::alloc_in_ctx(value)
    }

    pub fn get(&self) -> &T {
        #[cfg(all(debug_assertions, not(feature = "no-gc")))]
        {
            use gc_header::GC_MAGIC_ALIVE;
            let header = unsafe { &(*self.0.as_ptr()).header };
            assert_eq!(
                header.magic.get(),
                GC_MAGIC_ALIVE,
                "GcPtr::get() on freed object! magic={:#x}",
                header.magic.get(),
            );
        }
        unsafe { &(*self.0.as_ptr()).value }
    }

    pub fn get_mut(&mut self) -> &mut T {
        #[cfg(all(debug_assertions, not(feature = "no-gc")))]
        {
            use gc_header::GC_MAGIC_ALIVE;
            let header = unsafe { &(*self.0.as_ptr()).header };
            assert_eq!(
                header.magic.get(),
                GC_MAGIC_ALIVE,
                "GcPtr::get_mut() on freed object! magic={:#x}",
                header.magic.get(),
            );
        }
        unsafe { &mut (*self.0.as_ptr()).value }
    }

    pub fn ptr_eq(a: &Self, b: &Self) -> bool {
        a.0 == b.0
    }

    /// Return `true` if this pointer was allocated by the global `StaticArena`.
    ///
    /// Only meaningful (and only compiled) in `no-gc` debug builds.  Used by
    /// write-site assertions in `Atom::reset` / `Var::bind` to catch
    /// region-local values being stored in program-lifetime containers.
    #[cfg(all(feature = "no-gc", debug_assertions))]
    pub fn is_static_alloc(&self) -> bool {
        static_arena::is_static_addr(self.0.as_ptr() as usize)
    }
}

impl<T: Trace + 'static> Clone for GcPtr<T> {
    fn clone(&self) -> Self {
        GcPtr(self.0)
    }
}

impl<T: Trace + 'static + std::fmt::Debug> std::fmt::Debug for GcPtr<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        unsafe { (*self.0.as_ptr()).value.fmt(f) }
    }
}

impl<T: Trace + 'static> Drop for GcPtr<T> {
    fn drop(&mut self) {}
}

// =============================================================================
// Full GC implementation (default build)
// =============================================================================

#[cfg(not(feature = "no-gc"))]
mod gc_full {
    use std::cell::RefCell;
    use std::ptr::NonNull;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use crate::config::GcConfig;
    use crate::gc_header::GC_INITIAL_LIVES;
    use crate::{GcBox, GcBoxHeader, GcPtr, MarkVisitor, Trace};

    type RootTracer = Box<dyn Fn(&mut MarkVisitor)>;

    pub struct GcHeap {
        inner: Mutex<GcHeapInner>,
        config: Mutex<Option<Arc<GcConfig>>>,
        memory_in_use: AtomicUsize,
        total_allocated_bytes: AtomicUsize,
        root_tracers: Mutex<Vec<RootTracer>>,
        gc_suppressed: std::sync::atomic::AtomicBool,
        /// memory_in_use threshold above which GC is re-enabled after a
        /// zero-yield collection.  The headroom doubles on each consecutive
        /// zero-yield cycle (exponential backoff, capped at soft_limit) so a
        /// long computation where all objects are live doesn't spin in a
        /// constant GC storm of O(N) sweeps.  Resets to the base headroom
        /// (soft_limit / 10) once GC actually frees something.
        suppressed_threshold: AtomicUsize,
        /// Current headroom used for exponential backoff after zero-yield cycles.
        zero_yield_headroom: AtomicUsize,
    }

    struct GcHeapInner {
        head: *mut GcBoxHeader,
        count: usize,
        total_allocated: usize,
        total_freed: usize,
    }

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
                config: Mutex::new(None),
                memory_in_use: AtomicUsize::new(0),
                total_allocated_bytes: AtomicUsize::new(0),
                root_tracers: Mutex::new(Vec::new()),
                gc_suppressed: std::sync::atomic::AtomicBool::new(false),
                suppressed_threshold: AtomicUsize::new(0),
                zero_yield_headroom: AtomicUsize::new(0),
            }
        }

        pub fn set_config(&self, config: Arc<GcConfig>) {
            *self.config.lock().unwrap() = Some(config);
        }

        pub fn set_config_from_env(&self) {
            #[cfg(not(target_arch = "wasm32"))]
            let default_soft_limit: usize = (system_memory::total() / 3) as usize;
            #[cfg(target_arch = "wasm32")]
            let default_soft_limit: usize = 64 * 1024 * 1024;

            let soft_limit_mb: usize = match std::env::var("CLJRS_GC_SOFT_LIMIT_MB").ok() {
                Some(s) => s.parse::<usize>().unwrap() * (1024 * 1024),
                None => default_soft_limit,
            };
            let hard_limit_mb: usize = match std::env::var("CLJRS_GC_HARD_LIMIT_MB").ok() {
                Some(s) => s.parse::<usize>().unwrap() * (1024 * 1024),
                None => soft_limit_mb,
            };
            self.set_config(Arc::new(GcConfig::with_limits(
                soft_limit_mb,
                hard_limit_mb,
            )));
        }

        pub fn register_root_tracer(&self, tracer: impl Fn(&mut MarkVisitor) + 'static) {
            self.root_tracers.lock().unwrap().push(Box::new(tracer));
        }

        pub fn trace_registered_roots(&self, visitor: &mut MarkVisitor) {
            let tracers = self.root_tracers.lock().unwrap();
            for tracer in tracers.iter() {
                tracer(visitor);
            }
        }

        pub fn memory_in_use(&self) -> usize {
            self.memory_in_use.load(Ordering::Relaxed)
        }

        #[cfg(test)]
        pub fn set_memory_in_use(&self, bytes: usize) {
            self.memory_in_use.store(bytes, Ordering::Relaxed);
        }

        pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T> {
            crate::cancellation::safepoint();
            let heap_extra = value.gc_size_extra();
            let gc_box = Box::new(GcBox {
                header: GcBoxHeader::new::<T>(heap_extra),
                value,
            });
            let obj_size = gc_box.header.size; // GcBox<T> size + gc_size_extra()
            let raw: *mut GcBox<T> = Box::into_raw(gc_box);
            {
                let mut inner = self.inner.lock().unwrap();
                unsafe {
                    (*raw).header.next.set(inner.head);
                    inner.head = raw as *mut GcBoxHeader;
                }
                inner.count += 1;
                inner.total_allocated += 1;
            }
            self.total_allocated_bytes
                .fetch_add(obj_size, Ordering::Relaxed);
            crate::stats::GC_STATS.record_gc_alloc(obj_size);
            let current_usage =
                self.memory_in_use.fetch_add(obj_size, Ordering::Relaxed) + obj_size;

            if let Some(config) = self.config.lock().unwrap().as_ref()
                && config.soft_limit_exceeded(current_usage)
            {
                if self.gc_suppressed.load(Ordering::Relaxed) {
                    // Suppression active: only re-enable GC once memory has
                    // grown past the threshold set by the last zero-yield
                    // collection (current_memory + soft_limit/10).
                    let threshold = self.suppressed_threshold.load(Ordering::Relaxed);
                    if current_usage > threshold {
                        self.gc_suppressed.store(false, Ordering::Relaxed);
                        crate::cancellation::request_gc();
                    }
                } else {
                    crate::cancellation::request_gc();
                }
            }

            register_alloc(raw as *mut GcBoxHeader);
            GcPtr(unsafe { NonNull::new_unchecked(raw) })
        }

        pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, trace_roots: F) {
            let pre_count = self.inner.lock().unwrap().count;
            let pre_memory = self.memory_in_use.load(Ordering::Relaxed);
            cljrs_logging::feat_debug!(
                "gc",
                "starting collection: {} objects, ~{} bytes in use",
                pre_count,
                pre_memory
            );
            let mark_start = std::time::Instant::now();
            let mut visitor = MarkVisitor::new();
            trace_roots(&mut visitor);
            cljrs_logging::feat_debug!(
                "gc",
                "starting drain with {} grey objects",
                visitor.grey.len()
            );
            visitor.drain();
            let mark_elapsed = mark_start.elapsed();

            let sweep_start = std::time::Instant::now();
            let mut inner = self.inner.lock().unwrap();
            let mut live: Vec<*mut GcBoxHeader> = Vec::with_capacity(inner.count);
            let mut dead: Vec<*mut GcBoxHeader> = Vec::new();
            // Bytes of objects with lives==0 that will be freed now.
            let mut freed_bytes: usize = 0;
            let mut current = inner.head;
            while !current.is_null() {
                let header = unsafe { &*current };
                let next = header.next.get();
                let lives = header.lives.get();
                let obj_size = header.size;
                if lives >= GC_INITIAL_LIVES {
                    // Marked reachable this cycle — reset grace counter.
                    header.lives.set(GC_INITIAL_LIVES - 1);
                    live.push(current);
                } else if lives > 0 {
                    // In grace period (unreachable but not yet freed).
                    header.lives.set(lives - 1);
                    live.push(current);
                } else {
                    // Grace period exhausted — collect now.
                    freed_bytes += obj_size;
                    dead.push(current);
                }
                current = next;
            }
            let freed_count = dead.len();
            for ptr in dead {
                let header = unsafe { &*ptr };
                unsafe { (header.drop_fn)(ptr) };
                inner.count -= 1;
                inner.total_freed += 1;
            }
            inner.head = std::ptr::null_mut();
            for ptr in live {
                let header = unsafe { &*ptr };
                header.next.set(inner.head);
                inner.head = ptr;
            }
            // Decrement memory_in_use by the bytes actually freed.  All heap
            // objects (live + grace-period) remain counted; only physically
            // freed objects are subtracted.  This keeps memory pressure
            // accurate so GC fires again when the heap genuinely grows.
            self.memory_in_use.fetch_sub(freed_bytes, Ordering::Relaxed);
            let sweep_elapsed = sweep_start.elapsed();
            crate::stats::GC_STATS.record_gc_pause(
                mark_elapsed + sweep_elapsed,
                freed_count as u64,
                freed_bytes as u64,
            );
            let post_memory = self.memory_in_use.load(Ordering::Relaxed);
            cljrs_logging::feat_debug!(
                "gc",
                "collection complete: freed {} (~{} bytes), {} remaining (~{} bytes), mark={:.2?} sweep={:.2?}",
                freed_count,
                freed_bytes,
                inner.count,
                post_memory,
                mark_elapsed,
                sweep_elapsed
            );
            if freed_count == 0 {
                // Zero-yield collection: exponential-backoff suppression.
                // Each consecutive zero-yield cycle doubles the headroom before
                // the next GC attempt (capped at soft_limit/4).  This prevents a
                // GC storm during deep recursion where all objects are live —
                // without backoff, GC fires every soft_limit/10 bytes, tracing
                // the entire live set O(N) times to no benefit.
                // The headroom resets to soft_limit/10 when GC frees something.
                // Cap at soft_limit/4 (not soft_limit) so that GC still fires
                // frequently enough to catch short-lived test allocations after
                // a long namespace-loading phase of zero-yield cycles.
                let soft_limit = self
                    .config
                    .lock()
                    .unwrap()
                    .as_ref()
                    .map(|c| c.soft_limit())
                    .unwrap_or(64 * 1024 * 1024);
                let base_headroom = (soft_limit / 10).max(1);
                let max_headroom = (soft_limit / 4).max(base_headroom);
                let prev_headroom = self.zero_yield_headroom.load(Ordering::Relaxed);
                let headroom = if prev_headroom == 0 {
                    base_headroom
                } else {
                    prev_headroom.saturating_mul(2).min(max_headroom)
                };
                self.zero_yield_headroom.store(headroom, Ordering::Relaxed);
                self.suppressed_threshold
                    .store(post_memory + headroom, Ordering::Relaxed);
                self.gc_suppressed.store(true, Ordering::Relaxed);
            } else {
                // GC freed something: reset exponential backoff.
                self.zero_yield_headroom.store(0, Ordering::Relaxed);
                self.gc_suppressed.store(false, Ordering::Relaxed);
            }
        }

        pub fn count(&self) -> usize {
            self.inner.lock().unwrap().count
        }
        pub fn total_allocated(&self) -> usize {
            self.inner.lock().unwrap().total_allocated
        }
        pub fn total_freed(&self) -> usize {
            self.inner.lock().unwrap().total_freed
        }

        pub fn collect_auto(&self) -> bool {
            cljrs_logging::feat_debug!("gc", "automatic collection requested");
            let Some(_stw_guard) = crate::cancellation::begin_stw() else {
                cljrs_logging::feat_debug!("gc", "automatic collection skipped");
                return false;
            };
            self.collect(|visitor| self.trace_registered_roots(visitor));
            true
        }
    }

    pub static HEAP: GcHeap = GcHeap::new();

    thread_local! {
        pub(crate) static ALLOC_ROOTS: RefCell<Vec<*mut GcBoxHeader>> = const { RefCell::new(Vec::new()) };
    }

    pub struct AllocRootGuard {
        saved_len: usize,
    }

    impl Drop for AllocRootGuard {
        fn drop(&mut self) {
            ALLOC_ROOTS.with(|roots| roots.borrow_mut().truncate(self.saved_len));
        }
    }

    pub fn push_alloc_frame() -> AllocRootGuard {
        let saved_len = ALLOC_ROOTS.with(|roots| roots.borrow().len());
        AllocRootGuard { saved_len }
    }

    fn register_alloc(header: *mut GcBoxHeader) {
        ALLOC_ROOTS.with(|roots| roots.borrow_mut().push(header));
    }

    pub fn trace_thread_alloc_roots(visitor: &mut MarkVisitor) {
        ALLOC_ROOTS.with(|roots| {
            let roots = roots.borrow();
            for &header in roots.iter() {
                unsafe { visitor.mark_header(header) };
            }
        });
    }
}

// =============================================================================
// no-gc stubs
// =============================================================================

#[cfg(feature = "no-gc")]
mod nogc_stubs {
    use crate::MarkVisitor;
    use std::sync::Arc;

    #[derive(Debug, Clone)]
    pub struct GcConfig;
    impl GcConfig {
        pub fn new() -> Self {
            Self
        }
        pub fn with_hard_limit(_: usize) -> Self {
            Self
        }
        pub fn with_limits(_: usize, _: usize) -> Self {
            Self
        }
    }
    impl Default for GcConfig {
        fn default() -> Self {
            Self::new()
        }
    }

    pub struct GcHeap;
    impl Default for GcHeap {
        fn default() -> Self {
            Self::new()
        }
    }
    impl GcHeap {
        pub const fn new() -> Self {
            Self
        }
        pub fn set_config(&self, _: Arc<GcConfig>) {}
        pub fn register_root_tracer(&self, _: impl Fn(&mut MarkVisitor) + 'static) {}
        pub fn trace_registered_roots(&self, _: &mut MarkVisitor) {}
        pub fn memory_in_use(&self) -> usize {
            0
        }
        pub fn count(&self) -> usize {
            0
        }
        pub fn total_allocated(&self) -> usize {
            0
        }
        pub fn total_freed(&self) -> usize {
            0
        }
        pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, _: F) {}
        pub fn collect_auto(&self) -> bool {
            false
        }
    }
    unsafe impl Sync for GcHeap {}
    pub static HEAP: GcHeap = GcHeap::new();

    pub struct MutatorGuard;
    impl Drop for MutatorGuard {
        fn drop(&mut self) {}
    }
    pub struct StwGuard;
    impl Drop for StwGuard {
        fn drop(&mut self) {}
    }
    pub struct GcParked;
    pub struct CancellableGuard;

    pub struct GcCancellationStub;
    impl GcCancellationStub {
        pub const fn new() -> Self {
            Self
        }
        pub fn in_progress(&self) -> bool {
            false
        }
    }
    pub static CONFIG_CANCELLATION: GcCancellationStub = GcCancellationStub::new();

    pub fn safepoint() {}
    pub fn gc_requested() -> bool {
        false
    }
    pub fn take_gc_request() -> bool {
        false
    }
    pub fn begin_stw() -> Option<StwGuard> {
        None
    }
    pub fn register_mutator() -> MutatorGuard {
        MutatorGuard
    }
    pub fn registered_threads() -> usize {
        0
    }
    pub fn request_gc() {}
    pub fn check_cancellation() -> Result<(), GcParked> {
        Ok(())
    }
    pub fn park_thread() {}
    pub fn unpark_thread() {}
    pub fn wait_for_threads_to_park() {}

    pub struct AllocRootGuard;
    impl Drop for AllocRootGuard {
        fn drop(&mut self) {}
    }
    pub fn push_alloc_frame() -> AllocRootGuard {
        AllocRootGuard
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(all(test, not(feature = "no-gc")))]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug)]
    #[allow(dead_code)]
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

    #[derive(Debug)]
    #[allow(dead_code)]
    struct Parent {
        child: GcPtr<Tracked>,
    }
    impl Trace for Parent {
        fn trace(&self, visitor: &mut MarkVisitor) {
            visitor.visit(&self.child);
        }
    }

    fn fresh_heap() -> gc_full::GcHeap {
        let heap = gc_full::GcHeap::new();
        heap.set_config(Arc::new(GcConfig::with_limits(10000, 50000)));
        heap
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
    fn collect_keeps_reachable() {
        let heap = fresh_heap();
        let dropped = Arc::new(Mutex::new(false));
        let p = heap.alloc(Tracked {
            value: 2,
            dropped: dropped.clone(),
        });
        heap.collect(|vis| vis.visit(&p));
        assert_eq!(heap.count(), 1);
        assert!(!*dropped.lock().unwrap());
    }
}

#[cfg(all(test, feature = "no-gc"))]
mod nogc_tests {
    use super::*;
    use alloc_ctx::{ScratchGuard, StaticCtxGuard};

    #[test]
    fn alloc_in_static_context() {
        let _g = StaticCtxGuard::new();
        let p = GcPtr::new(42i64);
        assert_eq!(*p.get(), 42);
    }

    #[test]
    fn alloc_in_scratch_region() {
        let mut scratch = ScratchGuard::new();
        let p = GcPtr::new(99i64);
        assert_eq!(*p.get(), 99);
        scratch.pop_for_return();
        assert_eq!(*p.get(), 99);
        // scratch drops here, resets the region
    }

    #[test]
    fn ptr_eq() {
        let _g = StaticCtxGuard::new();
        let p = GcPtr::new(1i64);
        let q = p.clone();
        assert!(GcPtr::ptr_eq(&p, &q));
    }
}
