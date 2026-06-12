#[cfg(not(feature = "no-gc"))]
use crate::dynamics;
use crate::env::Env;
#[cfg(not(feature = "no-gc"))]
use crate::env::GlobalEnv;
use std::cell::RefCell;

// ── Stop-the-world reclaim hooks (JIT code unloading, cold-IR sweep) ────────
//
// Reclamation of execution-engine caches runs only at a stop-the-world
// safepoint, when every mutator thread is parked and active JIT frames can be
// scanned safely.  GC collection is the existing STW point, so interested
// tiers install hooks here that run at the tail of every collection while the
// STW guard is still held.  Current registrants: `cljrs-jit` (superseded
// native modules) and `cljrs-eval`'s lowering worker (idle Tier-1 IR,
// Phase 10.7).

type StwReclaimHook = Box<dyn Fn() + Send + Sync + 'static>;
static STW_RECLAIM_HOOKS: std::sync::RwLock<Vec<StwReclaimHook>> =
    std::sync::RwLock::new(Vec::new());

/// Register a stop-the-world reclaim hook.  Multiple hooks may be registered;
/// each runs at every STW point, in registration order.
///
/// Hooks run inside the STW guard after each collection, so they may assume
/// all other mutator threads are parked.
pub fn set_stw_reclaim_hook(f: impl Fn() + Send + Sync + 'static) {
    STW_RECLAIM_HOOKS.write().unwrap().push(Box::new(f));
}

/// Run the STW reclaim hooks, if any.  Caller must hold the STW guard.
#[cfg(not(feature = "no-gc"))]
fn run_stw_reclaim() {
    for hook in STW_RECLAIM_HOOKS.read().unwrap().iter() {
        hook();
    }
}

// ── Thread-local Env root registry ──────────────────────────────────────────
//
// When the interpreter enters a function call, the caller's Env stays on the
// Rust stack but the callee creates a fresh Env.  If GC triggers inside the
// callee, only the callee's Env is passed to `gc_safepoint`.  To keep the
// caller's local bindings alive we maintain a thread-local stack of pointers
// to all active Envs on this thread's call stack.
//
// SAFETY: the raw pointers are valid during STW collection because:
// - The collecting thread's own Envs are in earlier (still-live) stack frames.
// - Other threads are parked at safepoints; their stacks (and Envs) are frozen.

thread_local! {
    static ENV_ROOTS: RefCell<Vec<*const Env>> = const { RefCell::new(Vec::new()) };
    /// Shadow stack of Value pointers on the Rust call stack that need to
    /// survive GC.  Each entry is a `(ptr, count)` pair pointing to a
    /// contiguous slice of Values (e.g., a Vec's backing storage or a single
    /// Value on the stack).
    static VALUE_ROOTS: RefCell<Vec<(*const cljrs_value::Value, usize)>> =
        const { RefCell::new(Vec::new()) };
    /// Shadow stack for `Option<Value>` slices (e.g., the IR interpreter's
    /// register file).  Each entry is `(ptr, count)` pointing to a fixed-size
    /// heap slice whose address will not change for the lifetime of the entry.
    static OPTION_VALUE_ROOTS: RefCell<Vec<(*const Option<cljrs_value::Value>, usize)>> =
        const { RefCell::new(Vec::new()) };
}

/// RAII guard that pops the Env pointer on drop.
pub struct EnvRootGuard;

impl Drop for EnvRootGuard {
    fn drop(&mut self) {
        ENV_ROOTS.with(|roots| {
            roots.borrow_mut().pop();
        });
    }
}

/// RAII guard that pops one entry from the value shadow stack on drop.
pub struct ValueRootGuard {
    pushed: bool,
}

impl Drop for ValueRootGuard {
    fn drop(&mut self) {
        if self.pushed {
            VALUE_ROOTS.with(|roots| {
                roots.borrow_mut().pop();
            });
        }
    }
}

/// Register an Env as a GC root for the duration of its use.
/// Returns a guard that unregisters on drop.
pub fn push_env_root(env: &Env) -> EnvRootGuard {
    ENV_ROOTS.with(|roots| {
        roots.borrow_mut().push(env as *const Env);
    });
    EnvRootGuard
}

