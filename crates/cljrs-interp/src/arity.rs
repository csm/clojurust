// ── Arity ID generation ─────────────────────────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ARITY_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a fresh unique arity ID.
pub fn fresh_arity_id() -> u64 {
    NEXT_ARITY_ID.fetch_add(1, Ordering::Relaxed)
}
