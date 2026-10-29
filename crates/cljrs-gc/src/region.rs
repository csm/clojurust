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

/// Raised when a bounded region cannot satisfy an allocation.
///
/// `GcPtr::new` is intentionally infallible throughout the runtime, so a
/// bounded invocation reports exhaustion by unwinding with this typed payload.
/// Execution boundaries can catch it with `catch_unwind` and turn it into a
/// normal transaction error without confusing it with a user panic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RegionLimitExceeded {
    pub limit: usize,
    pub used: usize,
    pub requested: usize,
}

// ── Internal: chunk of raw memory ───────────────────────────────────────────

/// A region chunk.  Each chunk is charged to the calling thread's isolate
/// account (`MemoryClass::Region`) when it is allocated and released when it
/// is freed.
struct Chunk {
    data: NonNull<u8>,
    layout: Layout,
    /// Whether the chunk was charged to an isolate account.  `false` only
    /// when the thread's account was already torn down.
    charged: bool,
}

impl Chunk {
    fn new(size: usize, align: usize) -> Self {
        let layout =
            Layout::from_size_align(size, align.max(16)).expect("Region: invalid chunk layout");
        let charged =
            crate::governor::with_current_account(|a| a.charge_region(layout.size())).is_some();
        // SAFETY: layout has non-zero size.
        let data =
            unsafe { NonNull::new(alloc::alloc(layout)).expect("Region: chunk allocation failed") };
        Self {
            data,
            layout,
            charged,
        }
    }