/// Register a single Value as a GC root.
pub fn root_value(val: &cljrs_value::Value) -> ValueRootGuard {
    VALUE_ROOTS.with(|roots| {
        roots
            .borrow_mut()
            .push((val as *const cljrs_value::Value, 1));
    });
    ValueRootGuard { pushed: true }
}

/// Register a slice of Values as GC roots (e.g., a Vec<Value>).
pub fn root_values(vals: &[cljrs_value::Value]) -> ValueRootGuard {
    if vals.is_empty() {
        return ValueRootGuard { pushed: false };
    }
    VALUE_ROOTS.with(|roots| {
        roots.borrow_mut().push((vals.as_ptr(), vals.len()));
    });
    ValueRootGuard { pushed: true }
}

/// RAII guard that pops one entry from the option-value shadow stack on drop.
pub struct OptionValueRootGuard {
    pushed: bool,
}

impl Drop for OptionValueRootGuard {
    fn drop(&mut self) {
        if self.pushed {
            OPTION_VALUE_ROOTS.with(|roots| {
                roots.borrow_mut().pop();
            });
        }
    }
}

/// Register a slice of `Option<Value>` as GC roots.
///
/// The caller **must** ensure the slice's heap address is stable for the
/// lifetime of the returned guard — use `Box<[Option<Value>]>` rather than
/// a `Vec` that could reallocate.
pub fn root_option_values(vals: &[Option<cljrs_value::Value>]) -> OptionValueRootGuard {
    if vals.is_empty() {
        return OptionValueRootGuard { pushed: false };
    }
    OPTION_VALUE_ROOTS.with(|roots| {
        roots.borrow_mut().push((vals.as_ptr(), vals.len()));
    });
    OptionValueRootGuard { pushed: true }
}

/// Force an immediate GC collection, bypassing the memory-pressure threshold.
///
/// Unlike `gc_safepoint`, this always initiates collection regardless of
/// `gc_requested()`. Use this after removing namespaces from globals to ensure
/// their closures and form-trees are freed before the next namespace is loaded.
///
/// Under `no-gc` this is a no-op.
#[cfg(feature = "no-gc")]
pub fn force_collect(_env: &Env) {}

#[cfg(not(feature = "no-gc"))]
pub fn force_collect(env: &Env) {
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        // Another thread is already collecting — just wait for it.
        cljrs_gc::safepoint();
        return;
    };

    cljrs_gc::HEAP.collect(|visitor| {
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        trace_env_roots(env, visitor);
        trace_thread_env_roots(visitor);
        trace_value_roots(visitor);
        trace_option_value_roots(visitor);
        dynamics::trace_current(visitor);
        crate::taps::trace_roots(visitor);
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
}

/// Interpreter-level GC safepoint.
///
/// Under `no-gc` this is a no-op. Under GC mode it either parks (if collection
/// is in progress) or initiates a collection (if memory pressure was signalled).
#[cfg(feature = "no-gc")]
pub fn gc_safepoint(_env: &Env) {}

#[cfg(not(feature = "no-gc"))]
pub fn gc_safepoint(env: &Env) {
    // Fast path: no GC activity at all.
    if !cljrs_gc::gc_requested() && !cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        return;
    }

    // If a GC is already in progress (another thread is collecting), just park.
    if cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        cljrs_gc::safepoint();
        return;
    }

    // A GC was requested (memory pressure). Try to become the collector.
    if !cljrs_gc::take_gc_request() {
        // Another thread took the request; if collection started, park.
        cljrs_gc::safepoint();
        return;
    }

    // We won the request. Initiate STW collection.
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        // Race: another thread started collecting between our take and begin.
        cljrs_gc::safepoint();
        return;
    };

    // All other threads are now parked. Collect with registered roots
    // plus ALL of this thread's active environments and dynamic bindings.
    cljrs_gc::HEAP.collect(|visitor| {
        // Trace globally registered roots (GlobalEnv, etc.)
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        // Trace the current (innermost) env
        trace_env_roots(env, visitor);
        // Trace all caller Envs registered on this thread's stack
        trace_thread_env_roots(visitor);
        // Trace values on the Rust call stack (shadow stack)
        trace_value_roots(visitor);
        // Trace Option<Value> slices (e.g. IR interpreter register files)
        trace_option_value_roots(visitor);
        // Trace dynamic variable bindings on this thread
        dynamics::trace_current(visitor);
        // Trace the global tap system (functions and queued values)
        crate::taps::trace_roots(visitor);
        // Trace in-flight allocations from this thread's alloc root frames
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
    // _stw_guard drop clears in_progress, waking parked threads.
}

// ── GC-only root tracing helpers ─────────────────────────────────────────────

/// Trace all GcPtr values reachable from an Env's local frames.
#[cfg(not(feature = "no-gc"))]
fn trace_env_roots(env: &Env, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    // Trace local frame bindings
    for frame in &env.frames {
        for (_name, val) in &frame.bindings {
            val.trace(visitor);
        }
    }
    // Trace the globals (namespaces, vars) — these are also registered
    // as root tracers, but it's safe to trace twice (idempotent marking).
    trace_globals(&env.globals, visitor);
}

/// Trace all Values registered in the thread-local value shadow stack.
#[cfg(not(feature = "no-gc"))]
fn trace_value_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    VALUE_ROOTS.with(|roots| {
        for &(ptr, count) in roots.borrow().iter() {
            // SAFETY: pointers are valid — they point to Values on this thread's
            // still-live stack frames or heap-allocated Vecs whose owners are
            // on still-live stack frames.
            let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
            for val in slice {
                val.trace(visitor);
            }
        }
    });
}

