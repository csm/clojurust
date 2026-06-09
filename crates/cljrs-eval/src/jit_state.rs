//! JIT invocation counters and native function pointer cache.
//!
//! Bridges the Tier-1 IR interpreter and the background JIT compiler
//! (`cljrs-jit`). Each JIT-eligible arity has one [`JitEntry`] keyed by
//! `ir_arity_id`. The flow is:
//!
//! 1. `record_call` bumps the invocation counter; crossing the threshold
//!    triggers the enqueue hook (installed by `cljrs_jit::init`).
//! 2. The background worker compiles the function and calls `store_native_fn`.
//! 3. `get_native_fn` returns the pointer; `call_cljrs_fn` calls it.
//! 4. `dispatch_jit_call` transmutes the raw pointer to the correct arity
//!    and invokes the native code.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock, Weak};

use cljrs_ir::IrFunction;
use cljrs_value::Value;

// ── Threshold ─────────────────────────────────────────────────────────────────

pub const DEFAULT_JIT_THRESHOLD: u32 = 1_000;

/// Per-process override set by the CLI or programmatic callers.
/// 0 means "read from CLJRS_JIT_THRESHOLD env var or use the default".
static JIT_THRESHOLD_OVERRIDE: AtomicU32 = AtomicU32::new(0);

pub fn set_jit_threshold(t: u32) {
    JIT_THRESHOLD_OVERRIDE.store(t, Ordering::Relaxed);
}