    /// Free the chunk's memory and return its charge.  `retired` selects the
    /// account's retired-region count.
    fn free(self, retired: bool) {
        // SAFETY: `data` was allocated with `layout` in `Chunk::new`.
        unsafe { alloc::dealloc(self.data.as_ptr(), self.layout) };
        if self.charged {
            // No account means it was dropped at thread exit, and it
            // returned this charge then.
            crate::governor::with_registered_account(|a| {
                a.release_region(self.layout.size(), retired)
            });
        }
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
    /// Managed bytes charged to this region, including out-of-line storage
    /// reported by `Trace::gc_size_extra`.
    accounted_bytes: usize,
    /// Optional hard ceiling for managed bytes. A bounded region is backed by
    /// one fixed-size chunk and never grows.
    byte_limit: Option<usize>,
    /// Number of objects allocated.
    object_count: usize,
    /// Set when the region is retired; its chunks are then charged to the
    /// account's retired-region count.
    retired: bool,
    #[cfg(feature = "no-gc")]
    lease: Option<std::rc::Rc<LeaseOwner>>,
    #[cfg(feature = "no-gc")]
    id: u64,
    #[cfg(feature = "no-gc")]
    published: bool,
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
            accounted_bytes: 0,
            byte_limit: None,
            object_count: 0,
            retired: false,
            #[cfg(feature = "no-gc")]
            lease: None,
            #[cfg(feature = "no-gc")]
            id: NEXT_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            #[cfg(feature = "no-gc")]
            published: false,
        }
    }

    /// Create a region with a fixed managed-memory budget.
    ///
    /// The region allocates one `limit`-byte chunk and will never grow it.
    /// Object boxes, alignment, and out-of-line bytes reported through
    /// [`Trace::gc_size_extra`] are charged to the limit. Allocations made by
    /// ordinary Rust containers that are not owned by a traced value are not
    /// visible here; callers that require a process-wide hard RSS ceiling must
    /// add an outer sandbox (for example a WASM linear-memory limit).
    pub fn with_limit(limit: usize) -> Self {
        assert!(limit >= 64, "Region: byte limit must be at least 64");
        let chunk = Chunk::new(limit, 16);
        let base = chunk.data.as_ptr() as usize;
        Self {
            chunks: vec![chunk],
            ptr: base,
            end: base + limit,
            drops: Vec::new(),
            bytes_used: 0,
            accounted_bytes: 0,
            byte_limit: Some(limit),
            object_count: 0,
            retired: false,
            #[cfg(feature = "no-gc")]
            lease: None,
            #[cfg(feature = "no-gc")]
            id: NEXT_SCOPE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            #[cfg(feature = "no-gc")]
            published: false,
        }
    }

    /// Allocate a GC-compatible object in this region.
    ///
    /// The returned [`GcPtr`] is valid until this region is reset or dropped.
    /// The object is **not** registered in the global GC heap.
    pub fn alloc<T: Trace + 'static>(&mut self, value: T) -> GcPtr<T> {
        let layout = Layout::new::<GcBox<T>>();
        let requested = layout.size().saturating_add(value.gc_size_extra());
        self.charge(requested);
        let raw = self.bump_alloc(layout);

        let gc_box = raw as *mut GcBox<T>;
        // SAFETY: `raw` is properly aligned and sized for GcBox<T>.
        #[cfg(not(feature = "no-gc"))]
        unsafe {
            ptr::write(
                gc_box,
                GcBox {
                    header: GcBoxHeader::new::<T>(0),
                    value,
                },
            );
        }
        #[cfg(feature = "no-gc")]
        unsafe {
            ptr::write(gc_box, GcBox::new(value));
        }

        self.drops.push(DropEntry {
            ptr: raw,
            drop_fn: drop_gcbox_in_place::<T>,
        });
        self.object_count += 1;
        crate::stats::GC_STATS.record_region_alloc(layout.size());

        // SAFETY: `gc_box` is non-null (from bump_alloc) and ≥16-aligned.
        // In GC builds the pointer is tagged region-local so the collector
        // never dereferences it after this region is reset; in no-gc builds
        // there is no collector and the pointer is untagged.
        #[cfg(not(feature = "no-gc"))]
        {
            unsafe { GcPtr::from_region_raw(gc_box) }
        }
        #[cfg(feature = "no-gc")]
        {
            let pointer = GcPtr(unsafe { NonNull::new_unchecked(gc_box) });
            pointer.get().init_ownership();
            pointer
        }
    }

    /// Trace every live object in this region, marking any GC-heap objects
    /// they reference so the collector keeps those alive.
    ///
    /// Called by [`trace_active_regions`] during GC root scanning.  Each
    /// drop-entry pointer is a live `GcBox` whose header `trace_fn` walks the
    /// value's `GcPtr` children; the region is on the active stack, so its
    /// headers are valid.
    #[cfg(not(feature = "no-gc"))]
    pub(crate) fn trace_live(&self, visitor: &mut crate::MarkVisitor) {
        for entry in &self.drops {
            let header = entry.ptr as *const GcBoxHeader;
            // SAFETY: `entry.ptr` is a live GcBox in this (active) region.
            unsafe { ((*header).trace_fn)(header, visitor) };
        }
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
            #[cfg(feature = "no-gc")]
            unindex_chunk(&chunk);
            chunk.free(self.retired);
        }

        // Reset bump pointer to the start of the first chunk.
        if let Some(first) = self.chunks.first() {
            let base = first.data.as_ptr() as usize;
            self.ptr = base;
            self.end = base + first.layout.size();
        }

        self.bytes_used = 0;
        self.accounted_bytes = 0;
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

    /// Managed bytes charged against this region's optional limit.
    pub fn accounted_bytes(&self) -> usize {
        self.accounted_bytes
    }

    /// Configured managed-memory limit, if this is a bounded region.
    pub fn byte_limit(&self) -> Option<usize> {
        self.byte_limit
    }

    /// Bytes of chunk memory this region holds (its governor charge).
    pub fn chunk_bytes(&self) -> usize {
        self.chunks.iter().map(|c| c.layout.size()).sum()
    }

    /// Mark the region retired: move its chunk charge to the account's
    /// retired-region count.  The charge stays until the region is dropped.
    #[cfg_attr(feature = "no-gc", allow(dead_code))]
    fn retire(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        let bytes: usize = self
            .chunks
            .iter()
            .filter(|c| c.charged)
            .map(|c| c.layout.size())
            .sum();
        crate::governor::with_registered_account(|a| a.retire_region(bytes));
    }

    /// Bytes of the new chunk that allocating `layout` would charge, or
    /// `None` when the current chunk has room.  A bounded region charges no
    /// chunk to the governor (it reports exhaustion itself), so it also
    /// returns `None`.
    #[cfg(feature = "no-gc")]
    pub(crate) fn chunk_needed(&self, layout: Layout) -> Option<usize> {
        if self.byte_limit.is_some() {
            return None;
        }
        let aligned = (self.ptr + layout.align() - 1) & !(layout.align() - 1);
        if aligned + layout.size() <= self.end {
            None
        } else {
            Some(DEFAULT_CHUNK_SIZE.max(layout.size() * 2))
        }
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

    fn charge(&mut self, requested: usize) {
        if let Some(limit) = self.byte_limit
            && self.accounted_bytes.saturating_add(requested) > limit
        {
            std::panic::panic_any(RegionLimitExceeded {
                limit,
                used: self.accounted_bytes,
                requested,
            });
        }
        self.accounted_bytes = self.accounted_bytes.saturating_add(requested);
    }

    /// Allocate a new chunk large enough, then bump-allocate from it.
    fn grow_and_alloc(&mut self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        if let Some(limit) = self.byte_limit {
            std::panic::panic_any(RegionLimitExceeded {
                limit,
                used: self.accounted_bytes.saturating_sub(size),
                requested: size,
            });
        }
        let chunk_size = DEFAULT_CHUNK_SIZE.max(size * 2);
        let chunk = Chunk::new(chunk_size, layout.align());
        if self.retired && chunk.charged {
            crate::governor::with_registered_account(|account| {
                account.retire_region(chunk.layout.size())
            });
        }
        let base = chunk.data.as_ptr() as usize;

        let aligned = (base + layout.align() - 1) & !(layout.align() - 1);
        self.ptr = aligned + size;
        self.end = base + chunk_size;
        self.bytes_used += size;

        #[cfg(feature = "no-gc")]
        if let Some(owner) = &self.lease {
            index_chunk(&chunk, owner);
        }
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
        let retired = self.retired;
        for chunk in self.chunks.drain(..) {
            #[cfg(feature = "no-gc")]
            unindex_chunk(&chunk);
            chunk.free(retired);
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
crate::mutator_slot!("cljrs-gc/REGION_STACK", refcell REGION_STACK: Vec<*mut Region>, trace trace_saved_regions);

fn trace_saved_regions(regions: &Vec<*mut Region>, visitor: &mut crate::MarkVisitor) {
    #[cfg(not(feature = "no-gc"))]
    for &region in regions {
        // SAFETY: a suspended fiber retains the guards owning these regions.
        unsafe { &*region }.trace_live(visitor);
    }
    let _ = (regions, visitor);
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

// ── Poisoned / retired regions ───────────────────────────────────
//
// The heap-promotion fallback (Phase 10.5): when a value that is *opaque* to
// the promotion scan (e.g. an unrealized lazy seq) is published to a
// program-lifetime cell while regions are active, we can no longer prove the
// active regions hold only non-escaping values.  Rather than risk a dangling
// pointer, the publisher "poisons" the active regions: each one is *retired*
// when its scope closes — its memory is kept alive forever (and traced as a
// GC root so heap children stay live) instead of being reset.  A deliberate,
// bounded leak, mirroring the JIT's pinned-epoch precedent.

#[cfg(not(feature = "no-gc"))]
type RetiredRegion = Region;
#[cfg(feature = "no-gc")]
type RetiredRegion = Box<Region>;

thread_local! {
    /// Regions whose scopes have closed but which may still be referenced —
    /// kept alive for the rest of the process and traced as roots in GC builds.
    /// No-gc boxes also preserve captured allocation-target addresses.
    static RETIRED_REGIONS: RefCell<Vec<RetiredRegion>> = const { RefCell::new(Vec::new()) };
    /// Poison watermark: every region at stack depth ≤ this value must be
    /// retired (not reset) when it closes.
    static POISON_WATERMARK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
// The watermark is a depth into REGION_STACK, so it moves with it.
crate::mutator_slot!("cljrs-gc/POISON_WATERMARK", cell POISON_WATERMARK: usize);

/// Poison every region currently active on this thread: each will be retired
/// instead of reset when its scope closes.  No-op when no region is active.
pub fn poison_active_regions() {
    #[cfg(not(feature = "no-gc"))]
    {
        let depth = region_stack_depth();
        if depth == 0 {
            return;
        }
        POISON_WATERMARK.with(|w| w.set(w.get().max(depth)));
        crate::stats::GC_STATS.record_region_poison();
    }
}

/// True when every active region must remain alive after its scope closes.
/// Publishers can skip scans because no value can escape into freed regions.
pub fn active_regions_are_poisoned() -> bool {
    #[cfg(feature = "no-gc")]
    {
        REGION_STACK.with(|stack| {
            let stack = stack.borrow();
            !stack.is_empty() && stack.iter().all(|&region| unsafe { &*region }.published)
        })
    }
    #[cfg(not(feature = "no-gc"))]
    {
        let depth = region_stack_depth();
        depth != 0 && POISON_WATERMARK.with(|watermark| watermark.get() >= depth)
    }
}

/// Close a region whose scope has ended: pop it from the thread-local stack,
/// then either drop it (running destructors, freeing memory) or — if it was
/// poisoned — retire it.
///
/// All owners of stack-registered regions (the rt_abi bridge, the IR
/// interpreter, scratch guards) must close through here so the poison
/// protocol is honoured.
#[cfg(not(feature = "no-gc"))]
pub fn close_region(mut region: Box<Region>) {
    let depth = region_stack_depth();
    pop_region_guard();
    let poisoned = POISON_WATERMARK.with(|w| {
        if depth != 0 && depth <= w.get() {
            w.set(depth - 1);
            true
        } else {
            false
        }
    });
    if poisoned {
        tracing::debug!(
            target: "gc",
            "retiring poisoned region ({} objects, {} bytes)",
            region.object_count(),
            region.bytes_used()
        );
        region.retire();
        RETIRED_REGIONS.with(|r| r.borrow_mut().push(*region));
    }
    // else: `region` drops here — destructors run, chunks are freed.
}

#[cfg(feature = "no-gc")]
static NEXT_SCOPE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[cfg(feature = "no-gc")]
struct LeaseOwner {
    id: u64,
    region: *mut Region,
    closed: std::cell::Cell<bool>,
    external: std::cell::Cell<usize>,
    permanent: std::cell::Cell<bool>,
    dependents: RefCell<std::collections::BTreeMap<u64, std::rc::Weak<LeaseOwner>>>,
    dependencies: RefCell<std::collections::BTreeMap<u64, std::rc::Weak<LeaseOwner>>>,
    dependency_counts: RefCell<std::collections::BTreeMap<u64, usize>>,
    slots: RefCell<std::collections::BTreeMap<(usize, SlotId), RegionDependency>>,
}
#[cfg(feature = "no-gc")]
impl Drop for LeaseOwner {
    fn drop(&mut self) {
        // Remove both halves before the last strong reference disappears.
        // Otherwise Weak entries keep metadata allocations alive indefinitely.
        for source in self
            .dependencies
            .get_mut()
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            source.dependents.borrow_mut().remove(&self.id);
        }
        for dependent in self
            .dependents
            .get_mut()
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            dependent.dependencies.borrow_mut().remove(&self.id);
            dependent.dependency_counts.borrow_mut().remove(&self.id);
        }
    }
}
#[cfg(feature = "no-gc")]
impl Region {
    fn owner(&mut self) -> std::rc::Rc<LeaseOwner> {
        if let Some(owner) = &self.lease {
            return owner.clone();
        }
        let owner = std::rc::Rc::new(LeaseOwner {
            id: self.id,
            region: self,
            closed: std::cell::Cell::new(false),
            external: std::cell::Cell::new(0),
            permanent: std::cell::Cell::new(false),
            dependents: RefCell::new(Default::default()),
            dependencies: RefCell::new(Default::default()),
            dependency_counts: RefCell::new(Default::default()),
            slots: RefCell::new(Default::default()),
        });
        self.lease = Some(owner.clone());
        for chunk in &self.chunks {
            index_chunk(chunk, &owner);
        }
        owner
    }
    pub(crate) fn is_permanent(&self) -> bool {
        self.published
    }
    fn contains(&self, address: usize) -> bool {
        self.chunks.iter().any(|chunk| {
            let base = chunk.data.as_ptr() as usize;
            address >= base && address < base + chunk.layout.size()
        })
    }
}
#[cfg(feature = "no-gc")]
thread_local! {
    // Boxes keep allocation targets stable after their lexical scope closes.
    static LEASED_REGIONS: RefCell<std::collections::BTreeMap<u64, Box<Region>>> = const { RefCell::new(std::collections::BTreeMap::new()) };
    static INHERITED_LEASES: RefCell<Vec<RegionLease>> = const { RefCell::new(Vec::new()) };
    static STATIC_SLOTS: RefCell<std::collections::BTreeMap<(usize, SlotId), RegionLease>> = const { RefCell::new(std::collections::BTreeMap::new()) };
    static READ_SCOPES: RefCell<Vec<RegionLease>> = const { RefCell::new(Vec::new()) };
    static LEASE_SWEEPING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    // Isolate-local address index shared by its fibers; entries do not own regions.
    static REGION_OWNERS: RefCell<std::collections::BTreeMap<usize, (usize, std::rc::Weak<LeaseOwner>)>> = const { RefCell::new(std::collections::BTreeMap::new()) };
}
#[cfg(feature = "no-gc")]
crate::mutator_slot!("cljrs-gc/INHERITED_LEASES", refcell INHERITED_LEASES: Vec<RegionLease>);
#[cfg(feature = "no-gc")]
crate::mutator_slot!("cljrs-gc/READ_SCOPES", refcell READ_SCOPES: Vec<RegionLease>);

/// Identifies one independently replaceable field of a managed object.
#[cfg(feature = "no-gc")]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum SlotId {
    Index(usize),
    Named(String),
}
#[cfg(feature = "no-gc")]
impl From<usize> for SlotId {
    fn from(value: usize) -> Self {
        Self::Index(value)
    }
}
#[cfg(feature = "no-gc")]
impl From<String> for SlotId {
    fn from(value: String) -> Self {
        Self::Named(value)
    }
}

/// Weak cached backing scopes for a region-owned container.
#[cfg(feature = "no-gc")]
#[derive(Default, Debug)]
pub struct WeakRegionLease(Vec<std::rc::Weak<LeaseOwner>>);
#[cfg(feature = "no-gc")]
impl WeakRegionLease {
    pub fn upgrade(&self) -> RegionLease {
        RegionLease::new(self.0.iter().filter_map(std::rc::Weak::upgrade).collect())
    }
}

/// Borrows made during one top-level evaluation, released after its result escapes.
#[cfg(feature = "no-gc")]
pub struct ReadScope(bool, std::marker::PhantomData<std::rc::Rc<()>>);
#[cfg(feature = "no-gc")]
impl ReadScope {
    pub fn enter() -> Option<Self> {
        if Self::is_active() || crate::alloc_ctx::invocation_is_active() {
            return None;
        }
        READ_SCOPES.with(|scopes| scopes.borrow_mut().push(Default::default()));
        Some(Self(true, std::marker::PhantomData))
    }
    pub fn nested() -> Self {
        READ_SCOPES.with(|scopes| scopes.borrow_mut().push(Default::default()));
        Self(true, std::marker::PhantomData)
    }
    pub fn is_active() -> bool {
        READ_SCOPES.with(|scopes| !scopes.borrow().is_empty())
    }
    pub fn finish(mut self) -> RegionLease {
        self.0 = false;
        READ_SCOPES.with(|scopes| scopes.borrow_mut().pop().expect("balanced read scope"))
    }
}
#[cfg(feature = "no-gc")]
impl Drop for ReadScope {
    fn drop(&mut self) {
        if self.0 {
            let lease = READ_SCOPES.with(|scopes| scopes.borrow_mut().pop());
            drop(lease);
        }
    }
}

/// Read leases owned by one asynchronous task, independent of the polling caller.
#[cfg(feature = "no-gc")]
pub struct ReadContext(Vec<RegionLease>);
#[cfg(feature = "no-gc")]
impl Default for ReadContext {
    fn default() -> Self {
        Self(vec![RegionLease::default()])
    }
}
#[cfg(feature = "no-gc")]
impl ReadContext {
    pub fn enter(&mut self) -> ReadContextGuard<'_> {
        READ_SCOPES.with(|scopes| std::mem::swap(&mut *scopes.borrow_mut(), &mut self.0));
        ReadContextGuard(self)
    }
}
#[cfg(feature = "no-gc")]
pub struct ReadContextGuard<'a>(&'a mut ReadContext);
#[cfg(feature = "no-gc")]
impl Drop for ReadContextGuard<'_> {
    fn drop(&mut self) {
        READ_SCOPES.with(|scopes| std::mem::swap(&mut *scopes.borrow_mut(), &mut self.0.0));
    }
}

