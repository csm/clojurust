//! Regression test: interned scalar constants must not be GC-managed.
//!
//! `rt_const_nil` / `rt_const_true` / `rt_const_long` hand compiled code raw
//! pointers into process-lifetime intern caches.  These caches were once
//! allocated on the GC heap with no root tracing them: once the allocating
//! frame's alloc-root entries popped, two collections exhausted the lives
//! grace period and swept the boxes, after which every compiled use of an
//! interned pointer read freed memory.  Symptom: intermittent segfaults in
//! JIT/AOT code under allocation pressure (e.g. `samples/graph.cljrs` with
//! `--ir-threshold 5 --jit-threshold 10` and a low `--gc-soft-limit-mb`).
//!
//! The caches now use `cljrs_gc::static_alloc` (program-lifetime, never
//! collected), so interning must leave the GC heap untouched and the
//! pointers must stay valid across unrooted collections.
//!
//! NOTE: this test relies on being the only test in this binary — the heap
//! is thread-local and the intern caches are process-wide, so the counts
//! below are only meaningful on the thread that initializes the caches.

use cljrs_value::Value;

#[test]
fn interned_scalars_are_not_gc_managed() {
    let heap_before = cljrs_gc::HEAP.count();

    let nil_p = cljrs_compiler::rt_abi::rt_const_nil();
    let true_p = cljrs_compiler::rt_abi::rt_const_true();
    let false_p = cljrs_compiler::rt_abi::rt_const_false();
    let long_p = cljrs_compiler::rt_abi::rt_const_long(42);

    // The intern caches are never traced, so they must not be GC-heap
    // allocations.  (Pre-fix this was 1025 boxes — nil, true/false, and the
    // 1024-entry long cache — all of which the collector eventually swept.)
    assert_eq!(
        cljrs_gc::HEAP.count(),
        heap_before,
        "interning scalar constants must not allocate on the GC heap"
    );

    // Two collections with no roots traced: anything GC-managed that nothing
    // marks is freed (the lives grace period covers exactly one cycle).
    cljrs_gc::HEAP.collect(|_| {});
    cljrs_gc::HEAP.collect(|_| {});

    // SAFETY: interned constant pointers are documented process-lifetime.
    unsafe {
        assert!(matches!(&*nil_p, Value::Nil), "interned nil was collected");
        assert!(
            matches!(&*true_p, Value::Bool(true)),
            "interned true was collected"
        );
        assert!(
            matches!(&*false_p, Value::Bool(false)),
            "interned false was collected"
        );
        assert!(
            matches!(&*long_p, Value::Long(42)),
            "interned long was collected"
        );
    }

    // Interning must still be stable (same pointer on every call).
    assert_eq!(nil_p, cljrs_compiler::rt_abi::rt_const_nil());
    assert_eq!(long_p, cljrs_compiler::rt_abi::rt_const_long(42));
}
