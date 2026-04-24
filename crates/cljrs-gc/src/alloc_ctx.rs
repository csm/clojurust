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
use crate::{GcPtr, Trace};

pub(crate) enum AllocCtx {
    /// Allocate in the global static arena (program-lifetime).
    Static,
    /// Allocate in the given bump region.
    Region(*mut Region),
}

// SAFETY: AllocCtx values are only accessed from their owning thread.
unsafe impl Send for AllocCtx {}

thread_local! {
    /// Thread-local allocation context stack.
    /// Empty ≡ Static (program-startup default).
    pub(crate) static ALLOC_CTX: RefCell<Vec<AllocCtx>> = const { RefCell::new(Vec::new()) };
}

/// Allocate `value` in the currently active allocation context.
pub(crate) fn alloc_in_ctx<T: Trace + 'static>(value: T) -> GcPtr<T> {
    ALLOC_CTX.with(|ctx| {
        let ctx = ctx.borrow();
        match ctx.last() {
            None | Some(AllocCtx::Static) => static_arena().alloc(value),
            Some(AllocCtx::Region(ptr)) => {
                // SAFETY: The Region pointer is valid while the owning ScratchGuard is live.
                unsafe { &mut **ptr }.alloc(value)
            }
        }
    })
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
    region: Box<Region>,
    /// Whether the ctx entry is currently on the stack.
    in_ctx: bool,
}

impl ScratchGuard {
    pub fn new() -> Self {
        let mut region = Box::new(Region::new());
        let ptr = region.as_mut() as *mut Region;
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().push(AllocCtx::Region(ptr)));
        Self {
            region,
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
        self.region.reset();
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
pub struct StaticCtxGuard;

impl StaticCtxGuard {
    pub fn new() -> Self {
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().push(AllocCtx::Static));
        Self
    }
}

impl Default for StaticCtxGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for StaticCtxGuard {
    fn drop(&mut self) {
        ALLOC_CTX.with(|ctx| ctx.borrow_mut().pop());
    }
}
