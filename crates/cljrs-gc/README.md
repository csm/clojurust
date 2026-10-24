# cljrs-gc

Non-moving, stop-the-world mark-and-sweep garbage collector for clojurust;
or, with the `no-gc` Cargo feature, a region-based allocator with no GC pauses.

On `wasm32` targets the `system-memory` crate is excluded (it brings in `errno`,
which does not build for `wasm32-unknown-unknown`).  The GC heap defaults to a
fixed **64 MB** soft limit instead of consulting total system RAM.

**Phase:** 8.1 (GcVisitor + Trace infrastructure) + 8.2 (GcBox/GcHeap
raw-pointer implementation) — implemented.  `no-gc` mode (Phases 1–8 of
`docs/archive/no-gc-plan.md`) — implemented.  B3 (`StaticGcPtr`, `static_alloc`) —
implemented.  Process memory governor
(`docs/managed-memory-governor-plan.md`) — Phases 0–4 (chunked allocation
credit; region, shared-value, message-queue, static, and code memory;
fallible allocation and opt-in strict hard-limit admission) implemented.

---

## Purpose

Manages all Clojure runtime values.  `GcPtr<T>` is a raw pointer into either
the GC heap or a bump-allocated region; `clone` is O(1); `drop` is a no-op.

**Default build (GC mode):** memory is freed only during `GcHeap::collect`.

**Region provenance tagging (GC mode):** region-allocated `GcPtr`s carry a
low-bit tag (`REGION_PTR_TAG`; `GcBox<T>` is ≥8-aligned so bit 0 is free).
`MarkVisitor::visit` checks the tag *without dereferencing* and **skips**
region objects — a region whose scope has ended leaves dangling pointers whose
headers are freed/reused memory, so tracing them would follow a garbage
`trace_fn`.  Because the mark phase no longer traces *into* region objects,
`GcHeap::collect` instead treats every live region on the thread's region
stack as a root (`region::trace_active_regions`), so heap objects reachable
only through a live region are still kept alive.  `GcPtr::raw()` masks the tag
on every dereference; `GcPtr::is_region_alloc()` exposes it.

**`no-gc` build:** every function call and every `loop` iteration pushes a
scratch `Region`; intermediates are freed when the scope exits.  Return values
and `recur` arguments are evaluated in the caller's context (the
"return-expression-in-caller" mechanism).  Static-sink expressions (`def`,
`defn`, `defmacro`, `atom`, `agent`, `volatile!`, `reset!`, `vreset!`,
`swap!`, `vswap!`, `alter-var-root`, `intern`) go to the global `StaticArena`
and live for the program lifetime.
No `GcHeap`, no stop-the-world pauses, no `Trace` overhead at runtime.

An isolated invocation can instead install `alloc_ctx::InvocationGuard`. In
that profile every allocation uses one bounded region; nested `ScratchGuard`
and `StaticCtxGuard` values become no-ops, so arbitrary internal graphs share
the invocation lifetime and are reclaimed together at the boundary.

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
                    ScratchGuard, StaticCtxGuard, InvocationGuard
  region.rs       — Region bump allocator, RegionGuard, thread-local region
                    stack; trace_active_regions() (GC-root scan of live regions);
                    poison/retire protocol (Phase 10.5 heap-promotion fallback):
                    poison_active_regions(), close_region(), retired-region
                    root tracing
  cancellation.rs — (GC mode) STW coordination, MutatorGuard, safepoints
  config.rs       — (GC mode) GcConfig, GcCancellation (zero-sized proxy),
                    IsolateCancellation thread-local (per-isolate STW state), GcParked
  governor.rs     — process-wide managed-memory governor: MemoryConfig,
                    ProcessMemoryGovernor, IsolateAccount, IsolateControl,
                    strict admission, critical sections, runtime-fault slot,
                    MemoryCharge, MemorySnapshot, pressure policy; region
                    chunk, permanent (static), and queue-wait accounting
  stats.rs        — process-global GcStats counters: GC allocations,
                    region (bump) allocations, GC pauses + freed bytes/objects,
                    isolate-boundary crossings (bytes copied + serialize time)
