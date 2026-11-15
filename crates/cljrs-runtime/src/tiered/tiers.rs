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
//! each other's IR or native code.  Dropping a runtime drops its IR outright
//! and hands every epoch it still has published to the code cache for
//! reclamation (see [`JitState`]'s `Drop`), so its compiled modules are freed
//! at the next safepoint at which no thread is executing them.  The
//! cross-defn registry (`defn_registry`) is process-global, but `Tiers`
//! retires this runtime's registrations, dependent edges and relower marks
//! on drop, and the registrations of evicted arities on [`Tiers::sweep`].
//! The one exception is `^:async` poll functions, which are registered outside the
//! epoch-tagged cache and live for the process — see
//! `cljrs_compiler::jit::async_jit`.
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
        // The cross-defn registry must not keep evicted IR alive (#508).
        crate::tiered::defn_registry::retire_arities(self.globals_id, &evicted);
        for &id in &evicted {
            self.jit.evict_entry_if_cold(id);
            self.jit.stale_osr_code(id);
            tracing::debug!(target: "ir", "evicted idle IR arity_id={}", id);
        }
        evicted
    }
}

impl Drop for Tiers {
    /// Release everything the process-global cross-defn registry holds on
    /// this runtime's behalf (#508).  Runs when the last strong handle goes,
    /// so a background worker that upgraded [`Tiers::handle`] finishes (and
    /// registers) first, and its registration is retired here.
    fn drop(&mut self) {
        crate::tiered::defn_registry::retire_runtime(self.globals_id);
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

    // ── Issue #508: registry ownership follows the runtime ───────────────

    fn referenced(ns: &str, name: &str) -> std::collections::HashSet<(Arc<str>, Arc<str>)> {
        let mut r = std::collections::HashSet::new();
        r.insert((Arc::from(ns), Arc::from(name)));
        r
    }

    /// Dropping a runtime releases the registry's strong IR references and
    /// all of its dependent/relower bookkeeping; an in-flight snapshot taken
    /// before the drop stays usable until it is released.
    #[test]
    fn dropping_runtime_releases_registry_ir() {
        use crate::tiered::defn_registry as reg;
        let _g = sweep_guard();
        let gid = 0xE5AB_0001u64;
        let (ns, name) = ("test.retire-ns", "callee");
        let (callee_id, caller_id) = (0xE5AB_0101u64, 0xE5AB_0102u64);
        let tiers = Tiers::new(gid);

        let ir = dummy_ir();
        let weak = Arc::downgrade(&ir);
        tiers.ir_cache().store(callee_id, ir.clone());
        reg::register_defn(
            gid,
            &Arc::from(ns),
            &Arc::from(name),
            vec![(callee_id, 1, false, ir)],
        );
        // A caller consumes the callee (in-flight snapshot + edge), and a
        // relower mark is pending for another arity of this runtime.
        let snapshot = reg::snapshot_externals(gid, caller_id, &referenced(ns, name));
        assert_eq!(snapshot.len(), 1);
        reg::record_dependents(gid, 0xE5AB_0103, vec![(Arc::from(ns), Arc::from("other"))]);
        reg::on_redefined(ns, "other");
        assert!(reg::relower_marked(0xE5AB_0103));
        assert_eq!(reg::counts_for(gid), (1, 1, 2, 1));

        drop(tiers);
        assert_eq!(reg::counts_for(gid), (0, 0, 0, 0));
        assert!(!reg::relower_marked(0xE5AB_0103));
        assert!(reg::externals_for(gid, &referenced(ns, name)).is_empty());

        // The in-flight consumer still holds a valid strong snapshot...
        assert!(weak.upgrade().is_some());
        assert_eq!(snapshot[0].arity_irs.len(), 1);
        // ...and once it finishes, nothing keeps the IR alive.
        drop(snapshot);
        assert!(weak.upgrade().is_none());
    }

    /// Evicting a cold registered arity retires its registration, so the
    /// IR is actually freed; other runtimes' registrations are untouched.
    #[test]
    fn sweep_retires_evicted_registration() {
        use crate::tiered::defn_registry as reg;
        let _g = sweep_guard();
        let (gid, other_gid) = (0xE5AB_0002u64, 0xE5AB_0003u64);
        let (ns, name) = ("test.evict-ns", "cold");
        let id = 0xE5AB_0201u64;
        let tiers = Tiers::new(gid);

        let ir = dummy_ir();
        let weak = Arc::downgrade(&ir);
        tiers.ir_cache().store(id, ir.clone());
        reg::register_defn(
            gid,
            &Arc::from(ns),
            &Arc::from(name),
            vec![(id, 1, false, ir)],
        );
        reg::register_defn(
            other_gid,
            &Arc::from(ns),
            &Arc::from(name),
            vec![(id, 1, false, dummy_ir())],
        );
        reg::record_dependents(gid, id, vec![(Arc::from(ns), Arc::from("dep"))]);

        let evicted = tiers.sweep(now_secs() + 10_000, 600);
        assert!(evicted.contains(&id));
        assert!(reg::externals_for(gid, &referenced(ns, name)).is_empty());
        assert!(weak.upgrade().is_none(), "evicted IR must be released");
        // The evicted arity's outgoing edge is gone too.
        assert!(!reg::on_redefined(ns, "dep").contains(&id));
        // Scoped by runtime identity.
        assert_eq!(
            reg::externals_for(other_gid, &referenced(ns, name)).len(),
            1
        );
        reg::retire_runtime(other_gid);
    }

    /// Creating and destroying runtimes with unique names returns the
    /// registry to baseline for each of them.
    #[test]
    fn runtime_churn_returns_registry_to_baseline() {
        use crate::tiered::defn_registry as reg;
        let _g = sweep_guard();
        let mut weaks = Vec::new();
        for i in 0..32u64 {
            let gid = 0xE5AC_0000 + i;
            let id = 0xE5AC_1000 + i;
            let tiers = Tiers::new(gid);
            let name: Arc<str> = Arc::from(format!("churn-{i}").as_str());
            let ir = dummy_ir();
            weaks.push((gid, Arc::downgrade(&ir)));
            tiers.ir_cache().store(id, ir.clone());
            reg::register_defn(
                gid,
                &Arc::from("test.churn-ns"),
                &name,
                vec![(id, 1, false, ir)],
            );
            reg::record_dependents(gid, id + 0x100, vec![(Arc::from("test.churn-ns"), name)]);
            assert_eq!(reg::counts_for(gid).0, 1);
        }
        for (gid, weak) in weaks {
            assert_eq!(reg::counts_for(gid), (0, 0, 0, 0));
            assert!(reg::registered_irs(gid).is_empty());
            assert!(weak.upgrade().is_none());
        }
    }
}
