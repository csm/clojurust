// ── Arity ID generation ─────────────────────────────────────────────────────

use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ARITY_ID: AtomicU64 = AtomicU64::new(1);

/// Allocate a fresh unique arity ID.
pub fn fresh_arity_id() -> u64 {
    NEXT_ARITY_ID.fetch_add(1, Ordering::Relaxed)
}

/// The next arity ID that will be allocated.  Every previously allocated ID
/// is strictly below this value.  Used to snapshot a watermark separating
/// bootstrap-era definitions from later (user) ones — see
/// `cljrs_eval::jit_state::set_bootstrap_arity_watermark`.
pub fn next_arity_id() -> u64 {
    NEXT_ARITY_ID.load(Ordering::Relaxed)
}
