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
    /// Observed argument-type bitmasks, one byte per IR parameter (Phase
    /// 10.6).  OR-accumulated by [`record_call`] until the compile is
    /// queued; the JIT worker reads it to decide per-parameter
    /// specializations ([`arg_type_profile`]).
    pub arg_profile: Mutex<Vec<u8>>,
    /// Entry-guard failures of the published specialized code.  Crossing
    /// [`deopt_limit`] discards the specialization (see [`record_deopt`]).
    pub deopt_count: AtomicU32,
}

impl JitEntry {
    fn new() -> Self {
        Self {
            invocation_count: AtomicU32::new(0),
            compile_queued: AtomicBool::new(false),
            native_fn_ptr: AtomicPtr::new(std::ptr::null_mut()),
            epoch: AtomicU64::new(0),
            arg_profile: Mutex::new(Vec::new()),
            deopt_count: AtomicU32::new(0),
        }
    }
}

// ── Argument type profiles (Phase 10.6) ──────────────────────────────────────

/// Profile bitmask bits: the observed type classes of one argument position.
pub const PROFILE_LONG: u8 = 1;
pub const PROFILE_DOUBLE: u8 = 2;
pub const PROFILE_OTHER: u8 = 0x80;

/// Classify a value for the argument-type profile.
#[inline]
fn profile_tag(v: &Value) -> u8 {
    match v {
        Value::Long(_) => PROFILE_LONG,
        Value::Double(_) => PROFILE_DOUBLE,
        _ => PROFILE_OTHER,
    }
}

