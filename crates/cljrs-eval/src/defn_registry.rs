//! Cross-defn IR registry for context-driven bump allocation (Phase 10.5).
//!
//! In the AOT flow the whole program is lowered as one IR tree, so stage-4
//! cross-function region promotion (`cljrs_ir::lower::regionalize`) sees
//! every callee.  In the script/REPL flow each top-level `defn` is lowered
//! separately at definition time, so the promotion pass could never resolve a
//! call to another `defn` — `CallWithRegion` (and the hidden region
//! parameter) never appeared in JIT'd code.
//!
//! This module closes that gap:
//!
//! * [`register_defn`] records each eagerly-lowered top-level defn (keyed by
//!   `(GlobalEnv identity, ns, name)` so isolates never consume each other's
//!   IR).
//! * [`externals_for`] hands later lowerings the registered defns they
//!   reference, in `cljrs_ir::lower::ExternalDefn` form.
//! * [`record_dependents`] stores the inverse edges: which arities consumed
//!   which externals.
//! * [`on_redefined`] (called from the var-rebind hook installed by
//!   [`install_invalidation_hook`]) drops the registration and returns every
//!   dependent arity, which the hook invalidates: its cached IR is removed,
//!   any published native code is staled (via the stale-epoch hook,
//!   `jit_state::set_stale_epoch_hook`), and the arity is queued for lazy
//!   re-lowering on its next dispatch ([`take_relower`] in `try_ir_path`).
//!
//! Redefinition correctness is the point of all the bookkeeping: stage 4
//! *clones the callee's body* into the caller, so a caller lowered against
//! the old definition would keep executing it after a redefinition unless
//! invalidated.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock, RwLock};

use cljrs_ir::IrFunction;
use cljrs_ir::lower::ExternalDefn;

/// One registered arity of a top-level defn.
struct RegisteredArity {
    /// Process-unique registry key (mangled; see `arity_key`).
    fn_name: Arc<str>,
    /// Callable parameter count (fixed params only).
    param_count: usize,
    is_variadic: bool,
    ir: Arc<IrFunction>,
}

struct RegisteredDefn {
    arities: Vec<RegisteredArity>,
}

/// Key: (GlobalEnv identity, ns, name).  The identity component keeps
/// same-named defns in different isolates from cross-contaminating.
type DefnKey = (usize, Arc<str>, Arc<str>);

static REGISTRY: LazyLock<RwLock<HashMap<DefnKey, RegisteredDefn>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Inverse dependency edges: `(ns, name)` → arities whose lowering consumed
/// it.  Deliberately *not* keyed by GlobalEnv identity — the rebind hook only
/// sees the function value, so invalidation is over-approximated across
/// isolates (harmless: a spurious re-lower).
static DEPENDENTS: LazyLock<RwLock<HashMap<(Arc<str>, Arc<str>), HashSet<u64>>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

/// Arities whose cached lowering was invalidated and should be re-lowered on
/// their next dispatch.  `RELOWER_PENDING` keeps the dispatch fast path to a
/// single relaxed atomic load.
static RELOWER: LazyLock<RwLock<HashSet<u64>>> = LazyLock::new(|| RwLock::new(HashSet::new()));
static RELOWER_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Mangle a process-unique registry key for one arity.  Never emitted as a
/// symbol — stage-4 clones are renamed `…__rgN` before codegen sees them.
fn arity_key(globals_id: usize, ns: &str, name: &str, param_count: usize) -> Arc<str> {
    Arc::from(format!("{name}__{ns}__g{globals_id:x}__ext{param_count}").as_str())
}

/// Register (or replace) the lowered arities of a top-level defn.
///
/// `arities` supplies `(param_count, is_variadic, ir)` per arity.  Call after
/// every arity of the fn has been lowered and optimized.
pub fn register_defn(
    globals_id: usize,
    ns: &Arc<str>,
    name: &Arc<str>,
    arities: Vec<(usize, bool, Arc<IrFunction>)>,
) {
    let entry = RegisteredDefn {
        arities: arities
            .into_iter()
            .map(|(param_count, is_variadic, ir)| RegisteredArity {
                fn_name: arity_key(globals_id, ns, name, param_count),
                param_count,
                is_variadic,
                ir,
            })
            .collect(),
    };
    REGISTRY
        .write()
        .unwrap()
        .insert((globals_id, ns.clone(), name.clone()), entry);
}

