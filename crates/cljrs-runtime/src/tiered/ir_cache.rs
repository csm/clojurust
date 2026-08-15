//! Per-runtime cache of lowered IR, keyed by arity ID.
//!
//! Each `CljxFnArity` is assigned a unique `ir_arity_id` at creation time.
//! When a function is called, its runtime's cache is consulted:
//! - `NotAttempted` → try lowering
//! - `Cached(ir)` → execute via the IR interpreter
//! - `Unsupported` → fall back to tree-walking (don't retry)
//!
//! The hot path ([`IrCache::get`]) uses `RwLock` so concurrent reads don't
//! contend.  Writes (store) are infrequent (only during lowering).
//!
//! ## Ownership
//!
//! An [`IrCache`] belongs to one runtime's [`Tiers`], reached through
//! [`GlobalEnv::ir_cache`]: two runtimes in one process never read or evict
//! each other's entries, and a runtime's IR is freed when the runtime is.
//! Callers that hold only an arity id — the background lowering worker, the
//! JIT worker's publish guard — carry a [`Weak<Tiers>`](std::sync::Weak)
//! naming the runtime that asked, so no process-wide index of caches is
//! needed to resolve them.
//!
//! ## Cold-entry eviction (Phase 10.7)
//!
//! Cached entries carry a coarse last-access timestamp, refreshed on every
//! [`IrCache::get`] hit.  [`Tiers::sweep`] — run from the stop-the-world
//! reclaim pass once the background lowering worker is started — evicts
//! entries idle longer than [`ir_cache_ttl_secs`].  The IR cache is
//! deliberately *colder* than native code: eviction happens long after the
//! last access, and only when GC pressure triggers a collection anyway.
//! Entries whose arity has published native code or a queued compile are
//! never evicted (the IR is the deoptimization fallback), and `Unsupported`
//! markers are kept forever (they are tiny and prevent retry storms).
//!
//! [`Tiers`]: crate::tiered::tiers::Tiers
//! [`Tiers::sweep`]: crate::tiered::tiers::Tiers::sweep
//! [`GlobalEnv::ir_cache`]: crate::env::env::GlobalEnv::ir_cache

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
        /// Coarse seconds (see [`now_secs`]) of the last [`IrCache::get`] hit.
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

// ── The cache ────────────────────────────────────────────────────────────────

/// One runtime's lowered-IR cache.
pub struct IrCache {
    entries: RwLock<HashMap<u64, IrCacheEntry>>,
}

impl IrCache {
    /// Create an empty cache.  One runtime's [`Tiers`](crate::tiered::tiers::Tiers)
    /// owns exactly one.
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(HashMap::new()),
        }
    }

    /// Look up a cached IR function by arity ID, refreshing its last-access
    /// time.  `None` if not cached or if lowering previously failed.
    ///
    /// This is the hot path — uses a read lock so concurrent callers don't
    /// block (the access timestamp is a relaxed atomic store under it).
    pub fn get(&self, id: u64) -> Option<Arc<IrFunction>> {
        let guard = self.entries.read().unwrap();
        match guard.get(&id) {
            Some(IrCacheEntry::Cached { ir, last_access }) => {
                last_access.store(now_secs(), Ordering::Relaxed);
                Some(ir.clone())
            }
            _ => None,
        }
    }

    /// Check if lowering should be attempted for this arity.
    /// `true` if the entry is `NotAttempted` (or absent).
    pub fn should_attempt(&self, id: u64) -> bool {
        !self.entries.read().unwrap().contains_key(&id)
    }

    /// Store a successful IR compilation result.
    pub fn store(&self, id: u64, ir: Arc<IrFunction>) {
        self.entries.write().unwrap().insert(
            id,
            IrCacheEntry::Cached {
                ir,
                last_access: AtomicU64::new(now_secs()),
            },
        );
    }

    /// Mark an arity as unsupported (lowering failed; don't retry).
    pub fn store_unsupported(&self, id: u64) {
        self.entries
            .write()
            .unwrap()
            .insert(id, IrCacheEntry::Unsupported);
    }

    /// Drop the cache entry for an arity entirely (back to `NotAttempted`), so
    /// a later [`Self::should_attempt`] returns `true` and the arity can be
    /// re-lowered.
    ///
    /// Used by cross-defn invalidation: a lowering that specialized against
    /// another defn is stale once that defn is rebound.
    pub fn invalidate(&self, id: u64) {
        self.entries.write().unwrap().remove(&id);
    }

    /// Evict cached entries idle longer than `ttl_secs`, skipping any arity
    /// `pinned` reports as still needed (published native code or an
    /// in-flight compile: the IR is the deoptimization fallback).
    ///
    /// Returns the evicted arity ids so the caller can drop their JIT
    /// bookkeeping too; see [`Tiers::sweep`](crate::tiered::tiers::Tiers::sweep).
    pub fn sweep(&self, now: u64, ttl_secs: u64, pinned: impl Fn(u64) -> bool) -> Vec<u64> {
        let mut evicted = Vec::new();
        let mut guard = self.entries.write().unwrap();
        guard.retain(|&id, entry| {
            let IrCacheEntry::Cached { last_access, .. } = entry else {
                return true;
            };
            let idle = now.saturating_sub(last_access.load(Ordering::Relaxed));
            if idle <= ttl_secs {
                return true;
            }
            if pinned(id) {
                return true;
            }
            evicted.push(id);
            false
        });
        evicted
    }
}

impl Default for IrCache {
    fn default() -> Self {
        Self::new()
    }
}