tests/
  governor.rs     — (GC mode) governor integration tests: per-isolate limit
                    multiplication, exact sweep accounting, cross-thread
                    collection requests, credit returned by collection
                    and thread exit, idle capacity borrowed by an active
                    isolate
  no_gc_alloc.rs  — (no-gc mode) integration tests for the allocation context stack:
                    ScratchGuard, StaticCtxGuard, InvocationGuard,
                    pop_for_return protocol, nested guards, destructor ordering
examples/
  credit_bench.rs — allocation throughput and credit-refill rate for several
                    credit chunk sizes (`cargo run --release -p cljrs-gc
                    --example credit_bench`)
  region_bench.rs — cost of a region scope (create, allocate four objects,
                    drop), including its governor charge
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
`num_rational::Ratio<BigInt>`, and `Mutex<Vec<T>>` for the primitive array
element types (`i8`/`i16`/`i32`/`i64`/`f32`/`f64`/`bool`/`char`).

Compiled regular expressions are *not* here: `Trace` for them lives on
`cljrs-value`'s `Pattern` wrapper, so `cljrs-gc` does not depend on a regex
engine.

### `GcPtr<T: Trace + 'static>`

```rust
pub struct GcPtr<T: Trace + 'static>(NonNull<GcBox<T>>);

impl<T: Trace + 'static> GcPtr<T> {
    pub fn new(value: T) -> Self        // allocates on HEAP (or ctx in no-gc); infallible
    pub fn try_new(value: T) -> Result<Self, AllocError<T>>  // strict governor may refuse
    pub fn new_critical(value: T) -> Self  // may use the critical reserve; never faults
    pub fn get(&self) -> &T             // borrow; invalid after collect frees it
    pub fn ptr_eq(a: &Self, b: &Self) -> bool

    // GC mode only:
    pub fn is_region_alloc(&self) -> bool  // true if bump-allocated in a Region

    // no-gc + debug_assertions only:
    pub fn is_static_alloc(&self) -> bool  // true if allocated in StaticArena
}
impl<T: Trace + 'static> Clone for GcPtr<T> { /* O(1) raw-pointer copy */ }
impl<T: Trace + 'static> Drop  for GcPtr<T> { /* no-op */ }

/// A refused allocation; owns the value that was not allocated.
pub struct AllocError<T> { /* value, error */ }
impl<T> AllocError<T> {
    pub fn error(&self) -> MemoryLimitExceeded;
    pub fn into_inner(self) -> T;
    pub fn into_parts(self) -> (T, MemoryLimitExceeded);
}
// Debug, Display, Error; From<AllocError<T>> for MemoryLimitExceeded
```

`try_new` asks the thread's account for the object's bytes before the object
exists (`governor::try_reserve_current`).  `new` cannot refuse: in strict
mode a refused allocation takes the critical reserve and raises the thread's
runtime fault, which the interpreter resolves at its next safepoint.

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
/// Fallible form: the strict governor may refuse `size_of::<T>()`.
pub fn try_static_alloc<T: 'static>(value: T) -> Result<StaticGcPtr<T>, AllocError<T>>;
```

### Free functions

```rust
// always:
pub fn static_alloc<T: 'static>(value: T) -> StaticGcPtr<T>;
pub fn try_static_alloc<T: 'static>(value: T) -> Result<StaticGcPtr<T>, AllocError<T>>;

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
    pub fn with_limit(limit: usize) -> Self
    pub fn alloc<T: Trace + 'static>(&mut self, value: T) -> GcPtr<T>
    pub fn reset(&mut self)
    pub fn bytes_used(&self) -> usize
    pub fn accounted_bytes(&self) -> usize
    pub fn byte_limit(&self) -> Option<usize>
    pub fn object_count(&self) -> usize
    pub fn chunk_bytes(&self) -> usize   // chunk memory held (its governor charge)
}
```

Each chunk is charged to the calling thread's `IsolateAccount`
(`charge_region`) when it is allocated and released when it is freed, so
region memory uses thread-local credit and appears under
`MemoryClass::Region`.  `close_region` moves a poisoned region's charge to
the retired-region count; it stays charged until the thread exits.

`with_limit` raises the typed `RegionLimitExceeded` panic payload when the
managed allocation budget is exhausted. It charges boxes plus
`Trace::gc_size_extra`; it is not a process-wide RSS limiter.

### `alloc_ctx::InvocationGuard` (`no-gc` only)

```rust
pub struct InvocationGuard { /* one bounded Region */ }
impl InvocationGuard {
    pub fn new(byte_limit: usize) -> Self;
    pub fn accounted_bytes(&self) -> usize;
    pub fn object_count(&self) -> usize;
}
pub fn invocation_is_active() -> bool;
```

All `GcPtr::new` calls within the guard use its single arena, including calls
made beneath evaluator scratch/static guards. Copy or serialize results before
the guard drops.

Bump allocator for short-lived objects. ~2.6x faster than `GcHeap::alloc`
(no mutex, no `Box::new`). Objects are NOT in the GC heap linked list.
Destructors run on `reset()` or `drop`.

### `region::RegionGuard`

RAII guard that pushes a `Region` onto the thread-local stack. Use with
`try_alloc_in_region()` for opportunistic region allocation.

### Region poisoning / retirement (Phase 10.5)

```rust
/// Mark every region currently active on this thread: each will be *retired*
/// (kept alive forever and traced as a GC root) instead of reset when its
/// scope closes.  No-op when no region is active.
pub fn poison_active_regions();