/// Temporary ownership of scratch scopes. Empty until a scope is shared.
#[cfg(feature = "no-gc")]
#[derive(Default)]
pub struct RegionLease(
    Vec<std::rc::Rc<LeaseOwner>>,
    std::collections::BTreeSet<u64>,
);
#[cfg(feature = "no-gc")]
impl std::fmt::Debug for RegionLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.0.iter().map(|owner| owner.id))
            .finish()
    }
}
#[cfg(feature = "no-gc")]
impl Clone for RegionLease {
    fn clone(&self) -> Self {
        Self::new(self.0.clone())
    }
}
#[cfg(feature = "no-gc")]
impl RegionLease {
    fn new(owners: Vec<std::rc::Rc<LeaseOwner>>) -> Self {
        for owner in &owners {
            owner.external.set(owner.external.get() + 1);
        }
        let ids = owners.iter().map(|owner| owner.id).collect();
        Self(owners, ids)
    }
    pub fn capture_active() -> Self {
        let mut owners = INHERITED_LEASES.with(|stack| {
            stack
                .borrow()
                .iter()
                .flat_map(|lease| lease.0.iter().cloned())
                .collect::<Vec<_>>()
        });
        for ptr in live_regions() {
            // SAFETY: both scope registries contain live, stable regions.
            owners.push(unsafe { &mut *ptr }.owner());
        }
        owners.sort_by_key(|owner| owner.id);
        owners.dedup_by_key(|owner| owner.id);
        Self::new(owners)
    }
    pub fn for_value(value: &impl Trace) -> Self {
        crate::leases_for(value)
    }
    /// Lease the scopes reached by several values with one visited set.
    pub fn for_values<T: Trace>(values: &[T]) -> Self {
        struct Values<'a, T>(&'a [T]);
        impl<T: Trace> Trace for Values<'_, T> {
            fn trace(&self, visitor: &mut crate::MarkVisitor) {
                for value in self.0 {
                    value.trace(visitor);
                }
            }
        }
        Self::for_value(&Values(values))
    }
    pub(crate) fn add_address(&mut self, address: usize) {
        if let Some(owner) = find_owner(address)
            && !owner.permanent.get()
            && self.1.insert(owner.id)
        {
            owner.external.set(owner.external.get() + 1);
            self.0.push(owner);
        }
    }
    pub fn downgrade(&self) -> WeakRegionLease {
        WeakRegionLease(self.0.iter().map(std::rc::Rc::downgrade).collect())
    }
    pub fn extend(&mut self, lease: &Self) {
        for owner in &lease.0 {
            if self.1.insert(owner.id) {
                owner.external.set(owner.external.get() + 1);
                self.0.push(owner.clone());
            }
        }
    }
    /// Borrow a value only within an explicit read or scratch scope.
    pub fn protect_read(&self) {
        assert!(
            self.0.is_empty() || ReadScope::is_active() || !crate::alloc_ctx::is_static_context(),
            "a borrowed no-gc value requires ReadScope; use an owning snapshot outside evaluation"
        );
        self.protect_current();
    }
    pub fn protect_current(&self) {
        if self.0.is_empty() {
            return;
        }
        if READ_SCOPES.with(|scopes| {
            if let Some(scope) = scopes.borrow_mut().last_mut() {
                scope.extend(self);
                true
            } else {
                false
            }
        }) {
            return;
        }
        AllocationOwner::current().adopt(self);
    }
    pub fn enter(&self) -> LeaseScope {
        INHERITED_LEASES.with(|stack| stack.borrow_mut().push(self.clone()));
        LeaseScope(std::marker::PhantomData)
    }
}
/// Weak ownership of a value's destination scope; does not pin its own container.
#[cfg(feature = "no-gc")]
#[derive(Clone, Default)]
pub struct AllocationOwner(std::rc::Weak<LeaseOwner>);
#[cfg(feature = "no-gc")]
impl std::fmt::Debug for AllocationOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AllocationOwner")
    }
}
#[cfg(feature = "no-gc")]
impl AllocationOwner {
    pub fn current() -> Self {
        let owner = crate::alloc_ctx::current_region().map(|ptr| unsafe { &mut *ptr }.owner());
        Self(
            owner
                .as_ref()
                .map(std::rc::Rc::downgrade)
                .unwrap_or_default(),
        )
    }
    pub fn for_address(address: usize) -> Self {
        Self(
            find_owner(address)
                .as_ref()
                .map(std::rc::Rc::downgrade)
                .unwrap_or_default(),
        )
    }
    pub fn is_permanent(&self) -> bool {
        self.0.upgrade().is_none_or(|owner| owner.permanent.get())
    }
    pub fn protect(&self, value: &impl Trace) {
        self.adopt(&RegionLease::for_value(value));
    }
    /// Replace the ownership of one mutable object slot.
    pub fn set_slot(&self, address: usize, slot: impl Into<SlotId>, lease: &RegionLease) {
        let key = (address, slot.into());
        if self.is_permanent() {
            let previous = STATIC_SLOTS.with(|slots| {
                let mut slots = slots.borrow_mut();
                if lease.0.is_empty() {
                    slots.remove(&key)
                } else {
                    slots.insert(key, lease.clone())
                }
            });
            drop(previous);
        } else if let Some(owner) = self.0.upgrade() {
            let edges = self.replaceable(lease);
            let previous = {
                let mut slots = owner.slots.borrow_mut();
                if lease.0.is_empty() {
                    slots.remove(&key)
                } else {
                    slots.insert(key, edges)
                }
            };
            drop(previous);
        }
    }
    /// Release every cached field when its containing object is destroyed.
    pub fn clear_slots(&self, address: usize) {
        if self.is_permanent() {
            let removed = STATIC_SLOTS.try_with(|slots| {
                let mut slots = slots.borrow_mut();
                let keys = slots
                    .range((address, SlotId::Index(0))..)
                    .take_while(|((owner, _), _)| *owner == address)
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| slots.remove(&key))
                    .collect::<Vec<_>>()
            });
            drop(removed);
        } else if let Some(owner) = self.0.upgrade() {
            let removed = {
                let mut slots = owner.slots.borrow_mut();
                let keys = slots
                    .range((address, SlotId::Index(0))..)
                    .take_while(|((owner, _), _)| *owner == address)
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                keys.into_iter()
                    .filter_map(|key| slots.remove(&key))
                    .collect::<Vec<_>>()
            };
            drop(removed);
        }
    }
    /// Borrow a cached slot value into the caller's read scope.
    pub fn read_slot(&self, address: usize, slot: impl Into<SlotId>) {
        let key = (address, slot.into());
        let lease = if self.is_permanent() {
            STATIC_SLOTS.with(|slots| slots.borrow().get(&key).cloned())
        } else {
            self.0.upgrade().and_then(|owner| {
                owner
                    .slots
                    .borrow()
                    .get(&key)
                    .map(RegionDependency::snapshot)
            })
        };
        if let Some(lease) = lease {
            lease.protect_read();
        }
    }
    /// Store backing scopes for the destination's full lifetime.
    pub fn adopt(&self, lease: &RegionLease) {
        if let Some(owner) = self.0.upgrade() {
            if owner.permanent.get() {
                for source in &lease.0 {
                    retain_owner(source);
                }
                return;
            }
            let mut dependencies = owner.dependencies.borrow_mut();
            for source in &lease.0 {
                if source.id == owner.id || source.permanent.get() {
                    continue;
                }
                owner
                    .dependency_counts
                    .borrow_mut()
                    .insert(source.id, usize::MAX);
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    dependencies.entry(source.id)
                {
                    entry.insert(std::rc::Rc::downgrade(source));
                    source
                        .dependents
                        .borrow_mut()
                        .insert(owner.id, std::rc::Rc::downgrade(&owner));
                }
            }
        } else {
            // A static sink has program lifetime. Keep only actual backing scopes.
            for source in &lease.0 {
                retain_owner(source);
            }
        }
    }
}
/// Replaceable ownership edges. Dropping a slot releases only its own edges.
#[cfg(feature = "no-gc")]
#[derive(Default, Debug)]
pub struct RegionDependency {
    destination: AllocationOwner,
    sources: WeakRegionLease,
}
#[cfg(feature = "no-gc")]
impl RegionDependency {
    pub fn snapshot(&self) -> RegionLease {
        self.sources.upgrade()
    }
}
#[cfg(feature = "no-gc")]
impl AllocationOwner {
    pub fn replaceable(&self, lease: &RegionLease) -> RegionDependency {
        let Some(owner) = self.0.upgrade() else {
            return RegionDependency::default();
        };
        for source in &lease.0 {
            if source.id == owner.id || source.permanent.get() {
                continue;
            }
            let mut counts = owner.dependency_counts.borrow_mut();
            let count = counts.entry(source.id).or_default();
            if *count != usize::MAX {
                *count += 1;
            }
            owner
                .dependencies
                .borrow_mut()
                .insert(source.id, std::rc::Rc::downgrade(source));
            source
                .dependents
                .borrow_mut()
                .insert(owner.id, std::rc::Rc::downgrade(&owner));
        }
        RegionDependency {
            destination: self.clone(),
            sources: lease.downgrade(),
        }
    }
}
#[cfg(feature = "no-gc")]
impl Drop for RegionDependency {
    fn drop(&mut self) {
        let Some(owner) = self.destination.0.upgrade() else {
            return;
        };
        let mut released = Vec::new();
        for source in self.sources.0.iter().filter_map(std::rc::Weak::upgrade) {
            if source.id == owner.id {
                continue;
            }
            let mut counts = owner.dependency_counts.borrow_mut();
            let Some(count) = counts.get_mut(&source.id) else {
                continue;
            };
            if *count == usize::MAX {
                continue;
            }
            *count -= 1;
            if *count == 0 {
                counts.remove(&source.id);
                owner.dependencies.borrow_mut().remove(&source.id);
                source.dependents.borrow_mut().remove(&owner.id);
                released.push(source);
            }
        }
        collect_leased_regions(released);
    }
}

