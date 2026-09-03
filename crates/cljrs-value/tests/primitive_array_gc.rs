#![cfg(not(feature = "no-gc"))]

use std::sync::Mutex;

use cljrs_gc::{GcHeap, GcVisitor as _};
use cljrs_value::{PersistentVector, Value};

#[test]
fn primitive_arrays_reachable_through_a_collection_are_traced() {
    let _frame = cljrs_gc::push_alloc_frame();
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

    // This sibling allocation is not stored in the vector. It must be swept,
    // proving that the collections below reached the free path even if the GC
    // grace period changes.
    let _unreachable = Value::DoubleArray(heap.alloc(Mutex::new(vec![7.0])));

    assert_eq!(heap.count(), 10);

    for _ in 0..2 {
        heap.collect(|visitor| visitor.visit(&root));
    }

    assert_eq!(
        heap.count(),
        9,
        "the rooted vector and all eight primitive-array boxes must survive, \
         while the unreachable array box must not"
    );

    let vector = root.get();
    assert_eq!(vector.count(), 8);
    match (
        vector.nth(0),
        vector.nth(1),
        vector.nth(2),
        vector.nth(3),
        vector.nth(4),
        vector.nth(5),
        vector.nth(6),
        vector.nth(7),
    ) {
        (
            Some(Value::BooleanArray(boolean)),
            Some(Value::ByteArray(byte)),
            Some(Value::ShortArray(short)),
            Some(Value::IntArray(int)),
            Some(Value::LongArray(long)),
            Some(Value::FloatArray(float)),
            Some(Value::DoubleArray(double)),
            Some(Value::CharArray(character)),
        ) => {
            assert_eq!(*boolean.get().lock().unwrap(), vec![true]);
            assert_eq!(*byte.get().lock().unwrap(), vec![1]);
            assert_eq!(*short.get().lock().unwrap(), vec![2]);
            assert_eq!(*int.get().lock().unwrap(), vec![3]);
            assert_eq!(*long.get().lock().unwrap(), vec![4]);
            assert_eq!(*float.get().lock().unwrap(), vec![5.0]);
            assert_eq!(*double.get().lock().unwrap(), vec![6.0]);
            assert_eq!(*character.get().lock().unwrap(), vec!['x']);
        }
        other => panic!("unexpected primitive-array values: {other:?}"),
    }
}