/// Trace all Option<Value> slices registered in the thread-local shadow stack.
///
/// Used for the IR interpreter's register file (a `Box<[Option<Value>]>`).
#[cfg(not(feature = "no-gc"))]
fn trace_option_value_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    OPTION_VALUE_ROOTS.with(|roots| {
        for &(ptr, count) in roots.borrow().iter() {
            // SAFETY: the slice is a Box<[Option<Value>]> owned by an active
            // stack frame; the address is stable for the guard's lifetime.
            let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
            for val in slice.iter().flatten() {
                val.trace(visitor);
            }
        }
    });
}

/// Trace all Envs registered in the thread-local root stack.
#[cfg(not(feature = "no-gc"))]
fn trace_thread_env_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    ENV_ROOTS.with(|roots| {
        for env_ptr in roots.borrow().iter() {
            // SAFETY: pointers are valid — they point to Envs on this thread's
            // still-live stack frames (we are the collector, so our stack is active).
            let env = unsafe { &**env_ptr };
            for frame in &env.frames {
                for (_name, val) in &frame.bindings {
                    val.trace(visitor);
                }
            }
        }
    });
}

/// Trace all namespaces and their contents.
#[cfg(not(feature = "no-gc"))]
fn trace_globals(globals: &GlobalEnv, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::{GcVisitor as _, Trace};
    let namespaces = globals.namespaces.read().unwrap();
    for (_name, ns_ptr) in namespaces.iter() {
        visitor.visit(ns_ptr);
    }
    drop(namespaces);
    // Values resolved at a pinned commit may live only in the version cache
    // (e.g. native HEAD fallbacks) — without this they would be collected.
    let version_cache = globals.version_cache.lock().unwrap();
    for (_key, val) in version_cache.iter() {
        val.trace(visitor);
    }
}

/// Service a pending GC request from an async (LocalSet) context.
///
/// Safe to call from within a Tokio `LocalSet` task at any cooperative yield
/// point: when this executes, no other tasks are polling, so thread-local root
/// stacks (ENV_ROOTS, VALUE_ROOTS, ALLOC_ROOTS) fully describe all GcPtrs held
/// by suspended tasks and can be scanned safely.
///
/// Under `no-gc` this is a no-op.
#[cfg(feature = "no-gc")]
pub fn async_gc_collect() {}

#[cfg(not(feature = "no-gc"))]
pub fn async_gc_collect() {
    if !cljrs_gc::gc_requested() && !cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        return;
    }
    if cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        cljrs_gc::safepoint();
        return;
    }
    if !cljrs_gc::take_gc_request() {
        cljrs_gc::safepoint();
        return;
    }
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        cljrs_gc::safepoint();
        return;
    };
    cljrs_gc::HEAP.collect(|visitor| {
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        trace_thread_env_roots(visitor);
        trace_value_roots(visitor);
        trace_option_value_roots(visitor);
        dynamics::trace_current(visitor);
        crate::taps::trace_roots(visitor);
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // Reclaim superseded JIT code while the world is still stopped.
    run_stw_reclaim();
}
