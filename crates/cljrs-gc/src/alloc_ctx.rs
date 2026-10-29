//! Thread-local allocation context stack for no-gc mode.
//!
//! [`GcPtr::new`] always dispatches to the top entry of [`ALLOC_CTX`].
//!
//! * When the stack is empty (or the top entry is `Static`), allocations
//!   go to the global [`StaticArena`] and live for the program lifetime.
//! * When the top entry is `Region(ptr)`, allocations go into that bump region
//!   and are freed when the owning [`ScratchGuard`] drops.
//!
//! The evaluator pushes a [`ScratchGuard`] on every function call and every
//! `loop` iteration, and pops it (via [`ScratchGuard::pop_for_return`]) before
//! evaluating the tail (return) expression so that the return value lands in the
//! caller's active context.
//!
//! [`StaticCtxGuard`] temporarily forces allocations into the `StaticArena`
//! regardless of the enclosing region — used for `def`, `atom`, `reset!`, and
//! `swap!` value expressions.

use std::cell::RefCell;

use crate::region::Region;
use crate::static_arena::static_arena;
use crate::{AllocError, GcBox, GcPtr, Trace};
use std::alloc::Layout;

pub(crate) enum AllocCtx {
    /// Allocate in the global static arena (program-lifetime).
    Static,
    /// Allocate in the given bump region.
    Region(*mut Region),
    /// One arena owns every allocation made during an isolated invocation.
    /// Nested function/loop scratch guards and static-sink guards become
    /// no-ops while this entry is active.
    Invocation(*mut Region),
}

// SAFETY: AllocCtx values are only accessed from their owning thread.
unsafe impl Send for AllocCtx {}

thread_local! {
    /// Thread-local allocation context stack.
    /// Empty ≡ Static (program-startup default).
    pub(crate) static ALLOC_CTX: RefCell<Vec<AllocCtx>> = const { RefCell::new(Vec::new()) };
}
crate::mutator_slot!("cljrs-gc/ALLOC_CTX", refcell ALLOC_CTX: Vec<AllocCtx>);

/// Allocate `value` in the currently active allocation context.
pub(crate) fn alloc_in_ctx<T: Trace + 'static>(value: T) -> GcPtr<T> {
    ALLOC_CTX.with(|ctx| {
        let ctx = ctx.borrow();
        match ctx.last() {
            None | Some(AllocCtx::Static) => static_arena().alloc(value),
            Some(AllocCtx::Region(ptr) | AllocCtx::Invocation(ptr)) => {
                // SAFETY: The Region pointer is valid while the owning ScratchGuard is live.
                unsafe { &mut **ptr }.alloc(value)
            }
        }
    })
}

/// Fallible [`alloc_in_ctx`]: the governor is asked only when the active
/// region or the static arena needs a new chunk, so an allocation that fits
/// in memory already charged is never refused.
pub(crate) fn try_alloc_in_ctx<T: Trace + 'static>(value: T) -> Result<GcPtr<T>, AllocError<T>> {
    ALLOC_CTX.with(|ctx| {
        let ctx = ctx.borrow();
        match ctx.last() {
            None | Some(AllocCtx::Static) => static_arena().try_alloc(value),
            Some(AllocCtx::Region(ptr) | AllocCtx::Invocation(ptr)) => {
                // SAFETY: The Region pointer is valid while the owning ScratchGuard is live.
                let region = unsafe { &mut **ptr };
                if let Some(bytes) = region.chunk_needed(Layout::new::<GcBox<T>>())
                    && let Err(error) = crate::governor::try_reserve_current(bytes)
                {
                    return Err(AllocError { value, error });
                }
                Ok(region.alloc(value))
            }
        }
    })
}

pub fn invocation_is_active() -> bool {
    ALLOC_CTX.with(|ctx| {
        ctx.borrow()
            .iter()
            .any(|entry| matches!(entry, AllocCtx::Invocation(_)))
    })
}

/// Owns the single bounded arena used by a GC-less isolated invocation.
///
/// Every `GcPtr::new` within the guard's dynamic extent lands in this arena.
/// Existing evaluator scratch/static guards deliberately do not change that
/// routing, which gives every value the same lifetime and permits arbitrary
/// internal object graphs. Values must be serialized or otherwise copied out
/// before this guard drops.
pub struct InvocationGuard {
    region: Box<Region>,
    in_ctx: bool,
}

