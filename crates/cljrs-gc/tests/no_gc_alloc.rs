//! Integration tests for the no-gc allocation context stack.
//!
//! Verifies `ScratchGuard`, `StaticCtxGuard`, and the "return-expression-in-caller"
//! protocol described in `docs/no-gc-plan.md`.
//!
//! Run with:
//!   cargo test -p cljrs-gc --features no-gc

#![cfg(feature = "no-gc")]

use std::sync::{Arc, Mutex};

use cljrs_gc::alloc_ctx::{ScratchGuard, StaticCtxGuard};
use cljrs_gc::{GcPtr, MarkVisitor, Trace};

// ── Helper ────────────────────────────────────────────────────────────────────

/// A value that records whether its destructor has run.
struct DropTracked {
    dropped: Arc<Mutex<bool>>,
}

impl Drop for DropTracked {
    fn drop(&mut self) {
        *self.dropped.lock().unwrap() = true;
    }
}

impl Trace for DropTracked {
    fn trace(&self, _: &mut MarkVisitor) {}
}

// ── Default-context tests ─────────────────────────────────────────────────────

#[test]
fn default_ctx_allocates_in_static_arena() {
    // With no guard active the default context is static.
    let p = GcPtr::new(42i64);
    assert_eq!(*p.get(), 42);
    #[cfg(debug_assertions)]
    assert!(
        p.is_static_alloc(),
        "allocation with no active guard should go to StaticArena"
    );
}

#[test]
fn static_ctx_guard_allocates_in_static_arena() {
    let _g = StaticCtxGuard::new();
    let p = GcPtr::new(99i64);
    assert_eq!(*p.get(), 99);
    #[cfg(debug_assertions)]
    assert!(p.is_static_alloc());
}

// ── ScratchGuard tests ────────────────────────────────────────────────────────

#[test]
fn scratch_guard_allocates_in_region() {
    let _scratch = ScratchGuard::new();
    let p = GcPtr::new(42i64);
    assert_eq!(*p.get(), 42);
    #[cfg(debug_assertions)]
    assert!(
        !p.is_static_alloc(),
        "allocation inside ScratchGuard should go to Region, not StaticArena"
    );
}

#[test]
fn pop_for_return_restores_outer_static_ctx() {
    // Simulates the "return-expression-in-caller" protocol when the caller's
    // context is static (no enclosing guard).
    let mut scratch = ScratchGuard::new();

    #[cfg(debug_assertions)]
    {
        let inside = GcPtr::new(1i64);
        assert!(
            !inside.is_static_alloc(),
            "allocation before pop should be in the scratch region"
        );
    }

    scratch.pop_for_return();

    // After popping, new allocations land in the caller's context (static here).
    let after = GcPtr::new(2i64);
    #[cfg(debug_assertions)]
    assert!(
        after.is_static_alloc(),
        "allocation after pop_for_return should land in the caller's (static) context"
    );
    assert_eq!(*after.get(), 2);
}

#[test]
fn pop_for_return_is_idempotent() {
    // Calling pop_for_return more than once must not panic.
    let mut scratch = ScratchGuard::new();
    scratch.pop_for_return();
    scratch.pop_for_return(); // second call is a no-op
}

#[test]
fn drop_without_pop_restores_context() {
    {
        let _scratch = ScratchGuard::new();
        // Drop without calling pop_for_return.
    }
    // After the guard drops, the context must be restored to static.
    #[cfg(debug_assertions)]
    assert!(
        GcPtr::new(1i64).is_static_alloc(),
        "after ScratchGuard drop (no pop_for_return), context should revert to static"
    );
}

// ── StaticCtxGuard override tests ─────────────────────────────────────────────

#[test]
fn static_ctx_guard_overrides_enclosing_scratch_region() {
    let _scratch = ScratchGuard::new();

    #[cfg(debug_assertions)]
    assert!(!GcPtr::new(1i64).is_static_alloc());

    let _static_ctx = StaticCtxGuard::new();

    #[cfg(debug_assertions)]
    assert!(
        GcPtr::new(2i64).is_static_alloc(),
        "StaticCtxGuard pushed inside a ScratchGuard should route allocations to StaticArena"
    );
}