/// Captured allocation destination for a fiber, separate from its live roots.
#[cfg(feature = "no-gc")]
#[derive(Clone, Default)]
pub struct AllocationSnapshot(RegionLease);
#[cfg(feature = "no-gc")]
impl AllocationSnapshot {
    pub fn capture() -> Self {
        Self(RegionLease::new(
            crate::alloc_ctx::current_region()
                .map(|ptr| unsafe { &mut *ptr }.owner())
                .into_iter()
                .collect(),
        ))
    }
    pub fn enter(&self) -> crate::alloc_ctx::ContextGuard {
        crate::alloc_ctx::enter_target(self.0.0.first().map(|owner| owner.region), self.0.clone())
    }
    pub fn protect(&self, value: &impl Trace) {
        AllocationOwner(
            self.0
                .0
                .first()
                .map(std::rc::Rc::downgrade)
                .unwrap_or_default(),
        )
        .protect(value);
    }
}
#[cfg(feature = "no-gc")]
pub struct LeaseScope(std::marker::PhantomData<std::rc::Rc<()>>);
#[cfg(feature = "no-gc")]
impl Drop for LeaseScope {
    fn drop(&mut self) {
        let lease = INHERITED_LEASES.with(|stack| stack.borrow_mut().pop());
        drop(lease);
    }
}
#[cfg(feature = "no-gc")]
impl Drop for RegionLease {
    fn drop(&mut self) {
        let mut released = Vec::new();
        for owner in &self.0 {
            let remaining = owner
                .external
                .get()
                .checked_sub(1)
                .expect("balanced scope leases");
            owner.external.set(remaining);
            if remaining == 0 && owner.closed.get() {
                released.push(owner.clone());
            }
        }
        self.0.clear();
        collect_leased_regions(released);
    }
}