/// Close a region whose scope ended: pop the thread-local stack entry, then
/// reset/drop the region — or retire it if poisoned.  All owners of
/// stack-registered regions (rt_abi, the IR interpreter) close through here.
pub fn close_region(region: Box<Region>);
```

The heap-promotion fallback: when a publish barrier
(`cljrs_value::publish::publish_value`) meets a value it can neither verify
nor deep-copy while regions are open, it poisons them.  Retired regions are a
deliberate bounded leak (mirroring the JIT's pinned epochs) that can never
dangle; `GcHeap::collect` traces them as roots alongside the active stack.

### `governor` — process memory governor

Every isolate keeps its private heap; the governor owns counters, pressure
policy, and thread-safe control handles.  Each isolate allocates from local
credit that the governor grants in chunks; granted credit counts as committed.
**Observe-only by default:** no grant is refused; a grant that ends above the
hard limit is counted as an over-limit event.  **Strict mode**
(`MemoryConfig::strict`, `CLJRS_MEMORY_STRICT=1`) refuses application grants
that would take committed bytes above the hard limit; see *Strict admission*
below.  Region chunks use the same local credit
and are reported under `Region`.  Shared values (`cljrs-value`), queued
isolate-channel messages (`cljrs-async`), and JIT code (`cljrs-compiler`) hold
a `MemoryCharge`; static allocations are recorded with `charge_permanent`.

```rust
pub enum MemoryClass { GcHeap, Region, SharedValue, MessageQueue, Static, Code, Runtime }
pub enum PressureLevel { Green, Yellow, Red }   // < soft, >= soft, > hard (or strict refusal)
pub enum Admission { Application, Critical }    // <= hard, <= hard + critical reserve

pub const DEFAULT_CREDIT_CHUNK: usize;             // 64 KiB
pub const DEFAULT_RETAINED_CREDIT_CHUNKS: usize;   // 2
pub const ACCOUNTING_UNIT: usize;                  // 4 KiB; rounding for large grants
pub const MIN_COLLECTION_HEADROOM: usize;          // 1 MiB
pub const STRICT_ENV: &str;                         // "CLJRS_MEMORY_STRICT"

pub struct MemoryConfig { pub soft_limit, pub hard_limit, pub critical_reserve,
                          pub credit_chunk, pub retained_credit_chunks,
                          pub queue_limit, pub strict }   // bytes, except the chunk count
impl MemoryConfig {
    pub fn with_limits(soft: usize, hard: usize) -> Result<Self, MemoryConfigError>;
    pub fn from_optional_limits(soft: Option<usize>, hard: Option<usize>)
        -> Result<Self, MemoryConfigError>;
    pub fn platform_default() -> Self;
    pub fn from_env() -> Result<Self, MemoryConfigError>;   // CLJRS_MEMORY_*, deprecated CLJRS_GC_*
    pub fn from_lookup(lookup, warn) -> Result<Self, MemoryConfigError>;
    pub fn validate(&self) -> Result<(), MemoryConfigError>;
}
pub fn default_hard_limit() -> usize;                   // cgroup limit, else physical / 2
pub fn default_soft_limit(hard: usize) -> usize;        // 75%
pub fn default_critical_reserve(hard: usize) -> usize;  // max(4 MiB, 1%), <= 256 MiB
pub fn container_memory_limit() -> Option<usize>;       // not on wasm32

