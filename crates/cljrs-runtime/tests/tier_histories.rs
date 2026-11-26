//! Execution-history differential tests (issue #510): each history from
//! `history/mod.rs` replayed under tree-walk, background IR, and the default
//! tiered path.  Eager lowering and native JIT run the same histories from
//! their own binaries (`tier_histories_eager.rs`, and
//! `cljrs-compiler/tests/tier_histories_jit.rs`), because each needs a
//! process-wide switch.

#![cfg(not(target_arch = "wasm32"))]

mod history;

use history::*;

const CONFIGS: &[Config] = &[TREE_WALK, IR_BACKGROUND, TIERED_DEFAULT];

macro_rules! histories {
    ($($test:ident => $history:ident),* $(,)?) => {$(
        #[test]
        fn $test() {
            init_thresholds();
            run($history, CONFIGS);
        }
    )*};
}

histories! {
    retained_old_warmed_after_rebind => RETAINED_OLD_WARMED_AFTER_REBIND,
    retained_old_warmed_after_consumer => RETAINED_OLD_WARMED_AFTER_CONSUMER,
    rebind_between_snapshot_and_publish => REBIND_BETWEEN_SNAPSHOT_AND_PUBLISH,
    rebind_while_queued => REBIND_WHILE_QUEUED,
    rebind_after_dequeue => REBIND_AFTER_DEQUEUE,
    rebind_between_publish_and_register => REBIND_BETWEEN_PUBLISH_AND_REGISTER,
    nested_region_bottom_up => NESTED_REGION_BOTTOM_UP,
    nested_region_top_down => NESTED_REGION_TOP_DOWN,
    rebind_leaf_of_optimized_chain => REBIND_LEAF_OF_OPTIMIZED_CHAIN,
    effects_counted_across_tiers => EFFECTS_COUNTED_ACROSS_TIERS,
    rebind_changes_callee_shape => REBIND_CHANGES_CALLEE_SHAPE,
    retained_in_collection => RETAINED_IN_COLLECTION,
}

/// Every history in the matrix has a test above.
#[test]
fn matrix_is_covered() {
    assert_eq!(
        ALL.len(),
        12,
        "add a `histories!` entry for the new history"
    );
}
