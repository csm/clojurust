# cljrs-gc

Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust;
or, with the `no-gc` Cargo feature, a region-based allocator with no GC pauses.

**Phase:** 8.1 (GcVisitor + Trace infrastructure) + 8.2 (GcBox/GcHeap
raw-pointer implementation) — implemented.  `no-gc` mode (Phases 1–8 of
`docs/no-gc-plan.md`) — implemented.

---

## Purpose

Manages all Clojure runtime values.  `GcPtr<T>` is a raw pointer into either
the GC heap or a bump-allocated region; `clone` is O(1); `drop` is a no-op.

**Default build (GC mode):** memory is freed only during `GcHeap::collect`.

**`no-gc` build:** every function call and every `loop` iteration pushes a
scratch `Region`; intermediates are freed when the scope exits.  Return values
and `recur` arguments are evaluated in the caller's context (the
"return-expression-in-caller" mechanism).  Static-sink expressions (`def`,
`defn`, `defmacro`, `atom`, `agent`, `volatile!`, `reset!`, `vreset!`,
`swap!`, `vswap!`, `alter-var-root`, `intern`) go to the global `StaticArena`
and live for the program lifetime.
No `GcHeap`, no stop-the-world pauses, no `Trace` overhead at runtime.

**Phase 7 (debug provenance):** in `debug_assertions` builds with `no-gc`,
`StaticArena` tracks chunk ranges and exposes `is_static_addr(usize) -> bool`.
`GcPtr::is_static_alloc()` uses this to check pointer provenance at O(chunks)
cost.  `Atom::reset`, `Var::bind`, and `Volatile::reset` use `debug_assert!`
to catch region-local values being stored in program-lifetime containers.

---

## File layout

```
src/
  lib.rs          — GcVisitor, Trace, GcBox<T>, GcPtr<T>, MarkVisitor, HEAP,
                    leaf Trace impls; conditional GC vs no-gc implementations
  gc_header       — (GC mode only) GcBoxHeader, drop/trace fns
  gc_full         — (GC mode only) GcHeap, ALLOC_ROOTS, AllocRootGuard
  nogc_stubs      — (no-gc mode) stub GcHeap, GcConfig, cancellation stubs
  static_arena.rs — (no-gc mode) global program-lifetime bump allocator;
                    in debug builds, tracks chunk ranges for is_static_addr()
  alloc_ctx.rs    — (no-gc mode) thread-local allocation context stack;
                    ScratchGuard, StaticCtxGuard
  region.rs       — Region bump allocator, RegionGuard, thread-local region stack
  cancellation.rs — (GC mode) STW coordination, MutatorGuard, safepoints
  config.rs       — (GC mode) GcConfig, GcCancellation, GcParked
  stats.rs        — process-global GcStats counters: GC allocations,
                    region (bump) allocations, GC pauses + freed bytes/objects
tests/
  no_gc_alloc.rs  — (no-gc mode) integration tests for the allocation context stack:
                    ScratchGuard, StaticCtxGuard, pop_for_return protocol,
                    nested guards, destructor ordering
```

---

## Public API

### `GcVisitor`

```rust
pub trait GcVisitor {
    fn visit<T: Trace + 'static>(&mut self, ptr: &GcPtr<T>);
}
```

Implemented by [`MarkVisitor`].  Call `visitor.visit(ptr)` inside
`Trace::trace` for every `GcPtr` field.

### `Trace`

```rust
pub trait Trace: Send + Sync {
    fn trace(&self, visitor: &mut MarkVisitor);
}
```

Implemented by every type stored behind a `GcPtr`.  Must call
`visitor.visit(ptr)` for every `GcPtr` reachable from `self` (directly or
through `Arc`/`Mutex`/etc.).

Built-in leaf impls: `String`, `i64`, `f64`, `bool`,
`num_bigint::BigInt`, `bigdecimal::BigDecimal`,
`num_rational::Ratio<BigInt>`.

### `GcPtr<T: Trace + 'static>`

```rust
pub struct GcPtr<T: Trace + 'static>(NonNull<GcBox<T>>);

impl<T: Trace + 'static> GcPtr<T> {
    pub fn new(value: T) -> Self        // allocates on HEAP (or ctx in no-gc)
    pub fn get(&self) -> &T             // borrow; invalid after collect frees it
    pub fn ptr_eq(a: &Self, b: &Self) -> bool

    // no-gc + debug_assertions only:
    pub fn is_static_alloc(&self) -> bool  // true if allocated in StaticArena
}
impl<T: Trace + 'static> Clone for GcPtr<T> { /* O(1) raw-pointer copy */ }
impl<T: Trace + 'static> Drop  for GcPtr<T> { /* no-op */ }
```

