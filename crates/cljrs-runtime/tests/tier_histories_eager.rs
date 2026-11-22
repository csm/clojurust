//! The execution-history matrix (`history/mod.rs`, issue #510) under eager
//! lowering: every function is lowered when it is defined, so histories
//! exercise IR built against callees that were themselves lowered at
//! definition time rather than when they got warm.
//!
//! Eager lowering is a process-wide switch with no off position, so it lives
//! in this binary alone.  The pause ops are skipped here: nothing waits in
//! the background queue for them to intercept.

#![cfg(not(target_arch = "wasm32"))]

mod history;

use history::*;

#[test]
fn every_history_under_eager_lowering() {
    init_thresholds();
    cljrs_runtime::tiered::force_eager_lowering();
    for &h in ALL {
        run(h, &[TREE_WALK, IR_EAGER]);
    }
}
