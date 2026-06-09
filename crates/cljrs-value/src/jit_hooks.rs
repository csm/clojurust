//! Cross-layer hooks the JIT installs into the value layer.
//!
//! The JIT tier (`cljrs-jit`) needs to learn when a var is rebound so it can
//! mark the previous definition's compiled native code *stale* and reclaim it
//! at the next stop-the-world safepoint (Phase 10.2 — code unloading).
//!
//! `cljrs-value` sits far below `cljrs-jit` in the dependency graph, so the
//! coupling is inverted through a function-pointer hook: the JIT installs a
//! callback via [`set_var_rebind_hook`], and [`Var::bind`](crate::types::Var::bind)
//! invokes it (through [`notify_var_rebind`]) whenever it overwrites an existing
//! binding.  When no JIT is active the hook is unset and the cost is a single
//! relaxed atomic load.

use std::sync::OnceLock;

use crate::Value;

/// Callback signature: `(old_value, new_value)`.
///
/// Called with the value being replaced and the value replacing it, so the
/// implementation can stale only the definitions that are genuinely going away
/// (an arity present in both `old` and `new` — e.g. rebinding a var to the same
/// function object — must not be staled).
type VarRebindHook = Box<dyn Fn(&Value, &Value) + Send + Sync + 'static>;

static VAR_REBIND_HOOK: OnceLock<VarRebindHook> = OnceLock::new();

/// Install the var-rebind hook.  Idempotent: only the first call wins.
///
/// Installed once by `cljrs_jit::init`.
pub fn set_var_rebind_hook(f: impl Fn(&Value, &Value) + Send + Sync + 'static) {
    let _ = VAR_REBIND_HOOK.set(Box::new(f));
}

/// Notify the JIT that a var holding `old` is being rebound to `new`.
///
/// No-op (one atomic load) when no hook is installed.  Called from
/// `Var::bind` only when an existing value is being replaced.
#[inline]
pub(crate) fn notify_var_rebind(old: &Value, new: &Value) {
    if let Some(hook) = VAR_REBIND_HOOK.get() {
        hook(old, new);
    }
}
