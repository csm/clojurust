//! Background IR-lowering worker (Phase 10.7).
//!
//! When a function's Tier-0 (tree-walk) invocation count crosses
//! [`crate::jit_state::ir_threshold`], the dispatch seam macro-expands its
//! arity bodies on the calling thread (macros need the interpreter) and ships
//! the expanded forms here.  This worker runs the expensive, Env-free half of
//! lowering — ANF construction, inlining, escape analysis, region promotion —
//! and publishes the result to [`crate::ir_cache`], after which dispatch
//! switches to the Tier-1 IR interpreter.  Continued invocations promote to
//! the JIT exactly as before (the Tier-1 counter restarts at publish).
//!
//! The worker is **not** a GC mutator: it never touches `Value`, `Env`, or
//! the GC heap.  Everything it consumes (`Form`, `Arc<str>`) is plain data,
//! and all registries it updates (`ir_cache`, `defn_registry`, `jit_state`)
//! are lock-protected statics.
//!
//! ## Rebind races
//!
//! The mutator can rebind a defn while this worker lowers a dependent.  Two
//! mechanisms make that safe (see `defn_registry` for the lock-order
//! argument):
//!
//! 1. `snapshot_externals` (called inside `lower_expanded_arity`) records the
//!    dependent edges *atomically* with reading the externals, so a rebind
//!    can never miss an in-flight consumer — it always sets the relower mark.
//! 2. This worker is the **only consumer** of relower marks (`take_relower`).
//!    After `store_cached` it re-peeks the mark; if a rebind landed during
//!    optimization, the just-published IR is invalidated and the arity is
//!    re-lowered with fresh externals (bounded retries).  The dispatch seam
//!    only peeks marks (`relower_marked`) to decide when to enqueue.

use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, SyncSender};

use cljrs_reader::Form;
use std::sync::Arc;

/// One arity's lowering job: everything `lower_expanded_arity` needs, with
/// the body already macro-expanded on the mutator thread.
pub(crate) struct LowerArityRequest {
    pub arity_id: u64,
    pub params: Vec<Arc<str>>,
    pub rest_param: Option<Arc<str>>,
    pub destructure_params: Vec<(usize, Form)>,
    pub destructure_rest: Option<Form>,
    pub expanded_body: Vec<Form>,
}

/// A whole function's lowering job.  All arities travel together so the
/// cross-defn registration (`register_defn`) sees the complete fn, exactly
/// as eager lowering does.
pub(crate) struct LowerRequest {
    pub globals_id: usize,
    pub name: Option<Arc<str>>,
    pub ns: Arc<str>,
    pub is_async: bool,
    pub arities: Vec<LowerArityRequest>,
}

/// Rebind-retry budget per arity: how many times a lowering invalidated by a
/// concurrent rebind is redone before giving up and leaving the arity at
/// `NotAttempted` (the dispatch seam will re-trigger it).
const MAX_LOWER_ATTEMPTS: u32 = 3;

static SENDER: OnceLock<SyncSender<LowerRequest>> = OnceLock::new();

/// Enqueue a lowering request, lazily starting the worker on first use.
/// Returns `false` when the queue is full — the caller must *not* set
/// `lower_queued`, so the next threshold check retries.
pub(crate) fn enqueue(req: LowerRequest) -> bool {
    let tx = SENDER.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::sync_channel::<LowerRequest>(256);
        // The sweep hook and the rebind-invalidation hook are only needed
        // once background lowering actually produces cache entries.
        cljrs_env::gc_roots::set_stw_reclaim_hook(|| {
            crate::ir_cache::sweep_idle(
                crate::ir_cache::now_secs(),
                crate::ir_cache::ir_cache_ttl_secs(),
            );
        });
        crate::defn_registry::install_invalidation_hook();
        std::thread::Builder::new()
            .name("cljrs-ir-lower".into())
            .spawn(move || worker_loop(rx))
            .expect("failed to spawn IR lowering worker thread");
        tx
    });
    tx.try_send(req).is_ok()
}

fn worker_loop(rx: Receiver<LowerRequest>) {
    for req in &rx {
        // A panic while lowering one function must not kill the worker; the
        // affected arities simply stay at Tier 0 (their cache entries were
        // left absent or are re-triggered by the dispatch seam).
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            process_request(&req);
        }));
        if result.is_err() {
            cljrs_logging::feat_debug!(
                "ir",
                "background lower panicked for {:?}; staying at Tier 0",
                req.name
            );
        }
    }
}

