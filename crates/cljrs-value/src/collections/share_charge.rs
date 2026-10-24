//! Allocation charge for structurally shared persistent collections.
//!
//! An update such as `conj` or `assoc` path-copies O(log n) nodes and shares
//! everything else with the version it was derived from. Charging each new
//! version for the whole collection would make a loop of `n` updates account
//! O(n²) bytes, while the loop allocates only O(n log n) and frees almost all
//! of it with each dead intermediate. [`ShareCharge`] decides what a version
//! reports from `Trace::gc_size_extra`:
//!
//! - A version built wholesale (`empty`, `from_iter`, a clone, an in-place
//!   `*_mut` update, or one derived from a parent that was never
//!   GC-allocated, such as a Rust-side builder loop) is charged its full
//!   estimated size.
//! - A version derived by one update from a GC-allocated parent is charged
//!   only its path-copied nodes, because the parent's box already accounts
//!   for the shared remainder.
//! - A derived version whose count reaches twice the count of its nearest
//!   fully charged ancestor is charged in full again. A chain of `n` updates
//!   therefore charges O(n) element bytes in total plus O(log n) node bytes
//!   per step, and a collection cannot grow past twice the size the governor
//!   last admitted in full.
//!
//! This is an approximation. When the fully charged ancestor is collected, a
//! surviving derived version is under-counted by less than twice that
//! ancestor's full charge. Sweep is unaffected: it returns the size stored in
//! each box header at allocation time.

use std::sync::atomic::{AtomicBool, Ordering};

/// Per-version charge state embedded in a persistent collection.
///
/// `Clone` yields a full charge: a clone shares every node with its source,
/// and becomes their sole owner if the source is collected.
#[derive(Debug)]
pub(crate) struct ShareCharge {
    /// Bytes charged for a derived version; 0 means "charge in full".
    path_bytes: u32,
    /// Count of the nearest fully charged ancestor (derived versions only).
    base_count: usize,
    /// Set when this version is allocated behind a `GcPtr`.
    allocated: AtomicBool,
}

impl ShareCharge {
    /// A version that is charged its full size.
    pub(crate) fn full() -> Self {
        Self {
            path_bytes: 0,
            base_count: 0,
            allocated: AtomicBool::new(false),
        }
    }

    /// Charge for a version of `count` elements derived by one update from
    /// the version that owns `self`, which has `parent_count` elements.
    /// `path_bytes` estimates the nodes the update copied.
    pub(crate) fn derive(&self, parent_count: usize, count: usize, path_bytes: usize) -> Self {
        if !self.allocated.load(Ordering::Relaxed) {
            return Self::full();
        }
        let base_count = if self.path_bytes == 0 {
            parent_count
        } else {
            self.base_count
        };
        if count >= base_count.saturating_mul(2) {
            return Self::full();
        }
        Self {
            path_bytes: u32::try_from(path_bytes.max(1)).unwrap_or(u32::MAX),
            base_count,
            allocated: AtomicBool::new(false),
        }
    }

    /// The bytes to charge, given the version's full estimated size.
    pub(crate) fn size_extra(&self, full_bytes: usize) -> usize {
        if self.path_bytes == 0 {
            full_bytes
        } else {
            self.path_bytes as usize
        }
    }

    /// [`Self::size_extra`] for `Trace::gc_size_extra`, which the allocator
    /// calls once per allocation. Records that this version now lives behind
    /// a `GcPtr`, so versions derived from it are charged incrementally.
    pub(crate) fn on_alloc(&self, full_bytes: usize) -> usize {
        self.allocated.store(true, Ordering::Relaxed);
        self.size_extra(full_bytes)
    }
}

impl Clone for ShareCharge {
    fn clone(&self) -> Self {
        Self::full()
    }
}

/// Bytes for one `Arc` allocation's strong and weak counts.
const ARC_HEADER: usize = 16;

/// Bytes path-copied by one update of an `rpds::Vector` (32-way trie, no
/// tail) of `len` elements: one full node per level plus the new element.
pub(crate) fn vector_path_bytes<T>(len: usize) -> usize {
    // Arc + Node enum (Vec + tag) + a full 32-slot pointer buffer.
    const NODE: usize = ARC_HEADER + 32 + 32 * 8;
    let mut levels = 1;
    let mut capacity = 32usize;
    while capacity < len {
        capacity = capacity.saturating_mul(32);
        levels += 1;
    }
    levels * NODE + ARC_HEADER + std::mem::size_of::<T>()
}

/// Bytes path-copied by one insert or remove in an `rpds::HashTrieMap`
/// (64-way HAMT) of `len` entries whose entries are `E`.
pub(crate) fn hamt_path_bytes<E>(len: usize) -> usize {
    // Arc + Node enum (bitmap + Vec + tag) + a full 64-slot pointer buffer.
    const NODE: usize = ARC_HEADER + 40 + 64 * 8;
    let mut levels = 1;
    let mut capacity = 64usize;
    while capacity < len {
        capacity = capacity.saturating_mul(64);
        levels += 1;
    }
    // Root-to-leaf branches plus the leaf node, plus the new entry.
    (levels + 1) * NODE + ARC_HEADER + std::mem::size_of::<E>()
}

/// Bytes path-copied by one insert or remove in an `rpds::RedBlackTreeMap`
/// of `len` entries whose entries are `E`.
pub(crate) fn rb_path_bytes<E>(len: usize) -> usize {
    // Arc + entry pointer, colour, and two child pointers.
    const NODE: usize = ARC_HEADER + 32;
    // Height is at most 2·log2(n + 1); rebalancing copies a few siblings.
    let log2 = (usize::BITS - len.saturating_add(1).leading_zeros()) as usize;
    (2 * log2 + 3) * NODE + ARC_HEADER + std::mem::size_of::<E>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unallocated_parent_yields_full_charge() {
        let parent = ShareCharge::full();
        let child = parent.derive(100, 101, 500);
        assert_eq!(child.size_extra(10_000), 10_000);
    }

    #[test]
    fn allocated_parent_yields_path_charge() {
        let parent = ShareCharge::full();
        assert_eq!(parent.on_alloc(10_000), 10_000);
        let child = parent.derive(100, 101, 500);
        assert_eq!(child.on_alloc(10_100), 500);
        // The grandchild inherits the base count of 100.
        let grandchild = child.derive(101, 102, 500);
        assert_eq!(grandchild.size_extra(10_200), 500);
    }

    #[test]
    fn doubling_the_base_count_recharges_in_full() {
        let mut v = ShareCharge::full();
        v.on_alloc(0);
        let mut full_at = Vec::new();
        for count in 11..=100 {
            let next = v.derive(count - 1, count, 1);
            if next.on_alloc(count * 1000) == count * 1000 {
                full_at.push(count);
            }
            v = next;
        }
        assert_eq!(full_at, vec![20, 40, 80]);
    }

    #[test]
    fn clone_is_charged_in_full() {
        let parent = ShareCharge::full();
        parent.on_alloc(0);
        let child = parent.derive(100, 101, 500);
        assert_eq!(child.clone().size_extra(10_100), 10_100);
    }
}
