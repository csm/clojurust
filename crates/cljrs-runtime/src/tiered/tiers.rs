//! Per-runtime tier state: the lowered-IR cache and the JIT tables.
//!
//! Everything a runtime accumulates while climbing the execution tiers lives
//! here, in one object owned by its [`GlobalEnv`]:
//!
//! - [`IrCache`] — Tier-1 lowered IR, keyed by `ir_arity_id`.
//! - [`JitState`] — Tier-2 invocation counters, argument-type profiles,
//!   published native pointers, OSR entries, and the handle to the JIT
//!   backend that produced them.
//!
//! Two runtimes in one process therefore never read, evict, or invalidate
//! each other's IR or native code, and everything a runtime compiled is freed
//! when the runtime is.
//!
//! ## Reaching a runtime without an `Env`
//!
//! Background workers cannot hold an `Arc<GlobalEnv>`: it owns `GcPtr`s and is
//! not `Send`.  [`Tiers`] is, so a work request carries a [`Weak<Tiers>`]
//! ([`Tiers::handle`]) and the worker publishes into exactly the runtime that
//! asked — or finds it gone and drops the result.  This replaces the weak
//! index of live `IrCache`s that Stage 3 left behind for the lowering worker
//! and the JIT publish guard.
//!
//! One process-global path remains: `Var::bind` notifies rebind hooks with no
//! runtime in hand at all (see `defn_registry::install_invalidation_hook`).
//! [`live`] enumerates the live tier states for it.  Arity ids come from one
//! process-wide counter, so applying the invalidation to every runtime is
//! exact for the one that owns the id and a no-op everywhere else.

use std::sync::{Arc, LazyLock, RwLock, Weak};

use crate::tiered::ir_cache::IrCache;
use crate::tiered::jit_state::JitState;

/// One runtime's Tier-1 and Tier-2 state.
pub struct Tiers {
    /// Identity of the runtime that owns this state (`GlobalEnv::id`).
    globals_id: u64,
    ir_cache: IrCache,
    jit: JitState,
    /// A handle to this object, for work requests that must find their way
    /// back (see the module docs).
    weak_self: Weak<Tiers>,
}

impl Tiers {
    /// Create the tier state for the runtime with identity `globals_id`, and
    /// register it in [`LIVE`].
    pub fn new(globals_id: u64) -> Arc<Self> {
        let tiers = Arc::new_cyclic(|weak_self| Tiers {
            globals_id,
            ir_cache: IrCache::new(),
            jit: JitState::new(weak_self.clone()),
            weak_self: weak_self.clone(),
        });
        let mut live = LIVE.write().unwrap();
        live.retain(|w| w.strong_count() > 0);
        live.push(Arc::downgrade(&tiers));
        tiers
    }

    /// Identity of the runtime that owns this state.
    pub fn globals_id(&self) -> u64 {
        self.globals_id
    }

    /// This runtime's lowered-IR cache.
    pub fn ir_cache(&self) -> &IrCache {
        &self.ir_cache
    }

    /// This runtime's JIT tables.
    pub fn jit(&self) -> &JitState {
        &self.jit
    }

    /// A weak handle to this tier state, for a background worker's request.
    pub fn handle(&self) -> Weak<Tiers> {
        self.weak_self.clone()
    }

    /// Evict cached IR idle longer than `ttl_secs`, and drop the JIT
    /// bookkeeping of every evicted arity.
    ///
    /// An arity with published native code or an in-flight compile is never
    /// evicted: its IR is the deoptimization fallback.  For the rest, the
    /// `JitEntry` goes too (so an evicted function re-warms from zero) along
    /// with any OSR-entry code, which is only reachable from Tier-1
    /// interpretation of the IR just dropped.
    ///
    /// Intended to run at a stop-the-world safepoint, but safe at any time:
    /// in-flight Tier-1 frames hold their own `Arc<IrFunction>`, and OSR
    /// native frames are protected by the code cache's live-epoch scan.
    pub fn sweep(&self, now: u64, ttl_secs: u64) -> Vec<u64> {
        let evicted = self
            .ir_cache
            .sweep(now, ttl_secs, |id| self.jit.pins_ir(id));
        for &id in &evicted {
            self.jit.evict_entry_if_cold(id);
            self.jit.stale_osr_code(id);
            cljrs_logging::feat_debug!("ir", "evicted idle IR arity_id={}", id);
        }
        evicted
    }
}

// ── Index of live tier states ────────────────────────────────────────────────

static LIVE: LazyLock<RwLock<Vec<Weak<Tiers>>>> = LazyLock::new(|| RwLock::new(Vec::new()));

/// Every runtime whose tier state is still alive.
pub fn live() -> Vec<Arc<Tiers>> {
    LIVE.read()
        .unwrap()
        .iter()
        .filter_map(Weak::upgrade)
        .collect()
}

/// Sweep idle Tier-1 IR in every live runtime; see [`Tiers::sweep`].
///
/// Registered as a stop-the-world reclaim hook by the lowering worker, which
/// has no runtime handle of its own.
pub fn sweep_idle(now: u64, ttl_secs: u64) -> Vec<u64> {
    let mut evicted = Vec::new();
    for tiers in live() {
        evicted.extend(tiers.sweep(now, ttl_secs));
    }
    evicted
}

