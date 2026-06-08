//! JIT tiering state: invocation counters, code-pointer slots, and the hooks
//! that connect this crate's hot dispatch path to the `cljrs-jit` backend.
//!
//! This module is the shared structure the Phase 10.1 JIT is built around.  It
//! deliberately lives **below** `cljrs-jit` in the crate graph (`cljrs-jit`
//! depends on `cljrs-compiler` which depends on this crate), so it cannot call
//! into `cljrs-jit` directly.  Instead it exposes two function-pointer hooks
//! that `cljrs-jit::init` registers at startup:
//!
//! - [`register_compile_hook`] — enqueue an `(arity_id, IrFunction)` for
//!   background compilation.
//! - [`register_invoke_hook`] — call a finalized native code pointer with a
//!   slice of boxed `*const Value` arguments.
//!
//! The hot path (`crate::apply`) reads one `JitEntry` per eligible call: an
//! atomic counter bump and, once compiled, an atomic load of the code pointer.
//! Keyed by `ir_arity_id` so it parallels the IR cache.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use cljrs_ir::IrFunction;
use cljrs_value::Value;

// ── Tiering state for one arity ──────────────────────────────────────────────

// Lifecycle of a `JitEntry`:
//   New ──(threshold crossed)──▶ Queued ──(compile ok)──▶ Ready
//                                   └──────(compile fail)──▶ Failed
const STATE_NEW: u8 = 0;
const STATE_QUEUED: u8 = 1;
const STATE_READY: u8 = 2;
const STATE_FAILED: u8 = 3;

/// Per-arity JIT state.  One is created lazily the first time an eligible arity
/// is called while the JIT is enabled.
pub struct JitEntry {
    /// Number of times this arity has been invoked through the tiered path.
    counter: AtomicU32,
    /// Lifecycle state (`STATE_*`).
    state: AtomicU8,
    /// Finalized native code pointer once `state == READY`, else null.
    code: AtomicPtr<u8>,
    /// Parameter count (set when queued) — drives the call ABI selection.
    n_params: AtomicU8,
}

impl JitEntry {
    fn new() -> Self {
        JitEntry {
            counter: AtomicU32::new(0),
            state: AtomicU8::new(STATE_NEW),
            code: AtomicPtr::new(std::ptr::null_mut()),
            n_params: AtomicU8::new(0),
        }
    }

    /// Finalized code pointer if this arity has been compiled, else `None`.
    #[inline]
    pub fn ready_code(&self) -> Option<(*const u8, u8)> {
        if self.state.load(Ordering::Acquire) == STATE_READY {
            let p = self.code.load(Ordering::Acquire);
            if !p.is_null() {
                return Some((p as *const u8, self.n_params.load(Ordering::Relaxed)));
            }
        }
        None
    }

    /// Bump the invocation counter and return the new value.
    #[inline]
    pub fn bump(&self) -> u32 {
        self.counter.fetch_add(1, Ordering::Relaxed) + 1
    }

