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
//! Under `cfg(all(feature = "no-gc", debug_assertions))` the arena maintains a
//! registry of all addresses it has ever allocated.  [`is_static_addr`] checks
//! whether a raw pointer came from the arena, which lets the write-site
//! assertions in `Atom::reset` / `Var::bind` catch region-local values being
//! stored in program-lifetime containers.

use std::alloc::{self, Layout};
use std::ptr::NonNull;
use std::sync::{Mutex, OnceLock};

use crate::{GcBox, GcPtr, Trace};

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

impl Inner {
    fn new() -> Self {
        let layout = Layout::from_size_align(CHUNK_SIZE, 16).unwrap();
        // SAFETY: non-zero size, valid layout.
        let data = unsafe {
            NonNull::new(alloc::alloc(layout))
                .expect("StaticArena: initial chunk allocation failed")
        };
        let base = data.as_ptr() as usize;
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

    fn grow(&mut self, layout: Layout) -> *mut u8 {
        let chunk_size = CHUNK_SIZE.max(layout.size() * 2);
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
    #[cfg(debug_assertions)]
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
        unsafe { std::ptr::write(gc_box, GcBox { value }) };
        GcPtr(unsafe { NonNull::new_unchecked(gc_box) })
    }

    /// Return `true` if `addr` was allocated by this arena.
    ///
    /// Used by debug-mode provenance assertions to distinguish static-arena
    /// pointers from scratch-region pointers.
    #[cfg(debug_assertions)]
    pub fn contains_addr(&self, addr: usize) -> bool {
        self.inner.lock().unwrap().contains_addr(addr)
    }
}

static STATIC_ARENA: OnceLock<StaticArena> = OnceLock::new();

/// Return the global static arena singleton.
pub fn static_arena() -> &'static StaticArena {
    STATIC_ARENA.get_or_init(StaticArena::new)
}

/// Return `true` if the raw pointer address was allocated by the static arena.
///
/// This is an O(chunks) scan (typically 1–2 chunks) and is only intended for
/// use in `debug_assert!` write-site checks.
#[cfg(debug_assertions)]
pub fn is_static_addr(addr: usize) -> bool {
    // If the arena has not been initialised yet, no allocation has ever been
    // made, so the address cannot be static.
    match STATIC_ARENA.get() {
        Some(arena) => arena.contains_addr(addr),
        None => false,
    }
}
