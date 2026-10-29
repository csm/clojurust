//! Thread-local allocation context stack for no-gc mode.
//!
//! [`GcPtr::new`] always dispatches to the top entry of [`ALLOC_CTX`].
//!
//! * When the stack is empty (or the top entry is `Static`), allocations
//!   go to the global [`StaticArena`] and live for the program lifetime.
//! * When the top entry is `Region(ptr)`, allocations go into that bump region
//!   and are freed when the owning [`ScratchGuard`] drops.
//!
//! The evaluator pushes a [`ScratchGuard`] on each function call and loop
//! iteration. Before the guard drops, it deactivates the scratch destination and
//! transfers the result's backing scopes into the caller's allocation owner.
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
    /// A live tail scope that is no longer an allocation destination.
    Inactive(*mut Region),
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
        match ctx
            .iter()
            .rev()
            .find(|entry| !matches!(entry, AllocCtx::Inactive(_)))
        {
            Some(AllocCtx::Inactive(_)) => unreachable!(),
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
        match ctx
            .iter()
            .rev()
            .find(|entry| !matches!(entry, AllocCtx::Inactive(_)))
        {
            Some(AllocCtx::Inactive(_)) => unreachable!(),
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
/// `pop_for_return()` deactivates allocation while keeping the scope discoverable.
/// Callers then protect the result's backing scopes in the caller's context.
/// The guard frees unshared storage when it drops.
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
            ALLOC_CTX.with(|ctx| {
                let mut ctx = ctx.borrow_mut();
                let Some(AllocCtx::Region(ptr)) = ctx.last_mut() else {
                    panic!("scratch context out of order")
                };
                let ptr = *ptr;
                *ctx.last_mut().unwrap() = AllocCtx::Inactive(ptr);
            });
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
        if let Some(region) = self.region.take() {
            ALLOC_CTX.with(|ctx| {
                ctx.borrow_mut().pop();
            });
            crate::region::close_scratch_region(region);
        }
    }
}

/// Install a captured allocation destination for one fiber or task poll.
pub struct ContextGuard {
    _owner: crate::region::RegionLease,
}
impl Drop for ContextGuard {
    fn drop(&mut self) {
        ALLOC_CTX.with(|ctx| {
            ctx.borrow_mut().pop();
        });
    }
}
pub(crate) fn enter_target(
    region: Option<*mut Region>,
    owner: crate::region::RegionLease,
) -> ContextGuard {
    ALLOC_CTX.with(|ctx| {
        ctx.borrow_mut()
            .push(region.map(AllocCtx::Region).unwrap_or(AllocCtx::Static))
    });
    ContextGuard { _owner: owner }
}
pub(crate) fn current_region() -> Option<*mut Region> {
    ALLOC_CTX.with(|ctx| {
        ctx.borrow()
            .iter()
            .rev()
            .find_map(|entry| match entry {
                AllocCtx::Inactive(_) => None,
                _ => Some(match entry {
                    AllocCtx::Region(ptr) | AllocCtx::Invocation(ptr) => Some(*ptr),
                    _ => None,
                }),
            })
            .flatten()
    })
}
pub(crate) fn live_regions() -> Vec<*mut Region> {
    ALLOC_CTX.with(|ctx| {
        ctx.borrow()
            .iter()
            .filter_map(|entry| match entry {
                AllocCtx::Region(ptr) | AllocCtx::Inactive(ptr) | AllocCtx::Invocation(ptr) => {
                    Some(*ptr)
                }
                _ => None,
            })
            .collect()
    })
}
pub(crate) fn ancestor_ids(target: *mut Region) -> Vec<u64> {
    ALLOC_CTX.with(|ctx| {
        ctx.borrow()
            .iter()
            .take_while(|entry| match entry {
                AllocCtx::Region(ptr) | AllocCtx::Inactive(ptr) | AllocCtx::Invocation(ptr) => {
                    *ptr != target
                }
                _ => true,
            })
            .filter_map(|entry| match entry {
                AllocCtx::Region(ptr) | AllocCtx::Invocation(ptr) => {
                    Some(unsafe { &**ptr }.scope_id())
                }
                _ => None,
            })
            .collect()
    })
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
