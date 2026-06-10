//! Epoch-tagged registry of compiled JIT code, with stop-the-world reclamation
//! (Phase 10.2 — code unloading).
//!
//! ## Why this exists
//!
//! The REPL re-runs `defn` constantly; each redefinition produces a fresh
//! `CljxFn` arity (new `ir_arity_id`) and orphans the previous one's native
//! code.  Without reclamation, executable memory grows unbounded across a long
//! session.  `cranelift-jit` never frees a module's code on drop — the memory
//! must be released explicitly via [`JITModule::free_memory`], which is only
//! sound once no `fn` pointer into that module will be called again.
//!
//! ## How reclamation stays safe
//!
//! Every compiled module is tagged with a monotonically increasing **epoch**
//! and stored here.  Two events drive its lifecycle:
//!
//! - **Redefinition** ([`mark_stale`]): when a var holding a function is
//!   rebound, the value layer's rebind hook nulls the dispatch pointer (so no
//!   *new* call reaches the old code) and moves the old epoch to the stale set.
//! - **Safepoint reclaim** ([`reclaim_at_stw`]): at the existing stop-the-world
//!   GC safepoint — when every mutator thread is parked — we scan all active JIT
//!   frames ([`cljrs_eval::jit_state::live_epochs`]).  A stale epoch with **no**
//!   live frame can have no in-flight and no future caller, so its module memory
//!   is freed.  This piggybacks on the GC's quiescent point and sidesteps the
//!   unload-vs-execute race without a separate protocol.
//!
//! Because emitted code embeds no GC pointers (constants are materialized via
//! `rt_abi` runtime calls), freeing a module never disturbs the heap.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use crate::jit_compiler::CompiledFn;

/// Monotonic epoch source.  Never reused, so an epoch uniquely identifies one
/// compiled module for the life of the process.
static NEXT_EPOCH: AtomicU64 = AtomicU64::new(1);

/// One entry in the registry: the owning module plus accounting metadata.
struct CodeRecord {
    /// The arity this code was compiled for (diagnostics only).
    arity_id: u64,
    compiled: CompiledFn,
}

#[derive(Default)]
struct CacheState {
    /// Current, in-use compiled code, keyed by epoch.
    live: HashMap<u64, CodeRecord>,
    /// Superseded code awaiting reclamation, keyed by epoch.
    stale: HashMap<u64, CodeRecord>,
    /// Cumulative count of modules whose memory has been freed.
    reclaimed_count: u64,
    /// Cumulative bytes of machine code freed.
    reclaimed_bytes: u64,
}

fn cache() -> &'static Mutex<CacheState> {
    static CACHE: OnceLock<Mutex<CacheState>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(CacheState::default()))
}

/// Register a freshly compiled module, returning its reclamation epoch.
///
/// The epoch is published into the dispatch table alongside the function
/// pointer (see the worker), so a later [`mark_stale`] can find this module.
pub(crate) fn register(arity_id: u64, compiled: CompiledFn) -> u64 {
    let epoch = NEXT_EPOCH.fetch_add(1, Ordering::Relaxed);
    let mut state = cache().lock().unwrap();
    state.live.insert(epoch, CodeRecord { arity_id, compiled });
    epoch
}

/// Move the module for `epoch` from the live set to the stale set, scheduling it
/// for reclamation at the next safepoint.  No-op if the epoch is unknown or
/// already stale.
pub(crate) fn mark_stale(epoch: u64) {
    let mut state = cache().lock().unwrap();
    if let Some(record) = state.live.remove(&epoch) {
        cljrs_logging::feat_debug!(
            "jit",
            "mark stale epoch={} arity_id={} ({} bytes)",
            epoch,
            record.arity_id,
            record.compiled.code_size,
        );
        state.stale.insert(epoch, record);
    }
}

/// Decide which stale epochs are safe to free: those with no active frame.
///
/// Factored out as a pure function over epoch sets so the safety rule is
/// directly unit-testable without constructing real modules.
fn select_reclaimable(stale_epochs: &HashSet<u64>, live_frames: &HashSet<u64>) -> Vec<u64> {
    stale_epochs
        .iter()
        .copied()
        .filter(|e| !live_frames.contains(e))
        .collect()
}

/// Reclaim stale modules with no live frame.  **Must be called at a
/// stop-the-world safepoint** (all mutator threads parked) so the active-frame
/// scan is stable and no freed pointer can be entered afterward.
///
/// Returns the number of modules freed this pass.
pub fn reclaim_at_stw() -> usize {
    let live_frames = cljrs_eval::jit_state::live_epochs();
    let mut state = cache().lock().unwrap();

    let stale_epochs: HashSet<u64> = state.stale.keys().copied().collect();
    let to_free = select_reclaimable(&stale_epochs, &live_frames);

    let mut freed = 0usize;
    for epoch in to_free {
        if let Some(record) = state.stale.remove(&epoch) {
            let bytes = record.compiled.code_size as u64;
            // SAFETY: the module is stale (its dispatch pointer was nulled in
            // `mark_stale`, so no new call can reach it) and no frame is
            // executing its code (epoch ∉ live_frames, computed at STW with all
            // mutators parked).  Therefore no `fn` pointer into this module is
            // in use or will be used again.
            unsafe {
                record.compiled.module.free_memory();
            }
            state.reclaimed_count += 1;
            state.reclaimed_bytes += bytes;
            freed += 1;
            cljrs_logging::feat_debug!(
                "jit",
                "reclaimed epoch={} arity_id={} ({} bytes)",
                epoch,
                record.arity_id,
                bytes,
            );
        }
    }
    if freed > 0 {
        cljrs_logging::feat_debug!(
            "jit",
            "reclaim pass freed {} module(s); {} live, {} still-stale",
            freed,
            state.live.len(),
            state.stale.len(),
        );
    }
    freed
}

