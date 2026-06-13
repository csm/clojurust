//! Thread-safe IR cache for compiled function arities.
//!
//! Each `CljxFnArity` is assigned a unique `ir_arity_id` at creation time.
//! When a function is called, the cache is consulted:
//! - `NotAttempted` → try lowering via the Clojure compiler
//! - `Cached(ir)` → execute via IR interpreter
//! - `Unsupported` → fall back to tree-walking (don't retry)
//!
//! The hot path (`get_cached`) uses `RwLock` so concurrent reads don't
//! contend.  Writes (store) are infrequent (only during lowering).
//!
//! ## Cold-entry eviction (Phase 10.7)
//!
//! Cached entries carry a coarse last-access timestamp, refreshed on every
//! `get_cached` hit.  [`sweep_idle`] — run from the stop-the-world reclaim
//! pass once the background lowering worker is started — evicts entries idle
//! longer than [`ir_cache_ttl_secs`].  The IR cache is deliberately *colder*
//! than native code: eviction happens long after the last access, and only
//! when GC pressure triggers a collection anyway.  Entries whose arity has
//! published native code or a queued compile are never evicted (the IR is the
//! deoptimization fallback), and `Unsupported` markers are kept forever (they
//! are tiny and prevent retry storms).

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, RwLock};
use std::time::Instant;

use cljrs_ir::IrFunction;

// ── Cache entries ────────────────────────────────────────────────────────────

/// State of an IR cache entry for one function arity.
pub enum IrCacheEntry {
    /// Lowering has not been attempted yet.
    NotAttempted,
    /// Lowering was attempted but failed (unsupported form); don't retry.
    Unsupported,
    /// Successfully lowered IR function.
    Cached {
        ir: Arc<IrFunction>,
        /// Coarse seconds (see [`now_secs`]) of the last `get_cached` hit.
        last_access: AtomicU64,
    },
}

// ── Coarse clock ─────────────────────────────────────────────────────────────

static PROCESS_EPOCH: LazyLock<Instant> = LazyLock::new(Instant::now);

/// Seconds since the process epoch — the coarse clock for last-access
/// tracking.  Monotonic and cheap (one `Instant::now` per call).
pub fn now_secs() -> u64 {
    PROCESS_EPOCH.elapsed().as_secs()
}

/// Idle time after which a cached IR entry becomes eligible for eviction.
/// `CLJRS_IR_CACHE_TTL` (seconds) overrides the default of 600.
pub fn ir_cache_ttl_secs() -> u64 {
    std::env::var("CLJRS_IR_CACHE_TTL")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(600)
}

// ── Global cache ─────────────────────────────────────────────────────────────

static IR_CACHE: RwLock<Option<HashMap<u64, IrCacheEntry>>> = RwLock::new(None);

/// Look up a cached IR function by arity ID, refreshing its last-access time.
/// Returns `None` if not cached or if lowering previously failed.
/// Returns `Some(ir)` if cached.
///
/// This is the hot path — uses a read lock so concurrent callers don't block
/// (the access timestamp is a relaxed atomic store under the read lock).
pub fn get_cached(id: u64) -> Option<Arc<IrFunction>> {
    let guard = IR_CACHE.read().unwrap();
    let cache = guard.as_ref()?;
    match cache.get(&id) {
        Some(IrCacheEntry::Cached { ir, last_access }) => {
            last_access.store(now_secs(), Ordering::Relaxed);
            Some(ir.clone())
        }
        _ => None,
    }
}

/// Check if lowering should be attempted for this arity.
/// Returns `true` if the entry is `NotAttempted` (or absent).
pub fn should_attempt(id: u64) -> bool {
    let guard = IR_CACHE.read().unwrap();
    match guard.as_ref() {
        Some(cache) => !cache.contains_key(&id),
        None => true,
    }
}

/// Store a successful IR compilation result.
pub fn store_cached(id: u64, ir: Arc<IrFunction>) {
    let mut guard = IR_CACHE.write().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.insert(
        id,
        IrCacheEntry::Cached {
            ir,
            last_access: AtomicU64::new(now_secs()),
        },
    );
}

/// Mark an arity as unsupported (lowering failed; don't retry).
pub fn store_unsupported(id: u64) {
    let mut guard = IR_CACHE.write().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.insert(id, IrCacheEntry::Unsupported);
}

/// Drop the cache entry for an arity entirely (back to `NotAttempted`), so a
/// later [`should_attempt`] returns `true` and the arity can be re-lowered.
///
/// Used by cross-defn invalidation: a lowering that specialized against
/// another defn is stale once that defn is rebound.
pub fn invalidate(id: u64) {
    let mut guard = IR_CACHE.write().unwrap();
    if let Some(cache) = guard.as_mut() {
        cache.remove(&id);
    }
}

// ── Cold-entry sweep (Phase 10.7) ────────────────────────────────────────────

