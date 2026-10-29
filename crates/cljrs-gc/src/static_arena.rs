//! Global static arena for no-gc mode.
//!
//! Values allocated here live for the duration of the program. This is the
//! correct target for top-level `def`/`defn` values, namespace interns, `Atom`
//! initial values, and anything else that must outlive all scratch regions.
//!
//! The arena is a thread-safe bump allocator backed by a chain of
//! `Box<[u8]>`-backed chunks.  Destructors for allocated values are intentionally
//! NOT run — values stored here are semantically immutable program-lifetime data.
//!
//! ## Debug provenance (Phase 7)
//!
//! The arena tracks its chunk address ranges in every no-gc build.  [`is_static_addr`] checks
//! whether a raw pointer came from the arena, which lets the write-site
//! assertions in `Atom::reset` / `Var::bind` catch region-local values being
//! stored in program-lifetime containers.

use std::alloc::{self, Layout};
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use crate::governor::MemoryLimitExceeded;
use crate::{AllocError, GcBox, GcPtr, Trace};

/// Default chunk size: 256 KiB.
const CHUNK_SIZE: usize = 256 * 1024;

struct Chunk {
    data: NonNull<u8>,
    layout: Layout,
}

// SAFETY: accessed only under the arena's Mutex.
unsafe impl Send for Chunk {}

struct Inner {
    chunks: Vec<Chunk>,
    /// Current bump pointer (byte address).
    ptr: usize,
    /// One-past-end of the active chunk.
    end: usize,
}

/// Report an arena chunk to the governor.  Arena chunks are never freed.
fn charge_chunk(bytes: usize) {
    crate::governor::governor().charge_permanent(crate::governor::MemoryClass::Static, bytes);
}

impl Inner {
    fn new() -> Self {
        let layout = Layout::from_size_align(CHUNK_SIZE, 16).unwrap();
        // SAFETY: non-zero size, valid layout.
        let data = unsafe {
            NonNull::new(alloc::alloc(layout))
                .expect("StaticArena: initial chunk allocation failed")
        };
        let base = data.as_ptr() as usize;
        charge_chunk(CHUNK_SIZE);
        Self {
            chunks: vec![Chunk { data, layout }],
            ptr: base,
            end: base + CHUNK_SIZE,
        }
    }

    fn alloc_raw(&mut self, layout: Layout) -> *mut u8 {
        let align = layout.align();
        let size = layout.size();
        let aligned = (self.ptr + align - 1) & !(align - 1);
        let new_ptr = aligned + size;
        if new_ptr <= self.end {
            self.ptr = new_ptr;
            return aligned as *mut u8;
        }
        self.grow(layout)
    }

    /// [`Self::alloc_raw`], but a new chunk must first be admitted by the
    /// governor; on refusal nothing is allocated or charged.
    fn try_alloc_raw(&mut self, layout: Layout) -> Result<*mut u8, MemoryLimitExceeded> {
        let aligned = (self.ptr + layout.align() - 1) & !(layout.align() - 1);
        if aligned + layout.size() <= self.end {
            self.ptr = aligned + layout.size();
            return Ok(aligned as *mut u8);
        }
        crate::governor::governor().try_charge_permanent(
            crate::governor::MemoryClass::Static,
            Self::chunk_size(layout),
        )?;
        Ok(self.grow_charged(layout))
    }

    fn chunk_size(layout: Layout) -> usize {
        CHUNK_SIZE.max(layout.size() * 2)
    }

    fn grow(&mut self, layout: Layout) -> *mut u8 {
        charge_chunk(Self::chunk_size(layout));
        self.grow_charged(layout)
    }

    /// Add a chunk whose bytes the governor has already charged.
    fn grow_charged(&mut self, layout: Layout) -> *mut u8 {
        let chunk_size = Self::chunk_size(layout);
        let cl = Layout::from_size_align(chunk_size, layout.align().max(16)).unwrap();
        // SAFETY: non-zero size, valid layout.
        let data = unsafe {
            NonNull::new(alloc::alloc(cl)).expect("StaticArena: chunk allocation failed")
        };
        let base = data.as_ptr() as usize;
        let aligned = (base + layout.align() - 1) & !(layout.align() - 1);
        self.ptr = aligned + layout.size();
        self.end = base + chunk_size;
        self.chunks.push(Chunk { data, layout: cl });
        aligned as *mut u8
    }

    /// Check whether `addr` falls inside any chunk owned by this arena.
    fn contains_addr(&self, addr: usize) -> bool {
        for chunk in &self.chunks {
            let base = chunk.data.as_ptr() as usize;
            let end = base + chunk.layout.size();
            if addr >= base && addr < end {
                return true;
            }
        }
        false
    }
}

