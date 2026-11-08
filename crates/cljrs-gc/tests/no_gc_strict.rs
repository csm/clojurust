//! Strict governor admission for `no-gc` fallible allocation (Phase 4).
//!
//! This binary owns the process governor, so it has one test.
//!
//! Run with:
//!   cargo test -p cljrs-gc --features no-gc

#![cfg(feature = "no-gc")]

use cljrs_gc::GcPtr;
use cljrs_gc::alloc_ctx::ScratchGuard;
use cljrs_gc::governor::{self, MemoryConfig};

const MIB: usize = 1024 * 1024;

/// `try_new` and `try_static_alloc` ask the governor only when the region
/// or the static arena must add a chunk: memory already charged is used even
/// at the hard limit, and a new chunk past it is refused.
#[test]
fn fallible_no_gc_allocation_reserves_only_new_chunks() {
    let g = governor::governor();
    // Charge the static arena's first chunk and open a region chunk.
    let _warm = GcPtr::new(0i64);
    let scratch = ScratchGuard::new();
    let _in_region = GcPtr::new(0i64);

    // No headroom at all: only memory already charged can be used.
    let committed = g.committed_bytes();
    let mut config = MemoryConfig::with_limits(committed, committed).unwrap();
    config.strict = true;
    g.configure(config).unwrap();

    assert!(
        GcPtr::try_new(1i64).is_ok(),
        "fits in the region's charged chunk"
    );
    drop(scratch);
    assert!(
        GcPtr::try_new(2i64).is_ok(),
        "fits in the static arena's charged chunk"
    );
    assert!(cljrs_gc::try_static_alloc(3i64).is_ok());

    // Fill a fresh region chunk; the allocation that needs the next chunk
    // is refused and charges nothing.
    let _scratch = ScratchGuard::new();
    let mut refused = None;
    for _ in 0..100_000 {
        let before = g.committed_bytes();
        if let Err(e) = GcPtr::try_new(4i64) {
            assert_eq!(
                g.committed_bytes(),
                before,
                "a refused chunk is not charged"
            );
            refused = Some(e.error());
            break;
        }
    }
    let refused = refused.expect("a new region chunk past the limit is refused");
    assert_eq!(refused.class, governor::MemoryClass::GcHeap);
    assert!(governor::pending_runtime_fault().is_none());

    g.configure(MemoryConfig::with_limits(256 * MIB, 512 * MIB).unwrap())
        .unwrap();
}