pub struct ProcessMemoryGovernor { /* atomics + weak isolate registry */ }
impl ProcessMemoryGovernor {
    pub const fn new() -> Self;   // reads MemoryConfig::from_env on first use
    pub fn with_config(c: MemoryConfig) -> Result<Self, MemoryConfigError>;
    pub fn configure(&self, c: MemoryConfig) -> Result<(), MemoryConfigError>;
    pub fn config(&self) -> MemoryConfig;
    pub fn register_isolate(&'static self, name: impl Into<Arc<str>>) -> IsolateAccount;
    pub fn reserve_shared(&'static self, class: MemoryClass, bytes: usize)
        -> Result<MemoryCharge, MemoryLimitExceeded>;   // Err only in strict mode
    pub fn reserve_critical(&'static self, class: MemoryClass, bytes: usize)
        -> Result<MemoryCharge, MemoryLimitExceeded>;   // may use the critical reserve
    pub fn reserve_reported(&'static self, class: MemoryClass, bytes: usize)
        -> MemoryCharge;                                // never refused (JIT code)
    pub fn charge_permanent(&'static self, class: MemoryClass, bytes: usize);  // never returned;
                                                        // a strict refusal raises a fault
    pub fn try_charge_permanent(&'static self, class: MemoryClass, bytes: usize)
        -> Result<(), MemoryLimitExceeded>;
    pub fn is_strict(&self) -> bool;
    pub fn within_hard_limit(&self) -> bool;
    pub fn record_queue_wait(&self, waited: Duration);
    pub fn record_queue_rejection(&self);
    pub fn class_bytes(&self, class: MemoryClass) -> usize;
    pub fn committed_bytes(&self) -> usize;     // exact: used + free credit
    pub fn free_credit_bytes(&self) -> usize;   // as last published by each isolate
    pub fn min_headroom(&self) -> usize;        // max(16 chunks, MIN_COLLECTION_HEADROOM)
    pub fn pressure(&self) -> PressureLevel;
    pub fn snapshot(&self) -> MemorySnapshot;
}
pub fn governor() -> &'static ProcessMemoryGovernor;   // the process instance

pub struct IsolateAccount { /* single-threaded (!Sync) Cells */ }
// Drop: returns used bytes, region bytes, and free credit, unregisters.
impl IsolateAccount {
    pub fn charge(&self, bytes: usize);   // thread-local until credit runs out, then refills
    pub fn try_reserve_credit(&self, bytes: usize) -> Result<(), MemoryLimitExceeded>;
    pub fn try_charge(&self, bytes: usize) -> Result<(), MemoryLimitExceeded>;
    pub fn release(&self, bytes: usize);  // sweep: used bytes become free credit
    pub fn charge_region(&self, bytes: usize);                  // new region chunk
    pub fn retire_region(&self, bytes: usize);                  // active -> retired
    pub fn release_region(&self, bytes: usize, retired: bool);  // freed chunk; trims excess credit
    pub fn publish(&self);                // metrics poll
    pub fn poll(&self) -> bool;           // safepoint: serve a recall; collection requested?
    pub fn record_collection(&self, r: CollectionReport);   // epoch, next target, trim credit
    pub fn used_bytes(&self) -> usize;
    pub fn free_credit_bytes(&self) -> usize;
    pub fn region_bytes(&self) -> usize;
    pub fn retired_region_bytes(&self) -> usize;
    pub fn committed_bytes(&self) -> usize;   // used + region + retired + free credit
    pub fn control(&self) -> &Arc<IsolateControl>;
}
pub struct IsolateControl { /* atomics */ }
impl IsolateControl {
    pub fn id(&self) -> IsolateId;
    pub fn name(&self) -> Arc<str>;
    pub fn request_collection(&self);
    pub fn collection_requested(&self) -> bool;
    pub fn take_collection_request(&self) -> bool;
    pub fn request_recall(&self);
    pub fn recall_requested(&self) -> bool;
    pub fn used_bytes(&self) -> usize;
    pub fn free_credit_bytes(&self) -> usize;
    pub fn region_bytes(&self) -> usize;
    pub fn retired_region_bytes(&self) -> usize;
    pub fn collection_target(&self) -> usize;
    pub fn collection_epoch(&self) -> u64;
    pub fn collection_requests(&self) -> u64;
    pub fn credit_refills(&self) -> u64;
}
pub struct CollectionReport { pub bytes_before, pub bytes_after, pub bytes_returned, pub duration }
pub struct MemoryCharge;   // RAII; Drop returns its bytes once
pub struct MemoryLimitExceeded { pub class, pub requested, pub committed, pub hard_limit }
pub struct MemorySnapshot { /* strict flag, limits, pressure, committed/used/free
                               credit and peaks, per class, per isolate, transitions,
                               requests, refills, returned credit, recalls, over-limit
                               events, retired region bytes, queue waits and rejections,
                               rejected reservations/bytes, critical grants, reserve
                               overruns, runtime faults raised and cleared */ }
pub struct IsolateSnapshot { /* per-isolate counters */ }

// Calling thread:
pub fn register_current_isolate(name: &str) -> Option<Arc<IsolateControl>>;
pub fn with_current_account<R>(f: impl FnOnce(&IsolateAccount) -> R) -> Option<R>;
pub fn with_registered_account<R>(f: impl FnOnce(&IsolateAccount) -> R) -> Option<R>;  // never registers
pub fn current_control() -> Option<Arc<IsolateControl>>;
pub fn snapshot() -> MemorySnapshot;   // flushes this thread's account first
pub fn try_reserve_current(bytes: usize) -> Result<(), MemoryLimitExceeded>;

// Critical sections (calling thread):
pub struct CriticalSection;            // guard; !Send; nests
pub fn enter_critical() -> CriticalSection;
pub fn with_critical<R>(f: impl FnOnce() -> R) -> R;
pub fn critical_active() -> bool;

// Runtime-fault slot (calling thread; fixed-size, never allocates):
pub fn raise_runtime_fault(fault: MemoryLimitExceeded);    // keeps the first; asks to collect
pub fn confirm_runtime_fault(fault: MemoryLimitExceeded);  // delivered unconditionally
pub fn runtime_fault_possible() -> bool;                    // one relaxed global load
pub fn pending_runtime_fault() -> Option<MemoryLimitExceeded>;
pub fn take_runtime_fault() -> Option<MemoryLimitExceeded>;
pub fn resolve_runtime_fault() -> Option<MemoryLimitExceeded>;  // clears if back within limit
```

`GcHeap::alloc` charges the thread's account; `GcHeap::collect` releases the
freed bytes and calls `record_collection`.  A thread that allocates without
registering is registered lazily under its thread name.

Credit: a charge subtracts from local credit.  When credit runs out, the
account asks the governor for one chunk, or for the shortfall rounded up to
`ACCOUNTING_UNIT` when it exceeds a chunk.  A refill is the only time an
isolate publishes its used bytes.  After a
collection, an isolate keeps at most `retained_credit_chunks` of free credit
at `Green`, one chunk at `Yellow`, and none at `Red`; it returns the rest.
When pressure rises, the governor recalls credit from every isolate, and each
returns its excess at its next refill, collection, or safepoint poll
(`gc_requested`).

Collection: each heap's own trigger is its `GcConfig` soft limit, as in
Phase 1.  The flag behind `request_gc` / `gc_requested` / `take_gc_request`
lives on the thread's `IsolateControl`, so the governor can request a
collection from another thread.  At `Yellow` the governor requests collection
from the allocating isolate and from the largest heap, at most once per
collection epoch and only above the isolate's post-collection target (the
surviving bytes plus the larger of half of them or `min_headroom()`; after a
zero-yield collection, the larger of all of them or `min_headroom()`); at
`Red` the target is ignored.  Pressure transitions log at `info` and
over-limit events at `debug` under the `memory` tracing target.

Class split: credit is granted under `GcHeap`.  Each time an account
publishes (refill, collection, poll, trim), the governor moves the account's
region bytes from `GcHeap` to `Region`, so committed bytes stay exact and the
class split lags by at most the account's free credit.  A freed region chunk
becomes free credit; credit above the retained limit returns to the governor
at once, because a `no-gc` isolate has no collection to trim it.

Static: `static_alloc` records `size_of::<T>()` in GC builds; the `no-gc`
`StaticArena` records each chunk.

Strict admission: `admit` commits a grant with a compare-and-swap on the
committed counter, so concurrent requests never commit past the bound.  An
application grant must keep committed bytes at or below the hard limit; a
critical one (inside a `CriticalSection`, `reserve_critical`, `new_critical`)
at or below the hard limit plus the critical reserve.  A refused credit chunk
is retried at the exact shortfall.  A refusal changes no counter, counts as a
rejected reservation, logs at `info`, holds pressure at `Red` until a normal
chunk fits again, and requests collection.  Fallible callers get the
`MemoryLimitExceeded`.  An infallible charge (`GcPtr::new`, region chunks,
`charge_permanent`) raises the thread's runtime fault and takes the shortfall
from the critical reserve; if the reserve is exhausted too, it is admitted
anyway and counted as a reserve overrun (logged at `warn`).  The runtime
resolves the fault at its next safepoint: after collecting, it continues if
committed bytes are within the hard limit and fails the evaluation otherwise.

Gaps: the isolate heap is not torn down at thread exit, so
`IsolateAccount::drop` stops counting bytes that stay allocated until process
exit (region chunks freed after the account is gone are not released twice);
an idle isolate that never polls is not woken by a recall, so it keeps up to
its retained chunks.

### `GcConfig`

Per-heap collection trigger.  `with_soft_limit(soft)` keeps the default hard
limit (raised to `soft`); `try_with_limits` / `validate` reject a zero hard
limit and a soft limit above the hard limit.  The hard limit is not enforced.

### `stats::GcStats` and `GC_STATS`

```rust
pub struct GcStats { /* AtomicU64 counters */ }

impl GcStats {
    pub const fn new() -> Self
    pub fn record_gc_alloc(&self, bytes: usize)
    pub fn record_gc_allocs(&self, count: u64, bytes: u64)   // batched by isolate accounts
    pub fn record_region_alloc(&self, bytes: usize)
    pub fn record_region_poison(&self)
    pub fn record_gc_pause(&self, pause: Duration, freed_objects: u64, freed_bytes: u64)
    pub fn record_boundary_crossing(&self, bytes: u64, copy_time: Duration)
    pub fn snapshot(&self) -> GcStatsSnapshot
}

pub struct GcStatsSnapshot { /* immutable view of counters */ }
impl GcStatsSnapshot {
    pub fn total_pause(&self) -> Duration
    pub fn total_boundary_copy(&self) -> Duration
}
impl std::fmt::Display for GcStatsSnapshot { /* multi-line summary */ }

pub static GC_STATS: GcStats;

pub const CLJRS_GC_STATS_ENV: &str;       // = "CLJRS_GC_STATS"
pub fn dump_stats_from_env();
pub fn report() -> String;   // GC_STATS + governor snapshot; re-exported as stats_report
```

Process-global counters updated automatically by `GcHeap::alloc`,
`GcHeap::collect`, and `Region::alloc`.  GC heap allocations are batched in
each isolate account and flushed at every credit refill, collection, metrics
poll, and thread exit, so the allocation path does not touch these shared
atomics; a reading can lag each thread by up to one chunk of allocations.  The `cljrs --gc-stats [FILE]` CLI
flag prints a snapshot of these counters at program exit.

`record_boundary_crossing` is the **metered isolate-boundary seam** required by
`docs/isolate-boundary-plan.md`: every value deep-copied across an isolate
boundary (the Phase B2 structured-clone in `cljrs-async`'s `IsolateSender::send`)
records its estimated bytes copied and serialize time here, so a silent fan-out
copy shows up in `--gc-stats` as `Boundary crossings: N (B bytes copied)` rather
than as mystery latency.

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