/// Thread-safe bump allocator whose memory is never reclaimed.
pub struct StaticArena {
    inner: Mutex<Inner>,
}

// SAFETY: inner is Mutex-protected.
unsafe impl Sync for StaticArena {}

impl StaticArena {
    fn new() -> Self {
        Self {
            inner: Mutex::new(Inner::new()),
        }
    }

    /// Allocate `value` into the arena. Its destructor will never be called.
    pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T> {
        let layout = Layout::new::<GcBox<T>>();
        let raw = self.inner.lock().unwrap().alloc_raw(layout);
        let gc_box = raw as *mut GcBox<T>;
        // SAFETY: raw is properly aligned and sized for GcBox<T>.
        // We intentionally omit running Drop on the value — static lifetime.
        unsafe { std::ptr::write(gc_box, GcBox::new(value)) };
        let pointer = GcPtr(unsafe { NonNull::new_unchecked(gc_box) });
        pointer.get().init_ownership();
        pointer
    }

    /// Allocate `value` directly (no `GcBox` wrapper) and return a
    /// [`StaticGcPtr`].
    ///
    /// Use this when the caller needs a `Send + Sync` program-lifetime pointer
    /// but does **not** need `GcPtr` interoperability — e.g. keyword/symbol
    /// intern tables.  The allocation is never freed.
    pub fn alloc_val<T: 'static>(&self, value: T) -> crate::StaticGcPtr<T> {
        let layout = std::alloc::Layout::new::<T>();
        let raw = self.inner.lock().unwrap().alloc_raw(layout) as *mut T;
        // SAFETY: raw is properly aligned and sized for T.
        unsafe { std::ptr::write(raw, value) };
        crate::StaticGcPtr(unsafe { NonNull::new_unchecked(raw) })
    }

    /// Fallible [`Self::alloc`]: the governor is asked only when the arena
    /// needs a new chunk, and a refusal returns the value.
    pub fn try_alloc<T: Trace + 'static>(&self, value: T) -> Result<GcPtr<T>, AllocError<T>> {
        let layout = Layout::new::<GcBox<T>>();
        let raw = match self.inner.lock().unwrap().try_alloc_raw(layout) {
            Ok(raw) => raw,
            Err(error) => return Err(AllocError { value, error }),
        };
        let gc_box = raw as *mut GcBox<T>;
        // SAFETY: raw is properly aligned and sized for GcBox<T>.
        unsafe { std::ptr::write(gc_box, GcBox::new(value)) };
        let pointer = GcPtr(unsafe { NonNull::new_unchecked(gc_box) });
        pointer.get().init_ownership();
        Ok(pointer)
    }

    /// Fallible [`Self::alloc_val`].
    pub fn try_alloc_val<T: 'static>(
        &self,
        value: T,
    ) -> Result<crate::StaticGcPtr<T>, AllocError<T>> {
        let layout = Layout::new::<T>();
        let raw = match self.inner.lock().unwrap().try_alloc_raw(layout) {
            Ok(raw) => raw as *mut T,
            Err(error) => return Err(AllocError { value, error }),
        };
        // SAFETY: raw is properly aligned and sized for T.
        unsafe { std::ptr::write(raw, value) };
        Ok(crate::StaticGcPtr(unsafe { NonNull::new_unchecked(raw) }))
    }

    /// Return `true` if `addr` was allocated by this arena.
    ///
    /// Used by publication and provenance checks to distinguish static-arena
    /// pointers from scratch-region pointers.
    pub fn contains_addr(&self, addr: usize) -> bool {
        self.inner.lock().unwrap().contains_addr(addr)
    }
}

static STATIC_ARENA: OnceLock<StaticArena> = OnceLock::new();

/// Return the global static arena singleton.
pub fn static_arena() -> &'static StaticArena {
    STATIC_ARENA.get_or_init(StaticArena::new)
}

/// Allocate `value` into the global static arena (no `GcBox` wrapper) and
/// return a [`StaticGcPtr`].  Convenience wrapper around
/// `static_arena().alloc_val(value)`.
pub fn static_alloc_val<T: 'static>(value: T) -> crate::StaticGcPtr<T> {
    static_arena().alloc_val(value)
}

/// Return `true` if the raw pointer address was allocated by the static arena.
///
/// This is an O(chunks) scan used by no-gc publication and provenance checks.
pub fn is_static_addr(addr: usize) -> bool {
    // If the arena has not been initialised yet, no allocation has ever been
    // made, so the address cannot be static.
    match STATIC_ARENA.get() {
        Some(arena) => arena.contains_addr(addr),
        None => false,
    }
}