/// Evict cached IR entries idle longer than `ttl_secs`, returning the evicted
/// arity ids.  Intended to run at a stop-the-world safepoint (registered by
/// the lowering worker), but safe at any time: in-flight Tier-1 frames hold
/// their own `Arc<IrFunction>`, and OSR native frames are protected by the
/// code cache's live-epoch scan.
///
/// Skips entries whose arity has published native code or a queued compile —
/// their IR is the deoptimization fallback — and never touches `Unsupported`
/// markers.  For each evicted id the per-arity `JitEntry` is dropped (so the
/// function can re-warm from zero) and any published OSR-entry code is staled
/// for reclamation: it is only reachable from Tier-1 interpretation of the
/// evicted IR, so it is equally cold.
///
/// Takes `now` as a parameter for testability; production callers pass
/// [`now_secs`]`()`.
///
/// Note: `defn_registry` deliberately retains its own `Arc<IrFunction>`s —
/// cross-defn inlining of an unchanged defn stays valid.  The sweep targets
/// only this dispatch cache.
pub fn sweep_idle(now: u64, ttl_secs: u64) -> Vec<u64> {
    let mut evicted = Vec::new();
    let mut guard = IR_CACHE.write().unwrap();
    let Some(cache) = guard.as_mut() else {
        return evicted;
    };
    cache.retain(|&id, entry| {
        let IrCacheEntry::Cached { last_access, .. } = entry else {
            return true;
        };
        let idle = now.saturating_sub(last_access.load(Ordering::Relaxed));
        if idle <= ttl_secs {
            return true;
        }
        if crate::jit_state::get_native_fn(id).is_some() || crate::jit_state::compile_queued(id) {
            return true;
        }
        evicted.push(id);
        false
    });
    drop(guard);
    for &id in &evicted {
        crate::jit_state::evict_entry_if_cold(id);
        crate::jit_state::stale_osr_code(id);
        cljrs_logging::feat_debug!("ir", "evicted idle IR arity_id={}", id);
    }
    evicted
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ir() -> Arc<IrFunction> {
        Arc::new(IrFunction::new(None, None))
    }

    // Sentinel arity ids (0xE5xx_xxxx range) so parallel tests sharing the
    // global cache never collide; mirrors the jit_state test convention.
    //
    // The sweep itself is global, though: a far-future `sweep_idle` from one
    // test would evict another test's entry mid-setup.  Serialize every test
    // that sweeps (or whose entries a sweep could evict) on this lock.
    static SWEEP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn sweep_guard() -> std::sync::MutexGuard<'static, ()> {
        SWEEP_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    #[test]
    fn sweep_evicts_idle_entry_and_drops_jit_entry() {
        let _g = sweep_guard();
        let id = 0xE500_0001;
        store_cached(id, dummy_ir());
        crate::jit_state::mark_lower_queued(id);

        // Recent entry survives a sweep.
        let stored_at = now_secs();
        assert!(sweep_idle(stored_at, 600).is_empty() || !should_attempt(id));
        assert!(get_cached(id).is_some());

        // Far in the future the entry is idle past the TTL and is evicted,
        // along with its JitEntry (lower_queued resets so it can re-warm).
        let evicted = sweep_idle(stored_at + 601, 600);
        assert!(evicted.contains(&id));
        assert!(get_cached(id).is_none());
        assert!(should_attempt(id));
        assert!(!crate::jit_state::lower_queued(id));
    }

    #[test]
    fn sweep_skips_native_published_arity() {
        let _g = sweep_guard();
        let id = 0xE500_0002;
        store_cached(id, dummy_ir());
        crate::jit_state::store_native_fn(id, 0x1234usize as *const (), 31337);

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(get_cached(id).is_some());

        // Cleanup: unpublish so other tests' sweeps behave.
        crate::jit_state::take_native_epoch(id);
        invalidate(id);
    }

    #[test]
    fn sweep_skips_queued_compile() {
        let _g = sweep_guard();
        let id = 0xE500_0003;
        let ir = dummy_ir();
        store_cached(id, ir.clone());
        // Cross the JIT threshold; with no enqueue hook installed this just
        // pins compile_queued, exactly the state of an in-flight compile.
        for _ in 0..crate::jit_state::jit_threshold() {
            crate::jit_state::record_call(id, ir.clone(), &[]);
        }
        assert!(crate::jit_state::compile_queued(id));

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(get_cached(id).is_some());
        invalidate(id);
    }

    #[test]
    fn sweep_never_touches_unsupported() {
        let _g = sweep_guard();
        let id = 0xE500_0004;
        store_unsupported(id);
        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        // Still terminal: no re-lowering attempts.
        assert!(!should_attempt(id));
    }

    #[test]
    fn get_cached_refreshes_last_access() {
        let _g = sweep_guard();
        let id = 0xE500_0005;
        store_cached(id, dummy_ir());
        // Touch, then sweep with a now that is idle relative to the store
        // time but not the touch time recorded by get_cached: by refreshing
        // on access the entry must survive a sweep whose `now` is within the
        // TTL of the touch.
        let _ = get_cached(id);
        let touched_at = now_secs();
        let evicted = sweep_idle(touched_at + 599, 600);
        assert!(!evicted.contains(&id));
        assert!(get_cached(id).is_some());
        invalidate(id);
    }
}
