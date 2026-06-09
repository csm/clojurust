//! In-process JIT compiler for clojurust — Phase 10.1.
//!
//! ## How it fits into the execution tiers
//!
//! ```text
//! call_cljrs_fn (cljrs-eval/src/apply.rs)
//!     ↓ JIT-native   ← this crate publishes compiled function pointers
//!     ↓ Tier-1 IR    ← invocation counter bumped here; enqueue when hot
//!     ↓ Tree-walk    ← universal fallback
//! ```
//!
//! ## Usage
//!
//! Call [`init`] once at process startup (before any Clojure code runs):
//!
//! ```rust,ignore
//! cljrs_jit::init();
//! ```
//!
//! This:
//! 1. Forces eager IR lowering on (so functions get IR as they are defined).
//! 2. Installs an enqueue hook in `cljrs_eval::jit_state`.
//! 3. Spawns the background JIT worker thread.
//!
//! Hot functions (those whose Tier-1 call count exceeds
//! `CLJRS_JIT_THRESHOLD`, default 1000) are compiled in the background;
//! subsequent calls dispatch directly to native code.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use cljrs_ir::IrFunction;

mod jit_compiler;
mod jit_worker;

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Initialise the JIT tier.
///
/// Idempotent: safe to call multiple times, only initialises once.
///
/// Sets the JIT threshold to `CLJRS_JIT_THRESHOLD` (env) or 1000 (default).
/// Override the threshold before calling this with
/// [`cljrs_eval::jit_state::set_jit_threshold`].
pub fn init() {
    if INITIALIZED.swap(true, Ordering::AcqRel) {
        return;
    }

    // Ensure IR is generated for newly-defined functions.
    cljrs_eval::force_eager_lowering();

    let (tx, rx) = mpsc::sync_channel::<jit_worker::CompileRequest>(256);

    // Register the enqueue hook so the IR dispatch path can hand us hot
    // functions.
    cljrs_eval::set_enqueue_hook(move |arity_id, ir_func: Arc<IrFunction>| {
        // Non-blocking: if the queue is full, skip this compile request.
        // The function will keep running at Tier 1 until the queue drains.
        let _ = tx.try_send(jit_worker::CompileRequest { arity_id, ir_func });
    });

    // Spawn the background compilation thread.
    jit_worker::start_worker(rx);
}