// Follow only the released scopes' connected ownership paths. Dependency
// edges are weak; lexical scopes and external task/result leases are roots.
#[cfg(feature = "no-gc")]
fn collect_leased_regions(seeds: Vec<std::rc::Rc<LeaseOwner>>) {
    if seeds.is_empty()
        || LEASE_SWEEPING
            .try_with(|flag| flag.replace(true))
            .unwrap_or(true)
    {
        return;
    }
    struct Sweep;
    impl Drop for Sweep {
        fn drop(&mut self) {
            let _ = LEASE_SWEEPING.try_with(|flag| flag.set(false));
        }
    }
    let _sweep = Sweep;
    fn rooted(owner: &LeaseOwner) -> bool {
        !owner.closed.get() || owner.external.get() != 0 || owner.permanent.get()
    }
    let mut owners = std::collections::BTreeMap::new();
    // A direct live owner already keeps this entire outgoing graph alive.
    // Return-call scopes usually have exactly this shape; avoid traversing
    // their captured value graphs on every call or lazy-sequence step.
    let mut pending = seeds
        .into_iter()
        .filter(|owner| {
            !rooted(owner)
                && !owner
                    .dependents
                    .borrow()
                    .values()
                    .filter_map(std::rc::Weak::upgrade)
                    .any(|parent| rooted(&parent))
        })
        .collect::<Vec<_>>();
    while let Some(owner) = pending.pop() {
        if owners.contains_key(&owner.id) {
            continue;
        }
        if !rooted(&owner) {
            pending.extend(
                owner
                    .dependencies
                    .borrow()
                    .values()
                    .filter_map(std::rc::Weak::upgrade),
            );
        }
        owners.insert(owner.id, owner);
    }
    let candidates = owners.keys().copied().collect::<Vec<_>>();
    // Incoming paths reveal roots outside the candidate subgraph. Stop at a
    // root so unrelated descendants of long-lived scopes are never scanned.
    let mut pending = candidates.clone();
    while let Some(id) = pending.pop() {
        let owner = owners[&id].clone();
        if rooted(&owner) {
            continue;
        }
        for parent in owner
            .dependents
            .borrow()
            .values()
            .filter_map(std::rc::Weak::upgrade)
        {
            owners.entry(parent.id).or_insert_with(|| {
                pending.push(parent.id);
                parent
            });
        }
    }
    // Build only the incoming edges discovered for non-root candidates.
    // Scanning every dependency of a live caller would revisit all earlier
    // loop results whenever one small child scope closes.
    let mut outgoing: std::collections::BTreeMap<u64, Vec<u64>> = Default::default();
    for owner in owners.values().filter(|owner| !rooted(owner)) {
        for parent in owner
            .dependents
            .borrow()
            .keys()
            .filter(|id| owners.contains_key(id))
        {
            outgoing.entry(*parent).or_default().push(owner.id);
        }
    }
    let mut pending = owners
        .values()
        .filter(|owner| rooted(owner))
        .map(|owner| owner.id)
        .collect::<Vec<_>>();
    let mut reachable = std::collections::BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !reachable.insert(id) {
            continue;
        }
        if let Some(children) = outgoing.get(&id) {
            pending.extend(children.iter().copied());
        }
    }
    let Ok(dead) = LEASED_REGIONS.try_with(|slot| {
        let mut regions = slot.borrow_mut();
        candidates
            .into_iter()
            .filter(|id| !reachable.contains(id))
            .filter_map(|id| regions.remove(&id))
            .collect::<Vec<_>>()
    }) else {
        return;
    };
    drop(owners);
    drop(dead);
}

