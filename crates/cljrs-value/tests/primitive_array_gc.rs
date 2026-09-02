#![cfg(not(feature = "no-gc"))]

use std::sync::Mutex;

use cljrs_gc::{GcHeap, GcVisitor as _};
use cljrs_value::{PersistentVector, Value};

#[test]
fn primitive_arrays_reachable_through_a_collection_are_traced() {
    let heap = GcHeap::new();
    let arrays = [
        Value::BooleanArray(heap.alloc(Mutex::new(vec![true]))),
        Value::ByteArray(heap.alloc(Mutex::new(vec![1]))),
        Value::ShortArray(heap.alloc(Mutex::new(vec![2]))),
        Value::IntArray(heap.alloc(Mutex::new(vec![3]))),
        Value::LongArray(heap.alloc(Mutex::new(vec![4]))),
        Value::FloatArray(heap.alloc(Mutex::new(vec![5.0]))),
        Value::DoubleArray(heap.alloc(Mutex::new(vec![6.0]))),
        Value::CharArray(heap.alloc(Mutex::new(vec!['x']))),
    ];
    let root = heap.alloc(PersistentVector::from_iter(arrays));

    assert_eq!(heap.count(), 9);

    // Unreachable allocations have one collection cycle of grace. Keeping the
    // vector rooted across two cycles must also keep all array boxes reachable
    // from its Values.
    for _ in 0..2 {
        heap.collect(|visitor| visitor.visit(&root));
    }

    assert_eq!(
        heap.count(),
        9,
        "the rooted vector and all eight primitive-array boxes must survive"
    );
}