#[test]
fn static_ctx_guard_drop_restores_enclosing_scratch() {
    let _scratch = ScratchGuard::new();

    {
        let _static_ctx = StaticCtxGuard::new();
        #[cfg(debug_assertions)]
        assert!(GcPtr::new(1i64).is_static_alloc());
    }

    // After StaticCtxGuard drops, the enclosing ScratchGuard should be active again.
    #[cfg(debug_assertions)]
    assert!(
        !GcPtr::new(2i64).is_static_alloc(),
        "after StaticCtxGuard drop, allocations should return to the enclosing scratch region"
    );
}

// ── Nesting tests ─────────────────────────────────────────────────────────────

#[test]
fn nested_scratch_guards_maintain_stack_order() {
    let _outer = ScratchGuard::new();

    #[cfg(debug_assertions)]
    assert!(!GcPtr::new(1i64).is_static_alloc());

    {
        let _inner = ScratchGuard::new();
        #[cfg(debug_assertions)]
        assert!(!GcPtr::new(2i64).is_static_alloc());
    }

    // After the inner guard drops, the outer region should still be active.
    #[cfg(debug_assertions)]
    assert!(
        !GcPtr::new(3i64).is_static_alloc(),
        "after inner ScratchGuard drop, outer scratch region should still be active"
    );
}

#[test]
fn static_ctx_between_nested_scratches() {
    let _outer = ScratchGuard::new();
    {
        let _inner = ScratchGuard::new();
        {
            let _static_ctx = StaticCtxGuard::new();
            #[cfg(debug_assertions)]
            assert!(GcPtr::new(1i64).is_static_alloc());
        }
        // Back to inner scratch.
        #[cfg(debug_assertions)]
        assert!(!GcPtr::new(2i64).is_static_alloc());
    }
    // Back to outer scratch.
    #[cfg(debug_assertions)]
    assert!(!GcPtr::new(3i64).is_static_alloc());
}

// ── Destructor tests ──────────────────────────────────────────────────────────

#[test]
fn scratch_region_runs_destructors_on_drop() {
    let dropped = Arc::new(Mutex::new(false));
    {
        let _scratch = ScratchGuard::new();
        let _p = GcPtr::new(DropTracked {
            dropped: dropped.clone(),
        });
        assert!(
            !*dropped.lock().unwrap(),
            "destructor should not run while region is alive"
        );
    }
    // ScratchGuard drops → region.reset() → destructor runs.
    assert!(
        *dropped.lock().unwrap(),
        "destructor should run when ScratchGuard drops"
    );
}

#[test]
fn pop_for_return_does_not_run_destructors() {
    let dropped = Arc::new(Mutex::new(false));
    let mut scratch = ScratchGuard::new();
    let _p = GcPtr::new(DropTracked {
        dropped: dropped.clone(),
    });

    // pop_for_return removes scratch from the context stack but does NOT reset memory.
    scratch.pop_for_return();
    assert!(
        !*dropped.lock().unwrap(),
        "pop_for_return should not run destructors; memory must remain readable for the tail expression"
    );

    // The destructor runs when the guard drops.
    drop(scratch);
    assert!(
        *dropped.lock().unwrap(),
        "destructor should run when ScratchGuard finally drops"
    );
}

// ── Full return-value protocol ────────────────────────────────────────────────

#[test]
fn return_value_protocol_allocates_in_caller_context() {
    // Simulate a function call: push scratch, evaluate body in scratch,
    // pop scratch, evaluate tail (return expression) in caller's context.
    //
    // Outer context: another scratch region (simulates a loop iteration's region).
    let _outer_scratch = ScratchGuard::new();

    let return_value = {
        let mut fn_scratch = ScratchGuard::new();

        // Body allocation — an intermediate that should be freed.
        let _intermediate = GcPtr::new(100i64);
        #[cfg(debug_assertions)]
        assert!(!_intermediate.is_static_alloc());

        // Pop scratch before evaluating the tail expression.
        fn_scratch.pop_for_return();

        // Tail expression: lands in the outer scratch (caller's context).
        let ret = GcPtr::new(42i64);
        #[cfg(debug_assertions)]
        assert!(
            !ret.is_static_alloc(),
            "return value should land in the outer scratch (caller's region), not static arena"
        );

        // fn_scratch drops here, resetting intermediate memory.
        ret
    };

    assert_eq!(*return_value.get(), 42);
}