#[cfg(feature = "no-gc")]
pub fn close_region(region: Box<Region>) {
    pop_region_guard();
    close_scratch_region(region);
}
#[cfg(feature = "no-gc")]
pub(crate) fn close_scratch_region(mut region: Box<Region>) {
    if let Some(owner) = &region.lease {
        owner.closed.set(true);
    }
    if region.published {
        region.retire();
        RETIRED_REGIONS.with(|slot| slot.borrow_mut().push(region));
    } else if region.lease.is_some() {
        let owner = region.lease.as_ref().unwrap().clone();
        region.retire();
        LEASED_REGIONS.with(|slot| slot.borrow_mut().insert(region.id, region));
        collect_leased_regions(vec![owner]);
    }
}

#[cfg(feature = "no-gc")]
fn live_regions() -> Vec<*mut Region> {
    let mut regions = crate::alloc_ctx::live_regions();
    let _ = REGION_STACK.try_with(|stack| regions.extend(stack.borrow().iter().copied()));
    regions
}
#[cfg(feature = "no-gc")]
fn find_owner(address: usize) -> Option<std::rc::Rc<LeaseOwner>> {
    let indexed = REGION_OWNERS
        .try_with(|index| {
            let index = index.borrow();
            let (_, (end, owner)) = index.range(..=address).next_back()?;
            (address < *end).then(|| owner.upgrade()).flatten()
        })
        .ok()
        .flatten();
    if indexed.is_some() {
        return indexed;
    }
    // A fresh scope has no sharing metadata or address entries yet.
    for ptr in live_regions() {
        let region = unsafe { &mut *ptr };
        if region.contains(address) {
            return Some(region.owner());
        }
    }
    None
}
#[cfg(feature = "no-gc")]
fn index_chunk(chunk: &Chunk, owner: &std::rc::Rc<LeaseOwner>) {
    let base = chunk.data.as_ptr() as usize;
    REGION_OWNERS.with(|index| {
        index.borrow_mut().insert(
            base,
            (base + chunk.layout.size(), std::rc::Rc::downgrade(owner)),
        );
    });
}
#[cfg(feature = "no-gc")]
fn unindex_chunk(chunk: &Chunk) {
    let _ = REGION_OWNERS.try_with(|index| {
        index.borrow_mut().remove(&(chunk.data.as_ptr() as usize));
    });
}

#[cfg(feature = "no-gc")]
fn retain_owner(owner: &std::rc::Rc<LeaseOwner>) {
    // The owner reference pins this box in either a live scope or the deferred table.
    owner.permanent.set(true);
    unsafe { &mut *owner.region }.published = true;
    let deferred = LEASED_REGIONS.with(|slot| slot.borrow_mut().remove(&owner.id));
    if let Some(region) = deferred {
        RETIRED_REGIONS.with(|slot| slot.borrow_mut().push(region));
    }
}
#[cfg(feature = "no-gc")]
pub(crate) fn retain_containing(address: usize) {
    if let Some(owner) = find_owner(address) {
        retain_owner(&owner);
    }
}