/// Build the externals list for lowering a function: every registered defn in
/// `referenced` (the `(ns, name)` pairs the function's IR loads as globals)
/// for this GlobalEnv.
pub fn externals_for(
    globals_id: usize,
    referenced: &HashSet<(Arc<str>, Arc<str>)>,
) -> Vec<ExternalDefn> {
    if referenced.is_empty() {
        return Vec::new();
    }
    let registry = REGISTRY.read().unwrap();
    let mut out = Vec::new();
    for (ns, name) in referenced {
        let Some(defn) = registry.get(&(globals_id, ns.clone(), name.clone())) else {
            continue;
        };
        out.push(ExternalDefn {
            ns: ns.clone(),
            name: name.clone(),
            arity_fn_names: defn.arities.iter().map(|a| a.fn_name.clone()).collect(),
            param_counts: defn.arities.iter().map(|a| a.param_count).collect(),
            is_variadic: defn.arities.iter().map(|a| a.is_variadic).collect(),
            arity_irs: defn.arities.iter().map(|a| a.ir.clone()).collect(),
        });
    }
    out
}

/// Record that `arity_id`'s lowering consumed the given externals.
pub fn record_dependents(arity_id: u64, used: impl IntoIterator<Item = (Arc<str>, Arc<str>)>) {
    let mut deps = DEPENDENTS.write().unwrap();
    for key in used {
        deps.entry(key).or_default().insert(arity_id);
    }
}

/// Atomically snapshot the externals for lowering `arity_id` *and* record the
/// dependent edges (Phase 10.7).
///
/// Background lowering must not separate these steps: with
/// `externals_for` → optimize → `record_dependents`, a rebind landing between
/// the snapshot and the recording drains `DEPENDENTS` before our edge exists,
/// so no relower mark is ever set and stale IR is published undetected.
///
/// Holding the `REGISTRY` read lock across the `DEPENDENTS` write serializes
/// against [`on_redefined`] (which takes the `REGISTRY` write lock first):
/// a rebind runs either entirely before this call (we see the updated
/// registry and never consume the dead defn) or entirely after it (our edge
/// exists, so the rebind sets the relower mark the worker checks after
/// publishing).  Edges are recorded only for defns actually present in the
/// registry — exactly the set the optimizer can inline.
pub fn snapshot_externals(
    globals_id: usize,
    arity_id: u64,
    referenced: &HashSet<(Arc<str>, Arc<str>)>,
) -> Vec<ExternalDefn> {
    if referenced.is_empty() {
        return Vec::new();
    }
    let registry = REGISTRY.read().unwrap();
    let mut out = Vec::new();
    let mut deps = DEPENDENTS.write().unwrap();
    for (ns, name) in referenced {
        let Some(defn) = registry.get(&(globals_id, ns.clone(), name.clone())) else {
            continue;
        };
        deps.entry((ns.clone(), name.clone()))
            .or_default()
            .insert(arity_id);
        out.push(ExternalDefn {
            ns: ns.clone(),
            name: name.clone(),
            arity_fn_names: defn.arities.iter().map(|a| a.fn_name.clone()).collect(),
            param_counts: defn.arities.iter().map(|a| a.param_count).collect(),
            is_variadic: defn.arities.iter().map(|a| a.is_variadic).collect(),
            arity_irs: defn.arities.iter().map(|a| a.ir.clone()).collect(),
        });
    }
    out
}