### Free functions (no-gc only)

```rust
// debug_assertions only:
pub fn is_static_addr(addr: usize) -> bool;  // checks the StaticArena chunk registry
```

### `GcHeap`

```rust
pub struct GcHeap { /* Mutex<GcHeapInner> */ }

impl GcHeap {
    pub const fn new() -> Self
    pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T>
    pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, trace_roots: F)
    pub fn count(&self) -> usize
    pub fn total_allocated(&self) -> usize
    pub fn total_freed(&self) -> usize
}
```

`collect` is stop-the-world: must only be called when no other thread is
creating or dereferencing `GcPtr` values.

### `MarkVisitor`

```rust
pub struct MarkVisitor { /* grey stack */ }
impl GcVisitor for MarkVisitor { … }
```

Uses a grey stack (avoids recursion stack overflow) and handles cycles via
already-marked check.

### `HEAP`

```rust
pub static HEAP: GcHeap;
```

Global singleton; all `GcPtr::new` calls allocate here.

### `region::Region`

```rust
pub struct Region { /* chunks, bump pointer, drop registry */ }

impl Region {
    pub fn new() -> Self
    pub fn with_capacity(cap: usize) -> Self
    pub fn alloc<T: Trace + 'static>(&mut self, value: T) -> GcPtr<T>
    pub fn reset(&mut self)
    pub fn bytes_used(&self) -> usize
    pub fn object_count(&self) -> usize
}
```

Bump allocator for short-lived objects. ~2.6x faster than `GcHeap::alloc`
(no mutex, no `Box::new`). Objects are NOT in the GC heap linked list.
Destructors run on `reset()` or `drop`.

### `region::RegionGuard`

RAII guard that pushes a `Region` onto the thread-local stack. Use with
`try_alloc_in_region()` for opportunistic region allocation.

### `stats::GcStats` and `GC_STATS`

```rust
pub struct GcStats { /* AtomicU64 counters */ }

impl GcStats {
    pub const fn new() -> Self
    pub fn record_gc_alloc(&self, bytes: usize)
    pub fn record_region_alloc(&self, bytes: usize)
    pub fn record_gc_pause(&self, pause: Duration, freed_objects: u64, freed_bytes: u64)
    pub fn snapshot(&self) -> GcStatsSnapshot
}

pub struct GcStatsSnapshot { /* immutable view of counters */ }
impl GcStatsSnapshot { pub fn total_pause(&self) -> Duration }
impl std::fmt::Display for GcStatsSnapshot { /* multi-line summary */ }

pub static GC_STATS: GcStats;

pub const CLJRS_GC_STATS_ENV: &str;       // = "CLJRS_GC_STATS"
pub fn dump_stats_from_env();
```

Process-global counters updated automatically by `GcHeap::alloc`,
`GcHeap::collect`, and `Region::alloc`.  The `cljrs --gc-stats [FILE]` CLI
flag prints a snapshot of these counters at program exit.

`dump_stats_from_env()` is the AOT-binary equivalent: it reads the
`CLJRS_GC_STATS` environment variable and, if set, writes a snapshot to
stdout (when the value is empty or `"-"`) or to the named file.  AOT-compiled
programs and the AOT test harness call it once at exit.

---

## Design notes

- **Non-moving**: `GcPtr<T>` stores a stable `NonNull<GcBox<T>>` address.
- **Stop-the-world**: `collect` must pause all other threads that hold `GcPtr`s.
- **Intrusive linked list**: all `GcBox`es are linked via `GcBoxHeader::next`.
- **Type erasure**: `trace_fn` / `drop_fn` in the header enable type-erased
  mark and sweep without a vtable pointer per allocation.
- **Cycle collection**: because `GcPtr::drop` is a no-op, reference cycles do
  not prevent collection — any object unreachable from roots is freed.

---

## Deferred to later phases

- Incremental/concurrent collection — Phase 10+
- Write barriers for generational GC — Phase 10+
- Weak references (`WeakGcPtr<T>`) — deferred
- Safepoint integration with JIT frames — Phase 10+
- Automatic collection trigger (threshold-based) — deferred
