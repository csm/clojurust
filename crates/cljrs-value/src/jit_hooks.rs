//! Cross-layer hooks upper tiers install into the value layer.
//!
//! Two consumers need to learn when a var is rebound:
//! - the JIT tier (`cljrs-jit`) marks the previous definition's compiled
//!   native code *stale* and reclaims it at the next stop-the-world safepoint
//!   (Phase 10.2 — code unloading);
//! - the IR tier (`cljrs-eval`) invalidates cached lowerings of *other*
//!   functions that specialized against the old definition (Phase 10.5 —
//!   cross-defn region promotion).
//!
//! `cljrs-value` sits far below both in the dependency graph, so the coupling
//! is inverted through callbacks: each consumer registers one via
//! [`set_var_rebind_hook`], and [`Var::bind`](crate::types::Var::bind) invokes
//! all of them (through [`notify_var_rebind`]) whenever it overwrites an
//! existing binding.  When no hook is registered the cost is one atomic flag
//! load.

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::Value;

/// Callback signature: `(old_value, new_value)`.
///
/// Called with the value being replaced and the value replacing it, so the
/// implementation can stale only the definitions that are genuinely going away
/// (an arity present in both `old` and `new` — e.g. rebinding a var to the same
/// function object — must not be staled).
type VarRebindHook = Box<dyn Fn(&Value, &Value) + Send + Sync + 'static>;

static VAR_REBIND_HOOKS: RwLock<Vec<VarRebindHook>> = RwLock::new(Vec::new());
static ANY_HOOK: AtomicBool = AtomicBool::new(false);

/// Register a var-rebind hook.  Multiple hooks may be registered (the JIT's
/// code unloading and the IR tier's cross-defn invalidation); each is called
/// on every rebind, in registration order.
pub fn set_var_rebind_hook(f: impl Fn(&Value, &Value) + Send + Sync + 'static) {
    VAR_REBIND_HOOKS.write().unwrap().push(Box::new(f));
    ANY_HOOK.store(true, Ordering::Release);
}

/// Notify the registered hooks that a var holding `old` is being rebound to
/// `new`.
///
/// No-op (one atomic load) when no hook is installed.  Called from
/// `Var::bind` only when an existing value is being replaced.
#[inline]
pub(crate) fn notify_var_rebind(old: &Value, new: &Value) {
    if !ANY_HOOK.load(Ordering::Acquire) {
        return;
    }
    for hook in VAR_REBIND_HOOKS.read().unwrap().iter() {
        hook(old, new);
    }
}