/// A var holding `(ns, name)` was rebound: drop every registration of it (all
/// isolates) and drain its dependents, marking each for lazy re-lowering.
/// Returns the dependent arity ids so the caller can also stale their
/// published native code.
pub fn on_redefined(ns: &str, name: &str) -> Vec<u64> {
    REGISTRY
        .write()
        .unwrap()
        .retain(|(_, k_ns, k_name), _| !(k_ns.as_ref() == ns && k_name.as_ref() == name));

    let drained = DEPENDENTS
        .write()
        .unwrap()
        .remove(&(Arc::from(ns), Arc::from(name)));
    let Some(deps) = drained else {
        return Vec::new();
    };
    if !deps.is_empty() {
        let mut relower = RELOWER.write().unwrap();
        for &id in &deps {
            if relower.insert(id) {
                RELOWER_PENDING.fetch_add(1, Ordering::Release);
            }
        }
    }
    deps.into_iter().collect()
}

/// Fast-path check: is any arity awaiting re-lowering?
#[inline]
pub fn relower_pending() -> bool {
    RELOWER_PENDING.load(Ordering::Acquire) != 0
}

/// Peek the re-lower mark for `arity_id` without consuming it.
///
/// The dispatch seam uses this to decide whether to enqueue a background
/// re-lower request; only the lowering worker consumes marks
/// ([`take_relower`]), so a mark can never be lost between the worker
/// publishing IR and validating it against concurrent rebinds.
pub fn relower_marked(arity_id: u64) -> bool {
    RELOWER.read().unwrap().contains(&arity_id)
}

/// Take the re-lower mark for `arity_id`, if set.
pub fn take_relower(arity_id: u64) -> bool {
    let mut relower = RELOWER.write().unwrap();
    if relower.remove(&arity_id) {
        RELOWER_PENDING.fetch_sub(1, Ordering::Release);
        true
    } else {
        false
    }
}

// ── Var-rebind hook ──────────────────────────────────────────────────────────

/// Install the invalidation hook into `cljrs-value`'s var-rebind notification
/// (idempotent).  Called from `eager_lower_fn` the first time a defn is
/// registered, so plain (non-JIT) eager-lowering sessions are also covered.
pub fn install_invalidation_hook() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        cljrs_value::set_var_rebind_hook(|old, _new| {
            let cljrs_value::Value::Fn(f) = old else {
                return;
            };
            let f = f.get();
            let Some(name) = f.name.as_deref() else {
                return;
            };
            for dep in on_redefined(&f.defining_ns, name) {
                // Remove the stale cached IR; `try_ir_path` re-lowers on the
                // dependent's next dispatch (take_relower).
                crate::ir_cache::invalidate(dep);
                // Null any published native pointer and route the backing
                // epochs to the JIT's code cache for reclamation.
                crate::jit_state::stale_native_code(dep);
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_ir() -> Arc<IrFunction> {
        Arc::new(IrFunction::new(None, None))
    }

    #[test]
    fn snapshot_externals_records_edges_drained_by_rebind() {
        // Unique globals_id / names so parallel tests sharing the global
        // registry never collide.
        let gid = 0xD500_0001usize;
        let ns: Arc<str> = Arc::from("test.snapshot-ns");
        let name: Arc<str> = Arc::from("callee-fn");
        let dep_id = 0xD500_0002u64;

        register_defn(gid, &ns, &name, vec![(1, false, dummy_ir())]);

        let mut referenced = HashSet::new();
        referenced.insert((ns.clone(), name.clone()));
        // An unregistered reference must not create an edge.
        referenced.insert((ns.clone(), Arc::from("never-registered")));

        let externals = snapshot_externals(gid, dep_id, &referenced);
        assert_eq!(externals.len(), 1);
        assert_eq!(externals[0].name, name);

        // The dependent edge was recorded atomically with the snapshot:
        // a rebind drains it and marks the dependent for re-lowering.
        assert!(!relower_marked(dep_id));
        let deps = on_redefined(&ns, &name);
        assert!(deps.contains(&dep_id));
        assert!(relower_marked(dep_id));

        // Peeking does not consume; taking does.
        assert!(relower_marked(dep_id));
        assert!(take_relower(dep_id));
        assert!(!relower_marked(dep_id));
        assert!(!take_relower(dep_id));

        // The registration itself is gone.
        let externals = snapshot_externals(gid, dep_id, &referenced);
        assert!(externals.is_empty());
    }
}