/// Trace every retired region's objects (their heap children must stay
/// alive).  Called by `GcHeap::collect` alongside [`trace_active_regions`].
#[cfg(not(feature = "no-gc"))]
pub(crate) fn trace_retired_regions(visitor: &mut crate::MarkVisitor) {
    RETIRED_REGIONS.with(|r| {
        for region in r.borrow().iter() {
            region.trace_live(visitor);
        }
    });
}

/// Trace every object in every region on this thread's active region stack,
/// marking the GC-heap objects they reference.
///
/// Invoked by `GcHeap::collect` during root scanning so that region→heap
/// references are honoured even though the mark phase treats region objects
/// themselves as opaque (see `MarkVisitor::visit`).
///
/// Scanning only the *current* thread's regions is complete by design: each
/// `GcHeap` lives on a single OS thread, regions are thread-local, and region
/// objects are only ever bump-allocated into the region at the top of *this*
/// thread's stack (`try_alloc_in_region`).  A region is freed when it leaves
/// the stack, so every live region able to hold a pointer into this thread's
/// heap is exactly one of the regions iterated here.  (clojurust uses parallel
/// per-thread heaps with independent collections rather than a cross-thread
/// stop-the-world GC, so there is no other thread's heap to consider.)
#[cfg(not(feature = "no-gc"))]
pub(crate) fn trace_active_regions(visitor: &mut crate::MarkVisitor) {
    REGION_STACK.with(|stack| {
        for &region_ptr in stack.borrow().iter() {
            // SAFETY: `RegionGuard`/`push_region_raw` guarantee every pointer
            // on the stack refers to a live `Region`.
            let region = unsafe { &*region_ptr };
            region.trace_live(visitor);
        }
    });
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
    #[cfg(feature = "no-gc")]
    {
        Some(crate::alloc_ctx::alloc_in_ctx(value))
    }
    #[cfg(not(feature = "no-gc"))]
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
    fn fresh_nested_regions_are_not_covered_by_an_outer_poison() {
        assert!(!active_regions_are_poisoned());
        let mut outer = Box::new(Region::new());
        unsafe { push_region_raw(outer.as_mut() as *mut Region) };
        assert!(!active_regions_are_poisoned());
        poison_active_regions();
        assert!(active_regions_are_poisoned());
        let mut inner = Box::new(Region::new());
        unsafe { push_region_raw(inner.as_mut() as *mut Region) };
        assert!(
            !active_regions_are_poisoned(),
            "fresh regions still need a publish scan"
        );
        close_region(inner);
        assert!(active_regions_are_poisoned());
        close_region(outer);
        assert!(!active_regions_are_poisoned());
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
            let p = region.alloc(i);
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
    fn gc_skips_region_objects_from_heap_parent() {
        // A GC-heap parent holds a GcPtr to a region-allocated child.  The
        // marker must treat the region child as opaque (skip it), not trace
        // into it — region objects are not heap-managed.  Marking must
        // succeed and the parent must survive.
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Region::new();
        let child = region.alloc(Tracked {
            id: 1,
            dropped: dropped.clone(),
        });
        assert!(child.is_region_alloc(), "region alloc must be tagged");

        // The parent is on the GC heap, pointing to a region-allocated child.
        let heap = crate::GcHeap::new();
        let parent = heap.alloc(Parent {
            child: child.clone(),
        });

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
    fn gc_does_not_follow_reset_region_pointer() {
        // The exact crash this fix targets: a GC-heap object references a
        // region object, the region is reset (its memory freed/reused), and a
        // later GC marks the heap object.  The dangling region pointer must be
        // skipped (via its provenance tag), not dereferenced.
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let heap = crate::GcHeap::new();

        let mut region = Region::new();
        let region_child = region.alloc(Tracked {
            id: 1,
            dropped: dropped.clone(),
        });
        let parent = heap.alloc(Parent {
            child: region_child,
        });

        // Reset the region: `region_child`'s backing storage is now dead.
        region.reset();

        // Marking the parent must not touch the dangling region pointer.
        heap.collect(|vis| {
            use crate::GcVisitor as _;
            vis.visit(&parent);
        });
        assert_eq!(
            heap.count(),
            1,
            "parent survives; no crash on dangling region ptr"
        );
    }

    #[test]
    fn active_region_keeps_heap_child_alive() {
        // A region object holds a GcPtr to a heap object that is otherwise
        // unreachable.  While the region is active it is traced as a GC root,
        // so the heap child must survive collection.
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let heap = crate::GcHeap::new();
        let heap_child = heap.alloc(Tracked {
            id: 9,
            dropped: dropped.clone(),
        });

        let mut region = Region::new();
        let _parent = region.alloc(Parent { child: heap_child });
        let _guard = unsafe { RegionGuard::new(&mut region) };

        // Two collections (one to exhaust the grace period) with no explicit
        // roots: the heap child survives only because the active region is a
        // root.
        heap.collect(|_vis| {});
        heap.collect(|_vis| {});
        assert!(
            dropped.lock().unwrap().is_empty(),
            "heap object reachable only through an active region must survive GC"
        );
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
    fn close_region_resets_when_not_poisoned() {
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Box::new(Region::new());
        region.alloc(Tracked {
            id: 1,
            dropped: dropped.clone(),
        });
        unsafe { push_region_raw(region.as_mut() as *mut Region) };
        close_region(region);
        assert_eq!(*dropped.lock().unwrap(), vec![1], "destructor must run");
        assert!(!region_is_active());
    }

    #[test]
    fn poisoned_region_is_retired_not_reset() {
        // A publish barrier hit an opaque value while this region was open:
        // the region must be kept alive (its objects remain valid) rather
        // than reset when its scope closes.
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Box::new(Region::new());
        let p = region.alloc(Tracked {
            id: 7,
            dropped: dropped.clone(),
        });
        unsafe { push_region_raw(region.as_mut() as *mut Region) };
        poison_active_regions();
        close_region(region);
        assert!(
            dropped.lock().unwrap().is_empty(),
            "poisoned region must not run destructors"
        );
        // The object is still readable — retirement means the memory lives on.
        assert_eq!(p.get().id, 7);
        assert!(!region_is_active());

        // A region opened *after* the poison closes normally.
        let dropped2 = Arc::new(Mutex::new(Vec::new()));
        let mut r2 = Box::new(Region::new());
        r2.alloc(Tracked {
            id: 9,
            dropped: dropped2.clone(),
        });
        unsafe { push_region_raw(r2.as_mut() as *mut Region) };
        close_region(r2);
        assert_eq!(*dropped2.lock().unwrap(), vec![9]);
    }

    #[test]
    fn poison_with_no_active_region_is_a_no_op() {
        poison_active_regions();
        let dropped = Arc::new(Mutex::new(Vec::new()));
        let mut region = Box::new(Region::new());
        region.alloc(Tracked {
            id: 3,
            dropped: dropped.clone(),
        });
        unsafe { push_region_raw(region.as_mut() as *mut Region) };
        close_region(region);
        assert_eq!(*dropped.lock().unwrap(), vec![3]);
    }

    fn account_region_bytes() -> (usize, usize) {
        crate::governor::with_current_account(|a| (a.region_bytes(), a.retired_region_bytes()))
            .unwrap()
    }

    #[test]
    fn region_chunks_are_charged_to_the_thread_account() {
        let (before, _) = account_region_bytes();
        let mut region = Region::with_capacity(128);
        assert_eq!(account_region_bytes().0 - before, region.chunk_bytes());
        for i in 0..100 {
            region.alloc(i);
        }
        assert!(region.chunks.len() > 1);
        assert_eq!(account_region_bytes().0 - before, region.chunk_bytes());
        region.reset();
        assert_eq!(region.chunks.len(), 1);
        assert_eq!(account_region_bytes().0 - before, region.chunk_bytes());
        drop(region);
        assert_eq!(account_region_bytes().0, before);
    }

    #[test]
    fn retired_region_keeps_its_charge_as_retired() {
        let (active_before, retired_before) = account_region_bytes();
        let mut region = Box::new(Region::new());
        region.alloc(1i64);
        let bytes = region.chunk_bytes();
        unsafe { push_region_raw(region.as_mut() as *mut Region) };
        poison_active_regions();
        close_region(region);
        let (active, retired) = account_region_bytes();
        assert_eq!(active, active_before);
        assert_eq!(retired - retired_before, bytes);
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
        // No timing assertion: wall-clock comparisons are unreliable on shared
        // CI machines.  The eprintln above captures the numbers for inspection.
    }
}

#[cfg(all(test, feature = "no-gc"))]
mod lease_tests {
    use super::*;
    struct Probe(std::rc::Rc<std::cell::Cell<usize>>);
    impl crate::Trace for Probe {
        fn trace(&self, _: &mut crate::MarkVisitor) {}
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    #[test]
    fn released_dependents_remove_their_weak_edges() {
        struct Edge(crate::GcPtr<Probe>);
        impl crate::Trace for Edge {
            fn trace(&self, visitor: &mut crate::MarkVisitor) {
                use crate::GcVisitor;
                visitor.visit(&self.0);
            }
        }
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let _parent = crate::alloc_ctx::ScratchGuard::new();
        let value = crate::GcPtr::new(Probe(drops));
        let parent = AllocationOwner::current().0.upgrade().unwrap();
        for _ in 0..200 {
            let child = crate::alloc_ctx::ScratchGuard::new();
            AllocationOwner::current().protect(&Edge(value.clone()));
            assert_eq!(parent.dependents.borrow().len(), 1);
            drop(child);
            assert!(parent.dependents.borrow().is_empty());
            assert!(parent.dependencies.borrow().is_empty());
        }
        assert_eq!(
            std::rc::Rc::weak_count(&parent),
            1,
            "only the chunk address index remains"
        );
    }

    #[test]
    fn mutually_dependent_closed_scopes_live_until_the_last_external_lease() {
        struct Edge(crate::GcPtr<Probe>);
        impl crate::Trace for Edge {
            fn trace(&self, visitor: &mut crate::MarkVisitor) {
                use crate::GcVisitor;
                visitor.visit(&self.0);
            }
        }
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let a = crate::alloc_ctx::ScratchGuard::new();
        let value_a = crate::GcPtr::new(Probe(drops.clone()));
        let owner_a = AllocationOwner::current();
        let lease_a = RegionLease::capture_active();
        drop(a);
        let b = crate::alloc_ctx::ScratchGuard::new();
        let value_b = crate::GcPtr::new(Probe(drops.clone()));
        let owner_b = AllocationOwner::current();
        let lease_b = RegionLease::capture_active();
        drop(b);
        owner_a.protect(&Edge(value_b.clone()));
        owner_b.protect(&Edge(value_a.clone()));
        drop(lease_a);
        assert_eq!(drops.get(), 0);
        assert_eq!(value_a.get().0.get(), 0);
        drop(lease_b);
        assert_eq!(drops.get(), 2);
    }

    #[test]
    fn allocation_guard_pins_a_closed_destination_and_its_later_chunks() {
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let scratch = crate::alloc_ctx::ScratchGuard::new();
        let _value = crate::GcPtr::new(Probe(drops.clone()));
        let snapshot = AllocationSnapshot::capture();
        drop(scratch);
        let context = snapshot.enter();
        drop(snapshot);
        for _ in 0..2000 {
            let _ = crate::GcPtr::new(Probe(drops.clone()));
        }
        assert_eq!(drops.get(), 0);
        drop(context);
        assert_eq!(drops.get(), 2001);
    }
    #[test]
    fn popped_scratch_cannot_receive_dynamic_or_static_allocations() {
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let outer = crate::alloc_ctx::ScratchGuard::new();
        let mut inner = crate::alloc_ctx::ScratchGuard::new();
        let _temporary = crate::GcPtr::new(Probe(drops.clone()));
        inner.pop_for_return();
        let returned = unsafe { try_alloc_in_region(Probe(drops.clone())) }.unwrap();
        let permanent = {
            let _static_scope = crate::alloc_ctx::StaticCtxGuard::new();
            unsafe { try_alloc_in_region(Probe(drops.clone())) }.unwrap()
        };
        drop(inner);
        assert_eq!(drops.get(), 1);
        assert_eq!(returned.get().0.get(), 1);
        assert!(permanent.is_static_alloc());
        drop(outer);
        assert_eq!(drops.get(), 2);
        assert_eq!(permanent.get().0.get(), 2);
    }
    #[test]
    fn closed_scratch_scope_is_reclaimed_after_its_last_task_lease() {
        let drops = std::rc::Rc::new(std::cell::Cell::new(0));
        let scratch = crate::alloc_ctx::ScratchGuard::new();
        let value = crate::GcPtr::new(Probe(drops.clone()));
        let lease = RegionLease::capture_active();
        let inherited = {
            let _scope = lease.enter();
            RegionLease::capture_active()
        };
        drop(scratch);
        drop(lease);
        assert_eq!(drops.get(), 0);
        assert_eq!(value.get().0.get(), 0);
        drop(value);
        drop(inherited);
        assert_eq!(drops.get(), 1);
    }
}

#[cfg(feature = "no-gc")]
pub(crate) fn is_permanent_address(address: usize) -> bool {
    find_owner(address).is_some_and(|owner| unsafe { &*owner.region }.published)
}