// ── Diagnostics / test accessors ────────────────────────────────────────────

/// Number of live (in-use) compiled modules.
pub fn live_count() -> usize {
    cache().lock().unwrap().live.len()
}

/// Number of stale modules awaiting reclamation.
pub fn stale_count() -> usize {
    cache().lock().unwrap().stale.len()
}

/// Cumulative number of modules whose memory has been freed.
pub fn reclaimed_count() -> u64 {
    cache().lock().unwrap().reclaimed_count
}

/// Cumulative bytes of machine code freed.
pub fn reclaimed_bytes() -> u64 {
    cache().lock().unwrap().reclaimed_bytes
}

// ── Test-only inspection helpers ────────────────────────────────────────────

/// True if `epoch` is currently in the stale (awaiting-reclamation) set.
#[cfg(test)]
pub(crate) fn is_stale(epoch: u64) -> bool {
    cache().lock().unwrap().stale.contains_key(&epoch)
}

/// True if `epoch` is currently in the live set.
#[cfg(test)]
pub(crate) fn is_live(epoch: u64) -> bool {
    cache().lock().unwrap().live.contains_key(&epoch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reclaimable_excludes_live_frames() {
        let stale: HashSet<u64> = [1, 2, 3, 4].into_iter().collect();
        let live: HashSet<u64> = [2, 4].into_iter().collect();
        let mut got = select_reclaimable(&stale, &live);
        got.sort_unstable();
        assert_eq!(
            got,
            vec![1, 3],
            "only stale epochs without a live frame free"
        );
    }

    #[test]
    fn reclaimable_empty_when_all_live() {
        let stale: HashSet<u64> = [5, 6].into_iter().collect();
        let live: HashSet<u64> = [5, 6, 7].into_iter().collect();
        assert!(select_reclaimable(&stale, &live).is_empty());
    }

    #[test]
    fn reclaimable_all_when_none_live() {
        let stale: HashSet<u64> = [8, 9].into_iter().collect();
        let live: HashSet<u64> = HashSet::new();
        let mut got = select_reclaimable(&stale, &live);
        got.sort_unstable();
        assert_eq!(got, vec![8, 9]);
    }
}

/// End-to-end reclamation test over *real* compiled modules: a redefinition
/// stales native code, an executing frame defers its release, and the next
/// safepoint frees it (calling `JITModule::free_memory`).
///
/// Gated to the default GC build: under `no-gc`, `lower_via_rust` runs the
/// escape-blacklist check which can reject otherwise-fine functions.
#[cfg(all(test, not(feature = "no-gc")))]
mod reclaim_integration {
    use std::sync::Arc;

    /// Lower a tiny function to IR on a generously sized stack (Clojure eval is
    /// deeply recursive), mirroring the codegen-crate test helper.
    fn build_ir(name: &str, params: &[Arc<str>], body_src: &str) -> cljrs_ir::IrFunction {
        let name = name.to_string();
        let params = params.to_vec();
        let body_src = body_src.to_string();
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let mut parser = cljrs_reader::Parser::new(body_src, "<jit-test>".to_string());
                let mut forms = Vec::new();
                while let Ok(Some(f)) = parser.parse_one() {
                    forms.push(f);
                }
                let globals = cljrs_stdlib::standard_env();
                let mut env = cljrs_eval::Env::new(globals, "user");
                cljrs_compiler::aot::lower_via_rust(Some(&name), "user", &params, &forms, &mut env)
                    .expect("lowering should succeed")
            })
            .unwrap()
            .join()
            .unwrap()
    }

    #[test]
    fn redefinition_reclaims_native_code_only_when_no_frame_is_live() {
        let ir = build_ir("f", &[Arc::from("x")], "(+ x 1)");
        let arity_id = 0xC0DE_0001u64;

        // Emulate the worker: compile, register (→ epoch), publish ptr + epoch.
        let compiled =
            crate::jit_compiler::compile_jit(&format!("__cljrs_jit_{arity_id}"), &ir).unwrap();
        let fn_ptr = compiled.fn_ptr;
        let epoch = crate::code_cache::register(arity_id, compiled);
        cljrs_eval::jit_state::store_native_fn(arity_id, fn_ptr, epoch);
        assert!(crate::code_cache::is_live(epoch));
        assert_eq!(
            cljrs_eval::jit_state::get_native_fn(arity_id),
            Some((fn_ptr, epoch)),
            "dispatch table should resolve to the compiled code + epoch",
        );

        // Emulate redefinition: the rebind hook nulls dispatch and stales the
        // old epoch.
        let taken = cljrs_eval::jit_state::take_native_epoch(arity_id).unwrap();
        assert_eq!(taken, epoch);
        assert_eq!(
            cljrs_eval::jit_state::get_native_fn(arity_id),
            None,
            "future calls must no longer dispatch to stale code",
        );
        crate::code_cache::mark_stale(epoch);
        assert!(crate::code_cache::is_stale(epoch));
        assert!(!crate::code_cache::is_live(epoch));

        // A frame executing this epoch defers reclamation.
        {
            let _frame = cljrs_eval::jit_state::push_jit_frame(epoch);
            crate::code_cache::reclaim_at_stw();
            assert!(
                crate::code_cache::is_stale(epoch),
                "must not free code while a frame is executing it",
            );
        }

        // With no live frame, the next safepoint reclaim frees the module.
        let before = crate::code_cache::reclaimed_count();
        let freed = crate::code_cache::reclaim_at_stw();
        assert!(
            freed >= 1,
            "stale module with no live frame should be freed"
        );
        assert!(!crate::code_cache::is_stale(epoch));
        assert!(crate::code_cache::reclaimed_count() > before);
    }
}