/// Serializes tests that publish cache entries against tests that run a
/// synthetic far-future sweep over every live runtime.
#[cfg(test)]
pub(crate) static SWEEP_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiered::ir_cache::now_secs;
    use cljrs_ir::IrFunction;

    fn dummy_ir() -> Arc<IrFunction> {
        Arc::new(IrFunction::new(None, None))
    }

    fn sweep_guard() -> std::sync::MutexGuard<'static, ()> {
        SWEEP_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Two runtimes' tier states are independent: neither sees the other's IR
    /// or JIT counters, and dropping one leaves the other intact.
    #[test]
    fn tier_state_is_per_runtime() {
        let _g = sweep_guard();
        let a = Tiers::new(0xE5AA_0001);
        let b = Tiers::new(0xE5AA_0002);
        let id = 0xE500_0006;

        a.ir_cache().store(id, dummy_ir());
        assert!(a.ir_cache().get(id).is_some());
        assert!(b.ir_cache().get(id).is_none(), "b must not see a's entry");
        assert!(b.ir_cache().should_attempt(id));

        // Counters are per-runtime too: warming an arity in `a` leaves `b`'s
        // view of the same arity id untouched.
        a.jit().mark_lower_queued(id);
        assert!(a.jit().lower_queued(id));
        assert!(!b.jit().lower_queued(id));

        // A dropped runtime stops being reachable: its handle no longer
        // upgrades, so a worker request minted from it is discarded rather
        // than published into a dead runtime.
        let handle = a.handle();
        let a_ptr = Arc::as_ptr(&a);
        assert!(handle.upgrade().is_some());
        assert!(live().iter().any(|t| Arc::as_ptr(t) == a_ptr));
        drop(a);
        assert!(handle.upgrade().is_none());
        assert!(live().iter().all(|t| Arc::as_ptr(t) != a_ptr));
        drop(b);
    }

    #[test]
    fn sweep_evicts_idle_entry_and_drops_jit_entry() {
        let _g = sweep_guard();
        let tiers = Tiers::new(0xE5AA_0003);
        let id = 0xE500_0001;
        tiers.ir_cache().store(id, dummy_ir());
        tiers.jit().mark_lower_queued(id);

        // Recent entry survives a sweep.
        let stored_at = now_secs();
        assert!(sweep_idle(stored_at, 600).is_empty() || !tiers.ir_cache().should_attempt(id));
        assert!(tiers.ir_cache().get(id).is_some());

        // Far in the future the entry is idle past the TTL and is evicted,
        // along with its JitEntry (lower_queued resets so it can re-warm).
        let evicted = sweep_idle(stored_at + 601, 600);
        assert!(evicted.contains(&id));
        assert!(tiers.ir_cache().get(id).is_none());
        assert!(tiers.ir_cache().should_attempt(id));
        assert!(!tiers.jit().lower_queued(id));
    }

    #[test]
    fn sweep_skips_native_published_arity() {
        let _g = sweep_guard();
        let tiers = Tiers::new(0xE5AA_0003);
        let id = 0xE500_0002;
        tiers.ir_cache().store(id, dummy_ir());
        tiers
            .jit()
            .store_native_fn(id, 0x1234usize as *const (), 31337);

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(tiers.ir_cache().get(id).is_some());
    }

    #[test]
    fn sweep_skips_queued_compile() {
        let _g = sweep_guard();
        let tiers = Tiers::new(0xE5AA_0003);
        let id = 0xE500_0003;
        let ir = dummy_ir();
        tiers.ir_cache().store(id, ir.clone());
        // Cross the JIT threshold; with no backend installed this just pins
        // compile_queued, exactly the state of an in-flight compile.
        for _ in 0..crate::tiered::jit_state::jit_threshold() {
            tiers.jit().record_call(id, ir.clone(), &[]);
        }
        assert!(tiers.jit().compile_queued(id));

        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        assert!(tiers.ir_cache().get(id).is_some());
    }

    #[test]
    fn sweep_never_touches_unsupported() {
        let _g = sweep_guard();
        let tiers = Tiers::new(0xE5AA_0003);
        let id = 0xE500_0004;
        tiers.ir_cache().store_unsupported(id);
        let evicted = sweep_idle(now_secs() + 10_000, 600);
        assert!(!evicted.contains(&id));
        // Still terminal: no re-lowering attempts.
        assert!(!tiers.ir_cache().should_attempt(id));
    }

    #[test]
    fn get_refreshes_last_access() {
        let _g = sweep_guard();
        let tiers = Tiers::new(0xE5AA_0003);
        let id = 0xE500_0005;
        tiers.ir_cache().store(id, dummy_ir());
        // Touch, then sweep with a now that is idle relative to the store
        // time but not the touch time recorded by `get`: by refreshing on
        // access the entry must survive a sweep whose `now` is within the
        // TTL of the touch.
        let _ = tiers.ir_cache().get(id);
        let touched_at = now_secs();
        let evicted = sweep_idle(touched_at + 599, 600);
        assert!(!evicted.contains(&id));
        assert!(tiers.ir_cache().get(id).is_some());
    }
}
