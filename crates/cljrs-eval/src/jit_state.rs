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

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

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
}

impl JitEntry {
    fn new() -> Self {
        Self {
            invocation_count: AtomicU32::new(0),
            compile_queued: AtomicBool::new(false),
            native_fn_ptr: AtomicPtr::new(std::ptr::null_mut()),
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

/// Return the compiled native function pointer for `arity_id`, if available.
pub fn get_native_fn(arity_id: u64) -> Option<*const ()> {
    let guard = JIT_TABLE.read().unwrap();
    let cache = guard.as_ref()?;
    let entry = cache.get(&arity_id)?;
    let ptr = entry.native_fn_ptr.load(Ordering::Acquire);
    if ptr.is_null() {
        None
    } else {
        Some(ptr as *const ())
    }
}

/// Publish a compiled native function pointer for `arity_id`.
/// Called by the JIT worker thread after successful compilation.
pub fn store_native_fn(arity_id: u64, ptr: *const ()) {
    let entry = get_or_create_entry(arity_id);
    entry.native_fn_ptr.store(ptr as *mut (), Ordering::Release);
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
