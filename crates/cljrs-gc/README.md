# cljrs-gc

Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust;
or, with the `no-gc` Cargo feature, a region-based allocator with no GC pauses.

On `wasm32` targets the `system-memory` crate is excluded (it brings in `errno`,
which does not build for `wasm32-unknown-unknown`).  The GC heap defaults to a
fixed **64 MB** soft limit instead of consulting total system RAM.

**Phase:** 8.1 (GcVisitor + Trace infrastructure) + 8.2 (GcBox/GcHeap
raw-pointer implementation) — implemented.  `no-gc` mode (Phases 1–8 of
`docs/no-gc-plan.md`) — implemented.  B3 (`StaticGcPtr`, `static_alloc`) —
implemented.

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
  gc_full         — (GC mode only) GcHeap, HeapProxy, HEAP (per-isolate proxy),
                    ALLOC_ROOTS, AllocRootGuard
  nogc_stubs      — (no-gc mode) stub GcHeap, GcConfig, cancellation stubs
  static_arena.rs — (no-gc mode) global program-lifetime bump allocator;
                    in debug builds, tracks chunk ranges for is_static_addr()
  alloc_ctx.rs    — (no-gc mode) thread-local allocation context stack;
                    ScratchGuard, StaticCtxGuard
  region.rs       — Region bump allocator, RegionGuard, thread-local region stack
  cancellation.rs — (GC mode) STW coordination, MutatorGuard, safepoints
  config.rs       — (GC mode) GcConfig, GcCancellation (zero-sized proxy),
                    IsolateCancellation thread-local (per-isolate STW state), GcParked
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

    // Default impl returns 0; override for types with significant inline-owned heap.
    fn gc_size_extra(&self) -> usize { 0 }
}
```

Implemented by every type stored behind a `GcPtr`.  Must call
`visitor.visit(ptr)` for every `GcPtr` reachable from `self` (directly or
through `Arc`/`Mutex`/etc.).

`gc_size_extra` returns heap bytes owned by the value that are NOT counted by
`size_of::<GcBox<T>>()` — Vec buffers, String capacity, Form AST trees stored
inline.  The GC adds this to the tracked `memory_in_use` so collection fires at
the right threshold.  Do NOT cross `GcPtr` boundaries — pointed-to boxes are
counted separately when allocated.

Built-in leaf impls: `String` (overrides `gc_size_extra` to return `capacity()`),
`i64`, `f64`, `bool`, `num_bigint::BigInt`, `bigdecimal::BigDecimal`,
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

### `StaticGcPtr<T: 'static>` (always available — Phase B3)

Program-lifetime pointer safe to share across isolate threads.  Backed by the
global `StaticArena` (in `no-gc` builds) or `Box::leak` (in GC builds).
Unlike `GcPtr`, it wraps `*const T` directly (no `GcBox` header) and is
`Send + Sync`.

```rust
pub struct StaticGcPtr<T: 'static>(NonNull<T>);

impl<T: 'static> StaticGcPtr<T> {
    pub fn get(&self) -> &T
    pub fn ptr_eq(a: &Self, b: &Self) -> bool
}
impl<T: 'static> Clone for StaticGcPtr<T> { /* O(1) NonNull copy */ }

/// Allocate `value` as program-lifetime memory.
/// no-gc: StaticArena bump-alloc; GC: Box::leak.
pub fn static_alloc<T: 'static>(value: T) -> StaticGcPtr<T>;
```

### Free functions

```rust
// always:
pub fn static_alloc<T: 'static>(value: T) -> StaticGcPtr<T>;

// no-gc only:
pub fn static_arena() -> &'static StaticArena;

// no-gc + debug_assertions only:
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

### `HeapProxy` and `HEAP`

```rust
pub struct HeapProxy;   // zero-sized; all state in ISOLATE_HEAP thread-local

impl HeapProxy {
    pub fn alloc<T: Trace + 'static>(&self, value: T) -> GcPtr<T>
    pub fn set_config(&self, config: Arc<GcConfig>)
    pub fn set_config_from_env(&self)
    pub fn register_root_tracer(&self, tracer: impl Fn(&mut MarkVisitor) + 'static)
    pub fn trace_registered_roots(&self, visitor: &mut MarkVisitor)
    pub fn memory_in_use(&self) -> usize
    pub fn count(&self) -> usize
    pub fn total_allocated(&self) -> usize
    pub fn total_freed(&self) -> usize
    pub fn collect<F: FnOnce(&mut MarkVisitor)>(&self, trace_roots: F)
    pub fn collect_auto(&self) -> bool
}

pub static HEAP: HeapProxy;
```

`HEAP` is a zero-sized proxy that dispatches every operation to the calling
thread's `ISOLATE_HEAP` thread-local `GcHeap`. Each OS thread (isolate) owns
an independent heap; GC runs fully in parallel across threads with no
cross-isolate stop-the-world coordination. All `GcPtr::new` calls allocate
into the current thread's heap via this proxy.

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
- **Accurate allocation accounting**: `GcBoxHeader::size` stores
  `size_of::<GcBox<T>>() + value.gc_size_extra()` at allocation time.
  `memory_in_use` is incremented by this total (not a flat estimate) and
  decremented by the same value when the object is freed.  Types that own
  significant out-of-line heap (Form AST trees in `CljxFn`, String capacity)
  override `gc_size_extra` so the GC threshold fires before the process OOMs.
- **Fixed-headroom GC suppression**: after a zero-yield collection (nothing freed),
  GC is suppressed until `memory_in_use` grows by another `soft_limit/10` bytes
  (a fixed additive headroom, not a percentage of current memory).  Using a
  percentage of current memory as headroom would compound across consecutive
  zero-yield cycles (e.g. during deep recursion where all objects are live),
  causing the threshold to grow exponentially and GC to stop firing permanently
  after the computation finishes — leading to OOM on long test suites.  A fixed
  headroom gives linear growth, which stays bounded.  The old trigger —
  re-enabling on every alloc-frame drop — fired O(N-heap) sweeps on every
  eval-frame return, causing a GC storm with hundreds of useless traversals.
- **Minimal grace period** (`GC_INITIAL_LIVES = 2`): objects start at `lives = 1`.
  GC only fires at explicit `gc_safepoint()` calls, not at arbitrary Rust points.
  The single cycle of grace covers the narrow window between an alloc frame
  dropping and the next safepoint at which `VALUE_ROOTS` or the new alloc frame
  re-roots the value.  The old value of 10 kept 9× more garbage in RAM than
  necessary, worsening OOM pressure under long test suites.
- **Cycle collection**: because `GcPtr::drop` is a no-op, reference cycles do
  not prevent collection — any object unreachable from roots is freed.

---

## Deferred to later phases

- Incremental/concurrent collection — Phase 10+
- Write barriers for generational GC — Phase 10+
- Weak references (`WeakGcPtr<T>`) — deferred
- Safepoint integration with JIT frames — Phase 10+
- Automatic collection trigger (threshold-based) — deferred