    /// Atomically claim the right to queue this arity for compilation.
    /// Returns `true` exactly once (the first New→Queued transition).
    #[inline]
    pub fn try_begin_queue(&self, n_params: u8) -> bool {
        let won = self
            .state
            .compare_exchange(STATE_NEW, STATE_QUEUED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok();
        if won {
            self.n_params.store(n_params, Ordering::Relaxed);
        }
        won
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static ENTRIES: RwLock<Option<HashMap<u64, Arc<JitEntry>>>> = RwLock::new(None);

/// Fetch (or lazily create) the `JitEntry` for an arity id.
pub fn get_or_create(arity_id: u64) -> Arc<JitEntry> {
    // Fast path: shared read lock.
    {
        let guard = ENTRIES.read().unwrap();
        if let Some(map) = guard.as_ref()
            && let Some(entry) = map.get(&arity_id)
        {
            return entry.clone();
        }
    }
    // Slow path: create under the write lock.
    let mut guard = ENTRIES.write().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(arity_id)
        .or_insert_with(|| Arc::new(JitEntry::new()))
        .clone()
}

fn lookup(arity_id: u64) -> Option<Arc<JitEntry>> {
    let guard = ENTRIES.read().unwrap();
    guard.as_ref()?.get(&arity_id).cloned()
}

/// Publish a finalized native code pointer for an arity.  Called by the
/// `cljrs-jit` worker thread when compilation succeeds.
///
/// # Safety
/// `code` must point to executable, finalized native code with the
/// `extern "C" fn(*const Value, ...) -> *const Value` signature for the
/// arity's parameter count, and must remain valid for the rest of the process
/// (code unloading is Phase 10.2).
pub fn publish_code(arity_id: u64, code: *const u8) {
    if let Some(entry) = lookup(arity_id) {
        entry.code.store(code as *mut u8, Ordering::Release);
        entry.state.store(STATE_READY, Ordering::Release);
        cljrs_logging::feat_debug!("jit", "published native code for arity {arity_id}");
    }
}

/// Mark an arity as failed to compile so it is never re-queued.  Called by the
/// `cljrs-jit` worker when codegen fails.
pub fn mark_failed(arity_id: u64) {
    if let Some(entry) = lookup(arity_id) {
        entry.state.store(STATE_FAILED, Ordering::Release);
        cljrs_logging::feat_debug!("jit", "arity {arity_id} marked unJITtable");
    }
}

// ── Configuration ────────────────────────────────────────────────────────────

static ENABLED: AtomicBool = AtomicBool::new(false);
static THRESHOLD: OnceLock<u32> = OnceLock::new();

/// Default invocation count that trips a function into the JIT queue.
const DEFAULT_THRESHOLD: u32 = 1000;

/// Enable or disable the JIT tier process-wide.
///
/// Off by default; the CLI turns it on for `--jit` (or `CLJRS_JIT`).  When
/// disabled the hot path skips all JIT logic entirely.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Whether the JIT tier is enabled.
#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Invocation threshold for JIT promotion (env `CLJRS_JIT_THRESHOLD`, else
/// [`DEFAULT_THRESHOLD`]).
#[inline]
pub fn threshold() -> u32 {
    *THRESHOLD.get_or_init(|| {
        std::env::var("CLJRS_JIT_THRESHOLD")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_THRESHOLD)
    })
}

// ── Hooks into the `cljrs-jit` backend ───────────────────────────────────────

/// Enqueue an arity for background compilation.
pub type CompileHook = fn(arity_id: u64, ir: Arc<IrFunction>, n_params: u8);

/// Invoke finalized native code.  `args` are boxed `*const Value` pointers
/// (kept alive by the caller for the duration of the call).  Returns `Ok` with
/// the (cloned) result, or `Err` with a (cloned) thrown `Value`.
pub type InvokeHook = fn(code: *const u8, args: &[*const Value]) -> Result<Value, Value>;

static COMPILE_HOOK: OnceLock<CompileHook> = OnceLock::new();
static INVOKE_HOOK: OnceLock<InvokeHook> = OnceLock::new();

/// Register the background-compilation hook (called once by `cljrs-jit::init`).
pub fn register_compile_hook(hook: CompileHook) {
    let _ = COMPILE_HOOK.set(hook);
}

/// Register the native-invocation hook (called once by `cljrs-jit::init`).
pub fn register_invoke_hook(hook: InvokeHook) {
    let _ = INVOKE_HOOK.set(hook);
}

/// Hand an arity to the compile hook if one is registered.
pub fn enqueue(arity_id: u64, ir: Arc<IrFunction>, n_params: u8) {
    if let Some(hook) = COMPILE_HOOK.get() {
        cljrs_logging::feat_debug!(
            "jit",
            "queueing arity {arity_id} ({n_params} params) for JIT compilation"
        );
        hook(arity_id, ir, n_params);
    } else {
        // No backend registered (e.g. JIT enabled but `cljrs-jit::init` not
        // called): never retry.
        mark_failed(arity_id);
    }
}

/// Invoke finalized native code via the registered hook.  Returns `None` if no
/// invoke hook is registered (caller must fall back to the interpreter).
#[inline]
pub fn invoke_native(code: *const u8, args: &[*const Value]) -> Option<Result<Value, Value>> {
    INVOKE_HOOK.get().map(|hook| hook(code, args))
}