/// Snapshot the accumulated argument-type profile for `arity_id` (one
/// bitmask byte per IR parameter), if any calls were profiled.
pub fn arg_type_profile(arity_id: u64) -> Option<Vec<u8>> {
    let guard = JIT_TABLE.read().unwrap();
    let entry = guard.as_ref()?.get(&arity_id)?.clone();
    drop(guard);
    let prof = entry.arg_profile.lock().unwrap();
    if prof.is_empty() {
        None
    } else {
        Some(prof.clone())
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

// ── Stale-epoch hook ──────────────────────────────────────────────────────────
//
// Routing staled module epochs to the JIT's code cache without depending on
// `cljrs-jit`: the JIT installs `code_cache::mark_stale` here at init.

type StaleEpochFn = fn(u64);
static STALE_EPOCH_HOOK: OnceLock<StaleEpochFn> = OnceLock::new();

/// Install the stale-epoch sink (installed once by `cljrs_jit::init`).
pub fn set_stale_epoch_hook(f: StaleEpochFn) {
    let _ = STALE_EPOCH_HOOK.set(f);
}

/// Null any published native code for `arity_id` (whole-function and OSR
/// entries) and hand the backing epochs to the code cache for reclamation.
///
/// Used by cross-defn invalidation; a no-op when nothing was compiled or no
/// JIT is linked.
pub fn stale_native_code(arity_id: u64) {
    let mut epochs = Vec::new();
    if let Some(epoch) = take_native_epoch(arity_id) {
        epochs.push(epoch);
    }
    epochs.extend(take_osr_epochs(arity_id));
    if let Some(hook) = STALE_EPOCH_HOOK.get() {
        for epoch in epochs {
            hook(epoch);
        }
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
/// Bumps the invocation counter and folds the call's argument types into the
/// arity's type profile (`profile_args` are the positional call arguments
/// matching the IR parameters; for a variadic arity the caller passes only
/// the fixed prefix, leaving the rest-list parameter unprofiled — it is
/// padded with [`PROFILE_OTHER`] so it can never be specialized).  When the
/// counter crosses [`jit_threshold`] for the first time, submits a
/// compilation request via the enqueue hook.
///
/// Called on every Tier-1 IR dispatch; must be cheap.  Profiling stops once
/// the compile is queued, so the steady-state cost is one atomic increment,
/// one compare, and one relaxed load.
pub fn record_call(arity_id: u64, ir_func: Arc<IrFunction>, profile_args: &[Value]) {
    let entry = get_or_create_entry(arity_id);
    let count = entry.invocation_count.fetch_add(1, Ordering::Relaxed) + 1;

    if !entry.compile_queued.load(Ordering::Relaxed) {
        let n_params = ir_func.params.len();
        let mut prof = entry.arg_profile.lock().unwrap();
        if prof.len() < n_params {
            prof.resize(n_params, 0);
        }
        for (i, slot) in prof.iter_mut().enumerate().take(n_params) {
            *slot |= profile_args
                .get(i)
                .map(profile_tag)
                .unwrap_or(PROFILE_OTHER);
        }
    }

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

// ── Pending-exception hook ────────────────────────────────────────────────────
//
// Compiled code signals `(throw …)` by stashing the thrown value in a
// thread-local owned by `cljrs-compiler`'s rt_abi (this crate cannot depend on
// it).  `cljrs_jit::init` installs a taker so the dispatch seam can convert an
// uncaught native throw back into `Err(EvalError::Thrown)` instead of silently
// returning the nil sentinel (and leaking the stale pending slot into the next
// `rt_try` on this thread).

type PendingExceptionFn = fn() -> Option<Value>;
static PENDING_EXCEPTION_HOOK: OnceLock<PendingExceptionFn> = OnceLock::new();

/// Install the pending-exception taker (installed once by `cljrs_jit::init`).
pub fn set_pending_exception_hook(f: PendingExceptionFn) {
    let _ = PENDING_EXCEPTION_HOOK.set(f);
}

/// Take (and clear) the thread's pending exception, if any.
///
/// Called by the JIT-native and OSR dispatch seams immediately after native
/// code returns.  Returns `None` when no hook is installed (no JIT linked).
pub fn take_pending_exception() -> Option<Value> {
    PENDING_EXCEPTION_HOOK.get().and_then(|f| f())
}

// ── Deoptimization (Phase 10.6) ──────────────────────────────────────────────
//
// A specialized compilation guards its parameter types at entry; on a guard
// failure the native code returns a unique sentinel pointer (owned by
// rt_abi, which cljrs-eval cannot depend on — hence the hook).  The dispatch
// seam detects the sentinel, re-executes the call at Tier 1 (sound: guards
// precede all side effects), and counts the failure.  Crossing the deopt
// limit discards the specialized code and bans the arity from further
// specialization, so the next compile is generic.

type DeoptSentinelFn = fn() -> usize;
static DEOPT_SENTINEL_HOOK: OnceLock<DeoptSentinelFn> = OnceLock::new();

/// Install the deopt-sentinel address provider (installed once by
/// `cljrs_jit::init`).
pub fn set_deopt_sentinel_hook(f: DeoptSentinelFn) {
    let _ = DEOPT_SENTINEL_HOOK.set(f);
}

/// Whether `result` is the deopt sentinel returned by a failed entry guard.
#[inline]
pub fn is_deopt_result(result: *const Value) -> bool {
    DEOPT_SENTINEL_HOOK
        .get()
        .is_some_and(|f| f() == result as usize)
}

/// Arities whose specialization repeatedly deoptimized; the JIT worker
/// compiles these generically (all parameters boxed).
static SPEC_BANNED: RwLock<Option<HashSet<u64>>> = RwLock::new(None);

/// Whether `arity_id` may be compiled with type specializations.
pub fn specialization_allowed(arity_id: u64) -> bool {
    let guard = SPEC_BANNED.read().unwrap();
    !guard.as_ref().is_some_and(|s| s.contains(&arity_id))
}

/// Entry-guard failures tolerated before a specialization is discarded.
pub fn deopt_limit() -> u32 {
    std::env::var("CLJRS_JIT_DEOPT_LIMIT")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(10)
}

/// Record an entry-guard deopt for `arity_id`.
///
/// Once the failure count crosses [`deopt_limit`], the specialized code is
/// unpublished (dispatch falls back to Tier 1 immediately), its module is
/// handed to the code cache for reclamation, the arity is banned from
/// re-specialization, and its invocation counter restarts so the generic
/// recompile triggers through the ordinary hot path.
pub fn record_deopt(arity_id: u64) {
    let entry = get_or_create_entry(arity_id);
    let failures = entry.deopt_count.fetch_add(1, Ordering::Relaxed) + 1;
    cljrs_logging::feat_debug!(
        "jit",
        "deopt arity_id={} (failure #{} of {})",
        arity_id,
        failures,
        deopt_limit()
    );
    if failures < deopt_limit() {
        return;
    }
    {
        let mut guard = SPEC_BANNED.write().unwrap();
        guard.get_or_insert_with(HashSet::new).insert(arity_id);
    }
    // Unpublish + reclaim the specialized code.  `take_native_epoch` also
    // drops the JitEntry, so the arity re-counts from zero and re-enqueues a
    // (now generic) compile when hot again.
    if let Some(epoch) = take_native_epoch(arity_id)
        && let Some(hook) = STALE_EPOCH_HOOK.get()
    {
        cljrs_logging::feat_debug!(
            "jit",
            "specialization discarded arity_id={} epoch={}",
            arity_id,
            epoch
        );
        hook(epoch);
    }
}

// ── OSR (on-stack replacement) state — Phase 10.4 ────────────────────────────
//
// A single hot call containing a `loop*`/`recur` never returns to re-dispatch,
// so the invocation counter above can never promote it.  The IR interpreter
// instead counts loop back-edges per execution; when a header crosses
// [`osr_threshold`] it requests compilation of an OSR-entry variant (built by
// `cljrs_ir::osr::build_osr_function` on the JIT worker).  Once the worker
// publishes the compiled entry here, the interpreter transfers its register
// file into the native frame at the next loop-header entry.

/// A published, compiled OSR entry for one `(arity_id, loop header)` pair.
#[derive(Clone)]
pub struct OsrSlot {
    /// Native OSR-entry code: C ABI, `live_ins.len()` `*const Value` params,
    /// one `*const Value` return.
    pub fn_ptr: *const (),
    /// Reclamation epoch of the backing module (see [`push_jit_frame`]).
    pub epoch: u64,
    /// Interpreter registers to pass, in parameter order
    /// (`cljrs_ir::osr::OsrFunction::live_ins`).
    pub live_ins: Arc<[cljrs_ir::VarId]>,
}

// SAFETY: `fn_ptr` is executable code owned by the JIT code cache; it carries
// no thread affinity.  All other fields are plain data.
unsafe impl Send for OsrSlot {}
unsafe impl Sync for OsrSlot {}

enum OsrState {
    /// Compilation requested, worker has not finished yet.
    Queued,
    /// Native entry published.
    Ready(OsrSlot),
    /// Compilation declined or failed — stop polling, stay at Tier 1.
    Failed,
}

/// Result of polling for a compiled OSR entry.
pub enum OsrPoll {
    NotRequested,
    Pending,
    Ready(OsrSlot),
    Failed,
}

static OSR_TABLE: RwLock<Option<HashMap<(u64, u32), OsrState>>> = RwLock::new(None);

type OsrEnqueueFn = Box<dyn Fn(u64, u32, Arc<IrFunction>) + Send + Sync + 'static>;
static OSR_ENQUEUE_HOOK: OnceLock<OsrEnqueueFn> = OnceLock::new();

/// Install the OSR compilation enqueue hook (installed once by
/// `cljrs_jit::init`).  Receives `(arity_id, header_block, function)`; must be
/// non-blocking.
pub fn set_osr_enqueue_hook(f: impl Fn(u64, u32, Arc<IrFunction>) + Send + Sync + 'static) {
    let _ = OSR_ENQUEUE_HOOK.set(Box::new(f));
}

/// Per-process override; 0 means "env var or default".
static OSR_THRESHOLD_OVERRIDE: AtomicU32 = AtomicU32::new(0);

pub fn set_osr_threshold(t: u32) {
    OSR_THRESHOLD_OVERRIDE.store(t, Ordering::Relaxed);
}

/// Back-edge count at which a loop header is considered hot.  Defaults to the
/// invocation threshold ([`jit_threshold`]); override with
/// `CLJRS_OSR_THRESHOLD` or [`set_osr_threshold`].
pub fn osr_threshold() -> u32 {
    let v = OSR_THRESHOLD_OVERRIDE.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    std::env::var("CLJRS_OSR_THRESHOLD")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or_else(jit_threshold)
}

/// Poll for a compiled OSR entry for the loop at `header` in `arity_id`.
pub fn osr_poll(arity_id: u64, header: u32) -> OsrPoll {
    let guard = OSR_TABLE.read().unwrap();
    match guard.as_ref().and_then(|m| m.get(&(arity_id, header))) {
        None => OsrPoll::NotRequested,
        Some(OsrState::Queued) => OsrPoll::Pending,
        Some(OsrState::Ready(slot)) => OsrPoll::Ready(slot.clone()),
        Some(OsrState::Failed) => OsrPoll::Failed,
    }
}

/// Request OSR compilation for the loop at `header` in `arity_id`.  Idempotent:
/// only the first request per `(arity_id, header)` enqueues (and pays for one
/// `IrFunction` clone); with no JIT installed the entry is marked failed so
/// callers stop polling.
pub fn osr_request(arity_id: u64, header: u32, ir_func: &IrFunction) {
    let hook = OSR_ENQUEUE_HOOK.get();
    {
        let mut guard = OSR_TABLE.write().unwrap();
        let map = guard.get_or_insert_with(HashMap::new);
        match map.entry((arity_id, header)) {
            std::collections::hash_map::Entry::Occupied(_) => return,
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(if hook.is_some() {
                    OsrState::Queued
                } else {
                    OsrState::Failed
                });
            }
        }
    }
    if let Some(hook) = hook {
        cljrs_logging::feat_debug!(
            "jit",
            "osr enqueue arity_id={} header=bb{}",
            arity_id,
            header
        );
        hook(arity_id, header, Arc::new(ir_func.clone()));
    }
}

/// Publish a compiled OSR entry.  Called by the JIT worker thread.
pub fn store_osr_fn(
    arity_id: u64,
    header: u32,
    ptr: *const (),
    epoch: u64,
    live_ins: Vec<cljrs_ir::VarId>,
) {
    let mut guard = OSR_TABLE.write().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(
        (arity_id, header),
        OsrState::Ready(OsrSlot {
            fn_ptr: ptr,
            epoch,
            live_ins: live_ins.into(),
        }),
    );
}

/// Record that OSR compilation for `(arity_id, header)` declined or failed, so
/// interpreters stop polling and the loop stays at Tier 1.
pub fn mark_osr_failed(arity_id: u64, header: u32) {
    let mut guard = OSR_TABLE.write().unwrap();
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert((arity_id, header), OsrState::Failed);
}

/// Drop all OSR entries for `arity_id` (the owning var was rebound), returning
/// the epochs of published code so the caller can hand them to the code cache
/// for reclamation once no frame executes them.
pub fn take_osr_epochs(arity_id: u64) -> Vec<u64> {
    let mut guard = OSR_TABLE.write().unwrap();
    let Some(map) = guard.as_mut() else {
        return Vec::new();
    };
    let keys: Vec<(u64, u32)> = map
        .keys()
        .filter(|(a, _)| *a == arity_id)
        .copied()
        .collect();
    let mut epochs = Vec::new();
    for key in keys {
        if let Some(OsrState::Ready(slot)) = map.remove(&key) {
            epochs.push(slot.epoch);
        }
    }
    epochs
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

/// The epoch of the innermost native frame on this thread, if any.
///
/// Used by the closure-escape hook: a closure value materialized by
/// `rt_make_fn*` captures a raw pointer into the module of the currently
/// executing native code, so that module (this epoch) must be pinned against
/// reclamation.
pub fn current_jit_epoch() -> Option<u64> {
    MY_FRAMES.with(|f| f.stack.lock().unwrap().last().copied())
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
    fn osr_slot_round_trips_and_rebind_takes_epochs() {
        use cljrs_ir::VarId;
        let id = 0xF200_0001;
        // Unpublished header polls as NotRequested.
        assert!(matches!(osr_poll(id, 1), OsrPoll::NotRequested));

        let ptr = 0x5678usize as *const ();
        store_osr_fn(id, 1, ptr, 901, vec![VarId(3), VarId(4)]);
        match osr_poll(id, 1) {
            OsrPoll::Ready(slot) => {
                assert_eq!(slot.fn_ptr, ptr);
                assert_eq!(slot.epoch, 901);
                assert_eq!(&*slot.live_ins, &[VarId(3), VarId(4)]);
            }
            _ => panic!("expected Ready"),
        }

        // A second loop in the same arity that failed to compile.
        mark_osr_failed(id, 7);
        assert!(matches!(osr_poll(id, 7), OsrPoll::Failed));

        // Rebinding the var takes only the published epochs and clears all
        // entries for the arity.
        let epochs = take_osr_epochs(id);
        assert_eq!(epochs, vec![901]);
        assert!(matches!(osr_poll(id, 1), OsrPoll::NotRequested));
        assert!(matches!(osr_poll(id, 7), OsrPoll::NotRequested));
    }

    #[test]
    fn osr_request_without_hook_marks_failed() {
        // The test binary never installs the OSR enqueue hook (that's
        // cljrs_jit::init's job), so a request must immediately fail closed
        // rather than leave interpreters polling forever.
        let id = 0xF200_0002;
        let ir = IrFunction::new(None, None);
        osr_request(id, 2, &ir);
        assert!(matches!(osr_poll(id, 2), OsrPoll::Failed));
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
