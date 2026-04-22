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

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use cljrs_ir::IrFunction;

// ── Cache entries ────────────────────────────────────────────────────────────

/// State of an IR cache entry for one function arity.
pub enum IrCacheEntry {
    /// Lowering has not been attempted yet.
    NotAttempted,
    /// Lowering was attempted but failed (unsupported form); don't retry.
    Unsupported,
    /// Successfully lowered IR function.
    Cached(Arc<IrFunction>),
}

// ── Global cache ─────────────────────────────────────────────────────────────

static IR_CACHE: RwLock<Option<HashMap<u64, IrCacheEntry>>> = RwLock::new(None);

/// Look up a cached IR function by arity ID.
/// Returns `None` if not cached or if lowering previously failed.
/// Returns `Some(ir)` if cached.
///
/// This is the hot path — uses a read lock so concurrent callers don't block.
pub fn get_cached(id: u64) -> Option<Arc<IrFunction>> {
    let guard = IR_CACHE.read().unwrap();
    let cache = guard.as_ref()?;
    match cache.get(&id) {
        Some(IrCacheEntry::Cached(ir)) => Some(ir.clone()),
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
    cache.insert(id, IrCacheEntry::Cached(ir));
}

/// Mark an arity as unsupported (lowering failed; don't retry).
pub fn store_unsupported(id: u64) {
    let mut guard = IR_CACHE.write().unwrap();
    let cache = guard.get_or_insert_with(HashMap::new);
    cache.insert(id, IrCacheEntry::Unsupported);
}