pub fn jit_threshold() -> u32 {
    let v = JIT_THRESHOLD_OVERRIDE.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    std::env::var("CLJRS_JIT_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(DEFAULT_JIT_THRESHOLD)
}

// ── Per-arity state ───────────────────────────────────────────────────────────

pub struct JitEntry {
    pub invocation_count: AtomicU32,
    pub compile_queued: AtomicBool,
    /// Finalized native function pointer, or null if not yet compiled.
    /// Calling convention: SystemV/C ABI, N ptr-sized params, one ptr-sized return.
    pub native_fn_ptr: AtomicPtr<()>,
    /// Reclamation epoch of the published native code (0 = none).  Assigned by
    /// the JIT worker when it registers the compiled module; used by code
    /// unloading to identify which `JITModule` backs this pointer and to track
    /// whether a frame executing this code is live at a safepoint.
    pub epoch: AtomicU64,
}

impl JitEntry {
    fn new() -> Self {
        Self {
            invocation_count: AtomicU32::new(0),
            compile_queued: AtomicBool::new(false),
            native_fn_ptr: AtomicPtr::new(std::ptr::null_mut()),
            epoch: AtomicU64::new(0),
        }
    }
}

// SAFETY: all fields are atomics, inherently Send+Sync.
unsafe impl Send for JitEntry {}
unsafe impl Sync for JitEntry {}

static JIT_TABLE: RwLock<Option<HashMap<u64, Arc<JitEntry>>>> = RwLock::new(None);

fn get_or_create_entry(arity_id: u64) -> Arc<JitEntry> {
    {
        let guard = JIT_TABLE.read().unwrap();
        if let Some(cache) = guard.as_ref()
            && let Some(e) = cache.get(&arity_id)
        {
            return e.clone();
        }
    }
    let mut guard = JIT_TABLE.write().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    cache
        .entry(arity_id)
        .or_insert_with(|| Arc::new(JitEntry::new()))
        .clone()
}

/// Return the compiled native function pointer and its reclamation epoch for
/// `arity_id`, if native code is currently published.
///
/// The caller **must** keep the returned `epoch` live (via [`push_jit_frame`])
/// for the entire native call, so code unloading at a stop-the-world safepoint
/// does not free the backing module while it executes.
pub fn get_native_fn(arity_id: u64) -> Option<(*const (), u64)> {
    let guard = JIT_TABLE.read().unwrap();
    let cache = guard.as_ref()?;
    let entry = cache.get(&arity_id)?;
    let ptr = entry.native_fn_ptr.load(Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        let epoch = entry.epoch.load(Ordering::Acquire);
        Some((ptr as *const (), epoch))
    }
}

/// Publish a compiled native function pointer and its reclamation epoch for
/// `arity_id`.  Called by the JIT worker thread after successful compilation.
///
/// Stores the epoch before the pointer (release ordering) so that any reader
/// that observes a non-null pointer also observes the matching epoch.
pub fn store_native_fn(arity_id: u64, ptr: *const (), epoch: u64) {
    let entry = get_or_create_entry(arity_id);
    entry.epoch.store(epoch, Ordering::Release);
    entry.native_fn_ptr.store(ptr as *mut (), Ordering::Release);
}

/// Clear the published native pointer for `arity_id` and return the epoch that
/// was backing it, if any.
///
/// Called when a var holding this function is redefined: future dispatches fall
/// back to the interpreter immediately (the pointer is nulled), and the
/// returned epoch is handed to the code cache so the now-superseded module is
/// reclaimed at the next safepoint once no frame is executing it.  Also drops
/// the per-arity table entry, keeping `JIT_TABLE` bounded across a long REPL
/// session of redefinitions.
pub fn take_native_epoch(arity_id: u64) -> Option<u64> {
    let mut guard = JIT_TABLE.write().unwrap();
    let cache = guard.as_mut()?;
    let entry = cache.remove(&arity_id)?;
    let ptr = entry
        .native_fn_ptr
        .swap(std::ptr::null_mut(), Ordering::AcqRel);
    if ptr.is_null() {
        None
    } else {
        Some(entry.epoch.load(Ordering::Acquire))
    }
}

// ── Enqueue hook ──────────────────────────────────────────────────────────────

type EnqueueFn = Box<dyn Fn(u64, Arc<IrFunction>) + Send + Sync + 'static>;
static ENQUEUE_HOOK: OnceLock<EnqueueFn> = OnceLock::new();

/// Install the JIT compilation enqueue hook.
///
/// The hook is called (from the Tier-1 dispatch hot path) when a function
/// crosses the invocation threshold.  It must be non-blocking: enqueue onto a
/// channel and return immediately.  Called at most once per arity.
///
/// Installed once by `cljrs_jit::init`.
pub fn set_enqueue_hook(f: impl Fn(u64, Arc<IrFunction>) + Send + Sync + 'static) {
    let _ = ENQUEUE_HOOK.set(Box::new(f));
}

/// Record a call to `arity_id`.
///
/// Bumps the invocation counter.  When the counter crosses [`jit_threshold`]
/// for the first time, submits a compilation request via the enqueue hook.
///
/// Called on every Tier-1 IR dispatch; must be cheap (one atomic increment +
/// one compare on the fast path).
pub fn record_call(arity_id: u64, ir_func: Arc<IrFunction>) {
    let entry = get_or_create_entry(arity_id);
    let count = entry.invocation_count.fetch_add(1, Ordering::Relaxed) + 1;

    if count < jit_threshold() {
        return;
    }
    // Threshold crossed — enqueue exactly once.
    if entry.compile_queued.swap(true, Ordering::AcqRel) {
        return;
    }
    if let Some(hook) = ENQUEUE_HOOK.get() {
        cljrs_logging::feat_debug!("jit", "enqueue arity_id={} (count={})", arity_id, count);
        hook(arity_id, ir_func);
    }
}

// ── Active JIT frame tracking (for code unloading) ──────────────────────────────
//
// Code unloading (Phase 10.2) frees a superseded `JITModule` only once no frame
// is executing its code.  We track that precisely: each in-flight native call
// pushes its code epoch onto a per-thread stack for the duration of the call.
//
// At a stop-the-world safepoint every mutator thread is parked, so the reclaimer
// can read all threads' stacks to compute the set of epochs that must not be
// freed.  Cross-thread reads are sound because:
//   - Each thread mutates only its own stack, and only while running (never
//     while parked: push/pop bracket the native call and release before any
//     safepoint poll inside it).
//   - At STW all other threads are frozen, so their stacks are stable.
//
// The per-thread stack lives behind a `Mutex` that is essentially uncontended
// on the hot path (only the owning thread locks it, briefly); the reclaimer
// contends for it only at the rare STW safepoint.

struct ThreadFrames {
    stack: Mutex<Vec<u64>>,
}

static FRAME_REGISTRY: RwLock<Vec<Weak<ThreadFrames>>> = RwLock::new(Vec::new());

thread_local! {
    static MY_FRAMES: Arc<ThreadFrames> = {
        let frames = Arc::new(ThreadFrames { stack: Mutex::new(Vec::new()) });
        let mut reg = FRAME_REGISTRY.write().unwrap();
        // Opportunistically drop registrations for threads that have exited.
        reg.retain(|w| w.strong_count() > 0);
        reg.push(Arc::downgrade(&frames));
        frames
    };
}

/// RAII guard that pops one active-frame epoch on drop (including on unwind,
/// e.g. when native code throws back through the dispatch boundary).
pub struct JitFrameGuard {
    epoch: u64,
}

impl Drop for JitFrameGuard {
    fn drop(&mut self) {
        MY_FRAMES.with(|f| {
            let mut stack = f.stack.lock().unwrap();
            // Pop the matching epoch.  Frames are strictly LIFO, so the epoch
            // is almost always the top; search defensively in case of nesting.
            if let Some(pos) = stack.iter().rposition(|&e| e == self.epoch) {
                stack.remove(pos);
            }
        });
    }
}

/// Register that this thread is about to enter native code backed by `epoch`.
///
/// Returns a guard that unregisters the frame on drop.  Must wrap the native
/// call so code unloading never frees a module that is executing.
pub fn push_jit_frame(epoch: u64) -> JitFrameGuard {
    MY_FRAMES.with(|f| f.stack.lock().unwrap().push(epoch));
    JitFrameGuard { epoch }
}

/// Collect the set of epochs with at least one active native frame across all
/// mutator threads.
///
/// **Must be called at a stop-the-world safepoint** (all other mutator threads
/// parked), so each thread's frame stack is stable while it is read.  Used by
/// the JIT code cache to decide which stale modules are safe to free.
pub fn live_epochs() -> HashSet<u64> {
    let mut live = HashSet::new();
    let reg = FRAME_REGISTRY.read().unwrap();
    for weak in reg.iter() {
        if let Some(frames) = weak.upgrade() {
            for &epoch in frames.stack.lock().unwrap().iter() {
                live.insert(epoch);
            }
        }
    }
    live
}

// ── JIT function dispatch ─────────────────────────────────────────────────────

/// Transmute `fn_ptr` to the correct arity and invoke native JIT code.
///
/// # Safety
/// - `fn_ptr` must be a valid JIT-compiled function using the default
///   platform C calling convention (SystemV on x86-64 Linux/macOS).
/// - The function must accept exactly `args.len()` parameters of type
///   `*const Value` and return `*const Value`.
/// - The returned pointer is valid until the GC collects the backing object;
///   callers must clone the `Value` before yielding a safepoint.
#[allow(clippy::too_many_arguments)]
pub unsafe fn dispatch_jit_call(fn_ptr: *const (), args: &[*const Value]) -> *const Value {
    unsafe {
        match args.len() {
            0 => {
                let f: unsafe extern "C" fn() -> *const Value = std::mem::transmute(fn_ptr);
                f()
            }
            1 => {
                let f: unsafe extern "C" fn(*const Value) -> *const Value =
                    std::mem::transmute(fn_ptr);
                f(args[0])
            }
            2 => {
                let f: unsafe extern "C" fn(*const Value, *const Value) -> *const Value =
                    std::mem::transmute(fn_ptr);
                f(args[0], args[1])
            }
            3 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(args[0], args[1], args[2])
            }
            4 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(args[0], args[1], args[2], args[3])
            }
            5 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(args[0], args[1], args[2], args[3], args[4])
            }
            6 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(args[0], args[1], args[2], args[3], args[4], args[5])
            }
            7 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6],
                )
            }
            8 => {
                let f: unsafe extern "C" fn(
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                    *const Value,
                ) -> *const Value = std::mem::transmute(fn_ptr);
                f(
                    args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
                )
            }
            n => panic!("JIT dispatch: unsupported arity {n} (max 8 in Phase 10.1)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_round_trips_through_store_get_take() {
        // Use sentinel ids/ptrs unlikely to collide with concurrent tests.
        let id = 0xF100_0001;
        let ptr = 0x1234usize as *const ();
        store_native_fn(id, ptr, 777);
        assert_eq!(get_native_fn(id), Some((ptr, 777)));
        // take returns the epoch and removes the entry.
        assert_eq!(take_native_epoch(id), Some(777));
        assert_eq!(get_native_fn(id), None);
        assert_eq!(take_native_epoch(id), None);
    }

    #[test]
    fn frame_guard_marks_epoch_live_then_clears() {
        let e = 0xBEEF_0001;
        assert!(!live_epochs().contains(&e));
        let guard = push_jit_frame(e);
        assert!(live_epochs().contains(&e));
        drop(guard);
        assert!(!live_epochs().contains(&e));
    }

    #[test]
    fn nested_frames_pop_in_lifo_order() {
        let a = 0xBEEF_1001;
        let b = 0xBEEF_1002;
        let ga = push_jit_frame(a);
        let gb = push_jit_frame(b);
        let live = live_epochs();
        assert!(live.contains(&a) && live.contains(&b));
        drop(gb);
        assert!(live_epochs().contains(&a));
        assert!(!live_epochs().contains(&b));
        drop(ga);
        assert!(!live_epochs().contains(&a));
    }
}