fn process_request(req: &LowerRequest) {
    // Arities lowered (or already cached), collected for the cross-defn
    // registry: (param_count, is_variadic, ir).
    let mut registered: Vec<(usize, bool, Arc<cljrs_ir::IrFunction>)> = Vec::new();

    for arity in &req.arities {
        let id = arity.arity_id;

        // Skip arities that already have a terminal cache state — unless a
        // relower mark is pending, in which case the cached IR is stale
        // (rebound external) and must be redone.
        if !crate::ir_cache::should_attempt(id) && !crate::defn_registry::relower_marked(id) {
            if let Some(ir) = crate::ir_cache::get_cached(id) {
                registered.push((arity.params.len(), arity.rest_param.is_some(), ir));
            }
            continue;
        }

        let mut terminal = false;
        for _ in 0..MAX_LOWER_ATTEMPTS {
            // Consume any pending relower mark and clear the slate before
            // lowering; a rebind after this point re-sets the mark, which
            // the post-store peek below catches.
            crate::defn_registry::take_relower(id);
            crate::ir_cache::invalidate(id);

            match crate::lower::lower_expanded_arity(
                req.name.as_deref(),
                &arity.params,
                arity.rest_param.as_ref(),
                &arity.destructure_params,
                arity.destructure_rest.as_ref(),
                &arity.expanded_body,
                &req.ns,
                req.globals_id,
                Some(id),
                true,
                req.is_async,
            ) {
                Ok((ir, _used)) => {
                    let ir = Arc::new(ir);
                    crate::ir_cache::store_cached(id, ir.clone());
                    if crate::defn_registry::relower_marked(id) {
                        // An external this lowering specialized against was
                        // rebound mid-flight; the stored IR is stale.
                        crate::ir_cache::invalidate(id);
                        continue;
                    }
                    crate::jit_state::on_ir_published(id);
                    registered.push((arity.params.len(), arity.rest_param.is_some(), ir));
                    cljrs_logging::feat_debug!(
                        "ir",
                        "background lower published arity_id={} ({:?})",
                        id,
                        req.name
                    );
                    terminal = true;
                    break;
                }
                Err(e) => {
                    crate::ir_cache::store_unsupported(id);
                    cljrs_logging::feat_debug!(
                        "ir",
                        "background lower unsupported arity_id={} ({:?}): {}",
                        id,
                        req.name,
                        e
                    );
                    terminal = true;
                    break;
                }
            }
        }

        if !terminal {
            // Retry budget exhausted under rebind churn.  Leave the entry
            // absent and clear `lower_queued` so the dispatch seam can
            // re-trigger lowering on a later call.
            crate::ir_cache::invalidate(id);
            crate::jit_state::clear_lower_queued(id);
            cljrs_logging::feat_debug!(
                "ir",
                "background lower abandoned after {} rebind retries arity_id={} ({:?})",
                MAX_LOWER_ATTEMPTS,
                id,
                req.name
            );
        }
    }

    // Publish this defn so later lowerings of *other* functions can
    // region-promote calls into it (stage 4) — mirrors eager lowering.
    // Anonymous or async fns are not callable cross-defn by name.
    if !req.is_async
        && !registered.is_empty()
        && let Some(name) = &req.name
    {
        crate::defn_registry::register_defn(req.globals_id, &req.ns, name, registered);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> cljrs_types::span::Span {
        cljrs_types::span::Span::new(Arc::new("<test>".to_string()), 0, 0, 1, 1)
    }

    fn sym(name: &str) -> Form {
        Form::new(cljrs_reader::form::FormKind::Symbol(name.into()), span())
    }

    /// An identity-function arity request: `(fn [x] x)`.
    fn identity_request(arity_id: u64, fn_name: &str, ns: &str, gid: usize) -> LowerRequest {
        LowerRequest {
            globals_id: gid,
            name: Some(Arc::from(fn_name)),
            ns: Arc::from(ns),
            is_async: false,
            arities: vec![LowerArityRequest {
                arity_id,
                params: vec![Arc::from("x")],
                rest_param: None,
                destructure_params: Vec::new(),
                destructure_rest: None,
                expanded_body: vec![sym("x")],
            }],
        }
    }

    #[test]
    fn process_request_publishes_and_registers() {
        let id = 0xC500_0001u64;
        let gid = 0xC500_0001usize;
        let ns = "test.worker-ns";
        let req = identity_request(id, "ident", ns, gid);

        assert!(crate::ir_cache::should_attempt(id));
        process_request(&req);

        // IR published and the defn registered for cross-defn promotion.
        assert!(crate::ir_cache::get_cached(id).is_some());
        let mut referenced = std::collections::HashSet::new();
        referenced.insert((Arc::<str>::from(ns), Arc::<str>::from("ident")));
        let externals = crate::defn_registry::externals_for(gid, &referenced);
        assert_eq!(externals.len(), 1);

        // A second request for the same arity is a no-op (terminal state).
        let before = crate::ir_cache::get_cached(id).unwrap();
        process_request(&req);
        let after = crate::ir_cache::get_cached(id).unwrap();
        assert!(Arc::ptr_eq(&before, &after));
    }

    #[test]
    fn process_request_relowers_marked_arity_despite_cache_hit() {
        // A rebind of a consumed external invalidates the dependent and
        // marks it; if the (stale) IR is still cached when the worker runs —
        // the mark-then-invalidate window seen from the worker thread — the
        // mark must force a re-lower, and must be consumed by it.
        let id = 0xC500_0011u64;
        let gid = 0xC500_0011usize;
        let callee_ns: Arc<str> = Arc::from("test.worker-relower-ns");
        let callee: Arc<str> = Arc::from("callee");

        // Plant stale IR and a relower mark (via a real rebind drain).
        let stale = Arc::new(cljrs_ir::IrFunction::new(None, None));
        crate::ir_cache::store_cached(id, stale.clone());
        crate::defn_registry::record_dependents(id, vec![(callee_ns.clone(), callee.clone())]);
        crate::defn_registry::on_redefined(&callee_ns, &callee);
        assert!(crate::defn_registry::relower_marked(id));

        process_request(&identity_request(
            id,
            "dependent",
            "test.worker-relower-ns",
            gid,
        ));

        // Fresh IR replaced the stale one and the mark was consumed.
        let fresh = crate::ir_cache::get_cached(id).expect("re-lowered IR");
        assert!(!Arc::ptr_eq(&fresh, &stale));
        assert!(!crate::defn_registry::relower_marked(id));
    }
}