impl InvocationGuard {
    pub fn new(byte_limit: usize) -> Self {
        assert!(
            !invocation_is_active(),
            "nested isolated allocation invocations are not supported"
        );
        let mut region = Box::new(Region::with_limit(byte_limit));
        let ptr = region.as_mut() as *mut Region;
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().push(AllocCtx::Invocation(ptr)));
        Self {
            region,
            in_ctx: true,
        }
    }

    pub fn accounted_bytes(&self) -> usize {
        self.region.accounted_bytes()
    }

    pub fn object_count(&self) -> usize {
        self.region.object_count()
    }
}

impl Drop for InvocationGuard {
    fn drop(&mut self) {
        if self.in_ctx {
            ALLOC_CTX.with(|ctx| {
                let popped = ctx.borrow_mut().pop();
                debug_assert!(matches!(popped, Some(AllocCtx::Invocation(_))));
            });
            self.in_ctx = false;
        }
        self.region.reset();
    }
}

// ── ScratchGuard ──────────────────────────────────────────────────────────────

/// Pushes a fresh [`Region`] onto the allocation context stack.
///
/// Used by the evaluator at function-call boundaries and on every `loop`
/// iteration.  All allocations made between `new()` and `pop_for_return()` land
/// in the owned scratch region and are freed when this guard drops.
///
/// The "return-expression-in-caller" protocol:
/// 1. Create `ScratchGuard` → allocations enter the scratch region.
/// 2. Evaluate all non-tail body forms (intermediates land in scratch).
/// 3. Call `pop_for_return()` → the scratch is removed from the active context.
/// 4. Evaluate the tail (return) expression → lands in the caller's context.
/// 5. `ScratchGuard` drops → scratch memory is reset (intermediates freed).
pub struct ScratchGuard {
    region: Option<Box<Region>>,
    /// Whether the ctx entry is currently on the stack.
    in_ctx: bool,
}

impl ScratchGuard {
    pub fn new() -> Self {
        if invocation_is_active() {
            return Self {
                region: None,
                in_ctx: false,
            };
        }
        let mut region = Box::new(Region::new());
        let ptr = region.as_mut() as *mut Region;
        // Register the live scope even after pop_for_return removes its
        // allocation context. A spawned task can capture its tail locals.
        // SAFETY: the boxed region stays alive until close_region in Drop.
        std::mem::forget(unsafe { crate::region::RegionGuard::new(&mut region) });
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().push(AllocCtx::Region(ptr)));
        Self {
            region: Some(region),
            in_ctx: true,
        }
    }

    /// Remove the scratch region from the active allocation context.
    ///
    /// After this call, new allocations land in whatever is now at the top of
    /// the stack — the caller's active context.  The scratch region's memory
    /// is still live and readable until `self` drops.
    pub fn pop_for_return(&mut self) {
        if self.in_ctx {
            ALLOC_CTX.with(|ctx| ctx.borrow_mut().pop());
            self.in_ctx = false;
        }
    }
}

impl Default for ScratchGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ScratchGuard {
    fn drop(&mut self) {
        // Pop from ctx if pop_for_return was not yet called.
        if self.in_ctx {
            ALLOC_CTX.with(|ctx| ctx.borrow_mut().pop());
        }
        // Reset the region: runs destructors on all contained values and
        // reclaims bump-allocator memory for reuse.
        if let Some(region) = self.region.take() {
            crate::region::close_region(region);
        }
    }
}

// ── StaticCtxGuard ────────────────────────────────────────────────────────────

/// Temporarily forces allocations into the global `StaticArena`.
///
/// Used for expressions that must produce program-lifetime values regardless
/// of the enclosing scratch region:
/// - top-level `def` / `defn` value expressions
/// - `atom` / `Var` initializers
/// - `reset!` / `vreset!` new-value expressions
/// - the function passed to `swap!` / `vswap!`
pub struct StaticCtxGuard {
    pushed: bool,
}

impl StaticCtxGuard {
    pub fn new() -> Self {
        if invocation_is_active() {
            return Self { pushed: false };
        }
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().push(AllocCtx::Static));
        Self { pushed: true }
    }
}

impl Default for StaticCtxGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StaticCtxGuard {
    fn drop(&mut self) {
        if self.pushed {
            ALLOC_CTX.with(|ctx| ctx.borrow_mut().pop());
        }
    }
}
