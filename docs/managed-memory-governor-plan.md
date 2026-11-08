# Process-Wide Managed-Memory Governor

Status: Phases 0 through 4 implemented (chunked allocation credit, non-heap memory classes, fallible allocation, and opt-in strict hard-limit admission). Phase 5 is open.
See [Implementation status](#implementation-status).

This document defines a process-wide governor for memory that clojurust manages.
The governor preserves private isolate heaps and independent garbage collection.

This design replaces the memory-pressure section in `async-worker-pool-plan.md`.
That document remains the source for isolate execution and boundary rules.

## Decision

clojurust will keep a private heap for each isolate.
Each isolate will continue to collect its heap without a global stop-the-world pause.

A process-wide governor will control allocation admission across all isolates.
The governor will also account for managed memory outside the GC heaps.

The fast allocation path will use local allocation credit.
When an isolate needs more credit, it will contact the governor.

This design separates two operations:

- Garbage collection finds unreachable objects at cooperative safepoints.
- Allocation admission limits memory before the runtime commits more managed bytes.

Collection remains cooperative because it needs a complete root set.
Allocation admission does not depend on another isolate reaching a safepoint.

## Why this design

A shared heap makes arbitrary pointer sharing possible.
It also requires global root coordination and thread-safe mutation rules.

Fixed per-isolate quotas give simple accounting.
Uneven workloads then leave capacity unused.

A global atomic counter on every allocation gives current totals.
It also adds shared cache-line traffic to the local allocation path.

RSS-only control includes native and allocator memory.
It does not identify the isolate or memory class that caused pressure.

Chunked credit keeps common allocation local.
The process governor still controls the maximum admitted capacity.

## Goals

The governor must provide these properties:

1. One process budget covers all registered isolates.
2. An idle isolate does not reserve a permanent fixed share of the budget.
3. An active isolate can borrow unused capacity from other isolates.
4. Common allocations do not use a global lock or global atomic operation.
5. The governor can request collection from a selected isolate.
6. The hard limit rejects new governed allocations after all allowed reserves are exhausted.
7. Queue backpressure prevents serialized messages from bypassing the budget.
8. Metrics show current use by isolate and memory class.
9. Isolate shutdown returns all credit and releases all private heap objects.
10. The design supports a future shared immutable value tier.

## Non-goals

This feature does not introduce a shared tracing heap.
It does not make `GcPtr` implement `Send` or `Sync`.
It does not permit arbitrary closures or local mutable references to cross isolates.
It does not make the process resident set equal to the managed-memory count.
It does not add a Clojure-level isolate spawn operation.
It does not add an incremental or concurrent collector inside one isolate.

## Current problems

Each heap currently reads the same memory configuration.
The configured value therefore acts as a per-isolate value, not a process value.

The allocator only examines the soft limit.
The `hard_limit_exceeded` method has no allocation call site.

The allocator requests collection after it passes the soft limit.
The runtime services that request at a later interpreter or async safepoint.

Isolate channels use an unbounded queue of `SerializedValue` objects.
This queue is outside both GC heap counters.

`GC_STATS` contains process-wide cumulative counters.
It does not contain current bytes by heap or memory class.

Regions, shared byte blobs, static values, and code caches have separate lifetime rules.
The current heap counter does not cover all of them.

The thread-local heap owns objects through a raw linked list.
The heap has no explicit shutdown operation that releases all remaining objects.

## Terminology

**Managed memory** is memory that participates in governor accounting.

**Committed bytes** are bytes that the governor has admitted against the hard limit.

**Allocation credit** is committed capacity that an isolate can use without another governor call.

**Used bytes** are committed bytes that back live allocations or retained garbage.

**Free credit** is committed capacity that an isolate has not used.

**Critical reserve** is capacity for collection, shutdown, and error reporting.
Application allocations cannot use this reserve.

**Memory class** identifies the owner and lifetime rule for an allocation.

**Pressure level** is `Green`, `Yellow`, or `Red`.

## Required invariants

The implementation must preserve these invariants:

1. `application_committed_bytes` must not exceed `hard_limit`.
2. Total committed bytes must not exceed `hard_limit + critical_reserve`.
3. Only runtime control operations can use the critical reserve.
4. A byte belongs to one memory class at one time.
5. A shared allocation receives one charge, independent of its `Arc` count.
6. A queue charge exists while the serialized message exists.
7. Heap sweep returns the exact charge stored in each freed header.
8. Region release returns the charge for each released region chunk.
9. Isolate shutdown returns its free credit after it releases its private allocations.
10. A failed reservation does not change committed or used counters.
11. Counter underflow is a runtime error in debug builds.

The hard limit covers governed allocations only.
The runtime must not describe it as an exact RSS limit.

## Architecture

The first implementation has one process governor and one account per isolate.
All runtime instances in the process use the same process governor.

```text
ProcessMemoryGovernor
├── process limits and pressure level
├── current bytes by memory class
├── isolate registry
│   ├── IsolateAccount A
│   │   ├── local allocation credit
│   │   ├── current heap bytes
│   │   └── shared IsolateControl
│   └── IsolateAccount B
├── shared-value bytes
├── queued-message bytes
├── static bytes
└── critical reserve
```

The governor does not own any heap object.
It owns counters, allocation policy, and isolate control handles.

Each isolate still owns its GC heap, region stack, and root stacks.
The isolate account is thread-confined except for its control handle and published metrics.

### Process governor

The governor has this conceptual interface:

```rust
pub struct ProcessMemoryGovernor { /* process-global state */ }

pub enum MemoryClass {
    GcHeap,
    Region,
    SharedValue,
    MessageQueue,
    Static,
    Code,
    Runtime,
}

pub enum PressureLevel {
    Green,
    Yellow,
    Red,
}

impl ProcessMemoryGovernor {
    pub fn register_isolate(&self, name: Arc<str>) -> IsolateAccount;
    pub fn reserve_shared(
        &self,
        class: MemoryClass,
        bytes: usize,
    ) -> Result<MemoryCharge, MemoryLimitExceeded>;
    pub fn snapshot(&self) -> MemorySnapshot;
}
```

`MemoryCharge` is an RAII value for allocations with independent lifetimes.
Its `Drop` implementation returns the charge to the governor.

GC heap objects do not need one RAII value per object.
Each header stores its admitted size, and sweep returns the total freed size.

### Isolate registration

An isolate registers before it creates its runtime or allocates a `GcPtr`.
Registration returns an `IsolateAccount` and an `Arc<IsolateControl>`.

`IsolateControl` contains only thread-safe control data:

```rust
pub struct IsolateControl {
    id: IsolateId,
    gc_requested: AtomicBool,
    used_bytes: AtomicUsize,
    last_collection_epoch: AtomicU64,
}
```

The governor stores weak control handles in its registry.
The governor can request collection, but it cannot trace or sweep another isolate heap.

The current thread-local cancellation state will move behind `IsolateControl`.
This change lets the governor send a collection request across threads.

### Allocation credit

Each isolate keeps a thread-local credit balance.
The initial credit chunk is 64 KiB.
The final value will come from benchmarks.

An ordinary heap allocation performs these operations:

1. Calculate the stored object size.
2. Subtract that size from local credit.
3. Allocate and link the object.
4. Add the size to the thread-local used-byte counter.

The common path does not contact the governor.
It publishes its current used-byte value during a refill, collection, or metrics poll.

If local credit is insufficient, the isolate requests another credit chunk.
When the process budget has capacity, the governor admits the chunk.

Large allocations request their actual size, rounded to the accounting unit.
They do not reserve many normal credit chunks.

After a collection, an isolate keeps at most two unused chunks.
It returns all additional free credit to the process governor.

This rule prevents idle isolates from holding large reservations.

The governor can request a credit recall from an isolate.
The isolate returns excess free credit at its next runtime service poll.

The runtime must wake an idle isolate after a recall request.
An unresponsive isolate can retain only its configured maximum number of chunks.

### Credit accounting

The governor counts granted credit as committed memory.
This rule bounds later fast-path allocations without another global operation.

The following equation must hold:

```text
application_committed_bytes
  = used_isolate_credit
  + free_isolate_credit
  + active_direct_reservations

total_committed_bytes
  = application_committed_bytes
  + critical_reserve_in_use
```

Direct reservations cover regions, shared values, queued messages, static data, code, and runtime data.

Metrics will report used bytes and free credit separately.
Operators can then distinguish live data from reserved capacity.

The hard-limit overshoot is bounded by memory that the governor cannot admit in advance.
Examples include native-library memory and a value built before its GC wrapper exists.

Later phases will move reservations into large collection and buffer constructors.
This change will reduce temporary overshoot.

## Pressure policy

The governor derives pressure from committed bytes.
The default soft limit is 75 percent of the hard limit.

| Level | Condition | Governor action |
|---|---|---|
| `Green` | Below the soft limit | Grant normal credit. |
| `Yellow` | At or above the soft limit | Request collection and reduce retained free credit. |
| `Red` | A request cannot fit below the hard limit | Reject application credit and apply backpressure. |

### Yellow pressure

At `Yellow`, the governor first requests collection from the requesting isolate.
It can also request collection from the largest registered heap.

The governor must rate-limit repeated requests.
It must not request another collection from an isolate with the same collection epoch.

A collection request remains cooperative.
The isolate services it at its next valid safepoint.

The governor can still admit an allocation while collection is pending.
When the hard budget has capacity, it admits the allocation.

### Red pressure

At `Red`, the governor rejects new application credit.
The governor continues to admit control operations from the critical reserve.

The requesting isolate then performs one of these actions:

- It reaches a valid safepoint and collects its heap.
- It waits for another isolate to return credit.
- It returns `MemoryLimitExceeded` from a fallible allocation boundary.
- It sheds queued or incoming work.

The caller selects the action from its execution context.
The allocator must not block the isolate thread while that same thread must collect.
Queue operations can wait, but a heap allocation must collect or return an error.

## Hard-limit error path

The current `GcPtr::new` interface cannot report allocation rejection.
Strict enforcement therefore requires a fallible allocation path.

The GC crate will add this interface:

```rust
pub fn try_new(value: T) -> Result<GcPtr<T>, AllocError<T>>;
```

`AllocError<T>` returns ownership of `value`.
Dropping the error releases any memory that the value owns.

The runtime will add `ValueError::MemoryLimitExceeded` and `EvalError::MemoryLimitExceeded`.
The error data will use fixed-size numeric fields and a static memory-class name.

The error path must not allocate a Clojure error value before memory becomes available.
The interpreter can propagate the Rust error directly to its evaluation boundary.

Compiled code will use a pending runtime-fault slot.
This slot will remain separate from the pending Clojure-exception slot.

The allocation bridge will store the fixed-size fault and return a static sentinel.
The compiled entry seam will convert the fault to `EvalError::MemoryLimitExceeded`.

`GcPtr::new` will remain during migration.
In observe-only mode, it records virtual overage and continues the allocation.

Strict mode cannot become the default until all application allocation paths use `try_new`.
Runtime control allocations can use an explicitly named critical-allocation function.

## Garbage collection integration

Each `GcBoxHeader` already stores the accounted object size.
Allocation will reserve that size before `Box::into_raw` commits the object.

Sweep will return `freed_bytes` to the isolate account.
The isolate account can then return excess free credit to the governor.

The heap-local soft limit will become a collection target, not an ownership limit.
The governor will calculate the target from current process pressure and active heap sizes.

When process capacity exists, an isolate can grow beyond its current target.
This behavior removes fixed per-isolate partitions.

The collector will report these values after each collection:

- Bytes before collection
- Bytes after collection
- Bytes returned to the governor
- Collection duration
- Collection epoch
- Zero-yield collection count

The existing zero-yield backoff remains local to an isolate.
At `Red`, process pressure overrides local zero-yield suppression.

## Memory classes

### GC heap

GC heap accounting uses the size in `GcBoxHeader`.
Allocation consumes isolate credit, and sweep returns it.

Persistent vectors, hash maps, hash sets, sorted maps, and sorted sets share nodes between versions.
A version derived by one update from a GC-allocated version is charged only its path-copied nodes, which is O(log n).
A version built wholesale, cloned, or derived from a version that never reached the heap is charged its full size.
A derived version is also charged in full when its count reaches twice that of its nearest fully charged ancestor.
A loop of `conj` calls therefore charges O(n) element bytes in total plus O(log n) bytes per step.
A surviving version whose fully charged ancestor was collected is under-counted by less than twice that ancestor's charge.
`cljrs-value`'s `collections/share_charge.rs` holds the rule.

The implementation must remove the unused hard-limit logic from `GcHeap`.
The process governor becomes the only hard admission authority.

### Regions

When a region allocates a new chunk, it will reserve memory.
When the region releases the chunk, it will return that memory.

A retired region remains charged for its full retained capacity.
The charge makes region poisoning visible to the governor.

The region metrics will report active and retired bytes separately.

### Shared immutable values

Each shared allocation will contain one `MemoryCharge` in its owner object.
Cloning an `Arc` will not create another charge.

This rule applies first to `ByteBlob` and `SharedAtom` storage.
It will also apply to future shared vectors, maps, sets, and records.

Promotion must reserve the shared allocation before it publishes the value.
A failed reservation leaves the old shared value unchanged.

### Isolate message queues

An isolate channel will use a byte-bounded queue.
Each queued item will have this form:

```rust
struct AccountedMessage {
    value: SerializedValue,
    charge: MemoryCharge,
}
```

Serialization will calculate the owned wire size.
The existing `byte_size` estimate describes receiver materialization and cannot serve as the queue charge.

The wire size excludes shared `Arc` payloads and static references.
Their owner objects already hold the applicable process charge.

The sender must reserve queue bytes before enqueue.
The charge remains active until the receiver finishes deserialization or drops the item.

The receiver heap reserves its new objects separately.
This temporary double charge represents the real overlap during deserialization.

The channel will provide these operations:

- An async send that waits for byte capacity
- A nonblocking send that returns `Full`
- A configurable byte limit
- A close operation that releases all queued charges

The existing unbounded synchronous send will become a compatibility operation.
If the channel or process limit rejects the message, this operation will return `Full`.

### Static memory

Static allocations remain valid for the process lifetime.
The governor will charge them to `MemoryClass::Static`.

Static memory cannot return capacity during normal execution.
The default budget must leave room for bootstrap and interned values.

An intern-table limit is outside the first implementation.
Metrics must make static growth visible before that policy exists.

### Code and runtime memory

The first release will report JIT code and known runtime caches.
It will not deny these allocations through the governor.

A later release can add admission control at module publication and cache growth points.
Code reclamation at GC safepoints will return its charges.

## Configuration

`GcConfig` currently mixes process limits with heap collection policy.
The implementation will split these concerns.

```rust
pub struct MemoryConfig {
    pub soft_limit: usize,
    pub hard_limit: usize,
    pub critical_reserve: usize,
    pub credit_chunk: usize,
    pub queue_limit: usize,
}

pub struct HeapConfig {
    pub zero_yield_backoff: bool,
    pub retained_credit_chunks: usize,
}
```

The process reads `MemoryConfig` once.
An isolate does not read process-limit environment variables during spawn.

The proposed environment variables are:

| Variable | Meaning |
|---|---|
| `CLJRS_MEMORY_SOFT_LIMIT_MB` | Process-wide soft limit |
| `CLJRS_MEMORY_HARD_LIMIT_MB` | Process-wide hard limit |
| `CLJRS_MEMORY_CRITICAL_RESERVE_MB` | Reserve for runtime control operations |
| `CLJRS_MEMORY_CREDIT_KB` | Normal isolate credit chunk |
| `CLJRS_ISOLATE_QUEUE_LIMIT_MB` | Default byte limit for one isolate channel |

The existing `CLJRS_GC_SOFT_LIMIT_MB` and `CLJRS_GC_HARD_LIMIT_MB` names will remain as deprecated aliases.
The aliases will set process-wide values, not per-isolate values.

If both old and new variables exist, the new variable wins.
The runtime will write one warning for each used old variable.

The configuration parser must reject these states:

- A zero hard limit
- A soft limit greater than the hard limit
- A critical reserve greater than the hard limit
- A zero credit chunk
- A zero queue limit
- A queue limit greater than the application budget

The default hard limit will use the applicable container or process memory limit.
If no limit is available, it will use a fraction of physical memory.

The default critical reserve will be the larger of 4 MiB or one percent of the hard limit.
It will have a documented maximum.

## Metrics and diagnostics

The governor will provide a point-in-time `MemorySnapshot`.
The existing `GC_STATS` output will include this snapshot.

The snapshot will contain:

- Soft and hard limits
- Current pressure level
- Total committed bytes
- Total used bytes
- Total free isolate credit
- Current bytes for each memory class
- Current bytes for each isolate
- Collection requests and completed collections for each isolate
- Rejected reservation count and bytes
- Queue wait count and duration
- Peak committed and used bytes

The process will emit a structured event for each pressure transition.
It will emit a separate event for each rejected reservation.

Normal allocation will not emit an event.

## Isolate shutdown

Isolate shutdown must be explicit.
It will use this order:

1. Stop admission of new isolate work.
2. Close isolate channels and release queued messages.
3. Drain or cancel local tasks.
4. Drop the `LocalSet` and its futures.
5. Release all remaining GC heap boxes without a mark phase.
6. Release all active and retired region chunks.
7. Return unused allocation credit.
8. Unregister the isolate control handle.

The heap shutdown pass must run each object destructor once.
The pass must not use reachability or the normal grace cycle.

Runtime control allocations during shutdown can use the critical reserve.
Application allocation after shutdown starts is an error.

## Concurrency and ordering

The common allocation path uses only thread-local counters.
Credit refill uses atomic process counters and a registry lock on the slow path.

The registry lock must not remain locked while an isolate performs collection.
The governor only sets the target `gc_requested` flag while it holds the lock.

Counter updates will use relaxed ordering unless they publish a pressure transition.
Pressure transitions will use acquire-release ordering.

Exact snapshot consistency is not required.
Each snapshot field must represent a valid atomic observation.

## Implementation plan

### Implementation status

Phase 0 is complete:

- `--gc-soft-limit-mb` alone sets the soft limit (`GcConfig::with_soft_limit`).
- The CLI rejects a zero hard limit and a soft limit above the hard limit.
- `CLJRS_GC_HARD_LIMIT_MB` below `CLJRS_GC_SOFT_LIMIT_MB` produces a warning and uses the soft limit.
- `GcConfig`, the CLI help, and the book state that the hard limit is not enforced.
- `crates/cljrs-gc/tests/governor.rs` shows two heaps that each stay under the soft limit while their sum exceeds it.

Phase 1 is complete in `crates/cljrs-gc/src/governor.rs`:

- `MemoryConfig` reads the `CLJRS_MEMORY_*` variables and the deprecated aliases, and it rejects the invalid states listed in [Configuration](#configuration).
- `--gc-hard-limit-mb` configures the process governor (soft limit 75% of it). `--gc-soft-limit-mb` sets only the per-heap trigger, because N isolates each near a shared soft limit would hold the governor at `Yellow`.
- `Runtime::build` and `Isolate::spawn` register the calling thread. A thread that allocates first is registered under its thread name.
- `GcHeap::alloc` charges the thread's `IsolateAccount`. `GcHeap::collect` returns the freed header sizes and reports the collection.
- The GC request flag moved to `IsolateControl`, so the governor can request collection across threads.
- `--gc-stats` and `CLJRS_GC_STATS` print the `MemorySnapshot` after the GC counters.
- Pressure transitions log under the `memory` tracing target.

Phase 1 decisions that later phases must keep or replace:

- ~~An account publishes to the process counters after each credit chunk of growth and after each collection.~~
  Replaced in Phase 2 by granted credit.
- ~~Committed bytes equal used bytes, and `free_credit_bytes` is zero.~~
  Replaced in Phase 2: committed bytes include free credit.
- In observe-only mode, growth that ends above the hard limit increments `over_limit_events`.
  It does not reject the allocation. In Phase 2, the growth event is a credit grant.
- After each collection, the isolate gets a collection target that applies only at `Yellow`.
  For a collection that frees memory, the target is the remaining bytes plus the larger of half the remaining bytes or 16 credit chunks (at least 1 MiB).
  For a zero-yield collection, the headroom is the larger of the remaining bytes or that minimum.
  At `Red`, the governor ignores the target. It makes at most one request per collection epoch in both cases.
- The default hard limit is the cgroup memory limit, or half of physical memory if no cgroup limit applies.
- Thread exit drops the account and returns its used bytes and free credit.
  The heap objects remain allocated because the heap shutdown pass is not implemented yet.
- The heap-local soft limit still triggers collection, and Phase 2 keeps it. See [Phase 2 benchmark](#phase-2-benchmark).

Phase 2 is complete in `crates/cljrs-gc/src/governor.rs`:

- `IsolateAccount` holds local used bytes and free credit.
  `charge` subtracts from local credit and touches only thread-local cells.
- When credit runs out, the account asks the governor for one credit chunk.
  A shortfall larger than one chunk gets its own size, rounded up to a 4 KiB accounting unit.
  The governor adds the grant to committed bytes.
- `committed_bytes` is one process atomic, changed only by grants, returns, and direct reservations.
  It equals the sum of every account's used bytes and free credit plus direct reservations.
- Sweep moves freed bytes from used bytes to free credit.
  After a collection, the account keeps at most `retained_credit_chunks` (default 2) of free credit at `Green`, one chunk at `Yellow`, and none at `Red`.
  It returns the rest to the governor.
- When pressure rises, the governor sets a recall flag on every registered isolate.
  Each isolate returns its excess free credit at its next refill, collection, or safepoint poll (`gc_requested`).
- Thread exit returns the account's used bytes and free credit.
- Collection triggers are unchanged from Phase 1: the per-heap `GcConfig` soft limit, and governor requests at `Yellow` and `Red`.
- `GC_STATS` allocation counters are batched in the account and flushed at each refill, collection, metrics poll, and thread exit.
  The normal allocation path therefore performs no process-wide atomic operation.
- `MemorySnapshot` reports used bytes, free credit, peak used bytes, credit refills, returned credit, and recalls.
  `IsolateSnapshot` adds free credit, the collection target, and refills.
- `crates/cljrs-gc/examples/credit_bench.rs` measures allocation time and refill rate for several chunk sizes.

Phase 2 decisions that later phases must keep or replace:

- The governor never refuses a grant. Phase 4 adds strict admission at `ProcessMemoryGovernor::grant`.
- A recall does not wake an idle isolate. Such an isolate keeps at most its retained chunks until it polls.
- The governor's free-credit counter is the sum of each isolate's last published value.
  It can overstate current free credit by up to one chunk per isolate.
- The default credit chunk stays 64 KiB. See [Phase 2 benchmark](#phase-2-benchmark).
- The Phase 2 item "Replace per-isolate process-sized thresholds with dynamic collection targets" is deferred to Phase 5.
  A dynamic-target implementation was measured and reverted; see [Phase 2 benchmark](#phase-2-benchmark).
- The heap shutdown pass is not assigned to a phase yet.
  Completion criterion 7 requires it.

Phase 3 is complete:

- Region chunks consume the isolate's local credit (`IsolateAccount::charge_region`) and return it when freed (`release_region`).
  Each publish moves the account's region bytes from the governor's `GcHeap` class to its `Region` class; committed bytes do not change.
  A retired region keeps its charge under a separate retired count (`retire_region`).
  `MemorySnapshot` and `IsolateSnapshot` report active and retired region bytes separately.
- `ByteBlob` (`cljrs-value`) owns one `MemoryCharge` for its buffer; `Value::ByteBlob` holds `Arc<ByteBlob>` instead of `Arc<[u8]>`.
- `SharedAtom` stores `SharedSlot` values; each slot owns one `MemoryCharge` for its `Arc` allocation and string payload.
  Cloning an `Arc` does not add a charge.
- Isolate channels (`cljrs-async`) are byte-bounded queues of `AccountedMessage { value, charge }`.
  The charge is `SerializedValue::wire_size`, reserved before enqueue and returned after the receiver deserializes the message.
  `send`/`try_send` return `CloneError::Full`; `send_async` waits for capacity; `IsolateReceiver::close` (and drop) releases every queued charge.
  The default limit is `MemoryConfig::queue_limit`; `isolate_channel_with_limit` and `(isolate-chan limit-bytes)` set one per channel.
  `MemorySnapshot` reports queue waits, total wait time, and rejections.
- JIT modules hold a `MemoryClass::Code` charge for their code size until reclamation frees them.
- `static_alloc` records `size_of::<T>()` under `MemoryClass::Static` in GC builds; the `no-gc` static arena records each chunk.
  These use `ProcessMemoryGovernor::charge_permanent`, which never returns the bytes.

Phase 3 decisions that later phases must keep or replace:

- Region chunks use isolate credit rather than direct reservations, so a region scope performs no process-wide atomic operation.
  `crates/cljrs-gc/examples/region_bench.rs` measured about 5 ns (5%) more per region scope (create, four allocations, drop) on one thread.
  A freed chunk's bytes become free credit; credit above the retained limit returns to the governor at once, because a `no-gc` isolate has no collection to trim it.
- The `Region` class value lags each isolate's region use by at most its free credit, because the class split moves only when the account publishes.
- A region chunk freed after its thread's account is dropped is not released again; the account returned it at drop.
- Shared-value, queue, and code charges never fail while the governor is observe-only.
  `ByteBlob::new`, `SharedSlot::new`, and code registration keep an `Option<MemoryCharge>` so Phase 4 can add fallible variants.
- The compatibility `send` is the nonblocking send.
  A message larger than the channel limit is rejected with `Full` even by `send_async`, because it can never fit.
- Var shared roots (`Var::shared_root`), `SharedAtom` metadata, and the intern tables' `Arc<str>` names are not charged.
  Static charges count only `size_of::<T>()` in GC builds, not memory the value owns.
- `isolate-take!` still polls with `yield_now`; it does not use the channel's wakeup.

Phase 4 is complete:

- `MemoryConfig::strict` (`CLJRS_MEMORY_STRICT`, default off) turns on strict admission.
  A CLI hard limit (`--gc-hard-limit-mb`) keeps the environment's strict setting.
- `ProcessMemoryGovernor::admit` commits a grant with a compare-and-swap on the committed counter.
  An application grant (`Admission::Application`) must keep committed bytes at or below the hard limit.
  A critical grant (`Admission::Critical`) must keep them at or below the hard limit plus the critical reserve.
  A refused credit chunk is retried at the exact shortfall.
- A refusal changes no counter.
  It increments `rejected_reservations` and `rejected_bytes`, logs under the `memory` target, requests collection, and holds pressure at `Red` until a normal chunk fits below the hard limit again.
- Fallible paths: `GcPtr::try_new` returns `AllocError<T>`, which owns the value.
  `IsolateAccount::try_reserve_credit` and `try_charge`, `reserve_shared`, `reserve_critical`, `try_charge_permanent`, and `try_static_alloc` return `MemoryLimitExceeded`.
  `ByteBlob::try_new` and `try_with`, `SharedSlot::try_new`, and `SharedAtom::try_new`, `try_reset`, and `try_compare_and_set` reserve before they build or publish, so a refusal leaves the old shared value unchanged.
  The infallible `ByteBlob::new` and `SharedSlot::new` use `ProcessMemoryGovernor::reserve_infallible`, which always returns a charge for the full size (a counted reserve overrun once the reserve is exhausted).
- `ValueError::MemoryLimitExceeded`, `EvalError::MemoryLimitExceeded`, `CloneError::MemoryLimitExceeded`, and `SerializedErrorKind::MemoryLimitExceeded` carry the fixed-size `MemoryLimitExceeded`.
  `try`/`catch` and `rt_try` do not intercept it, like `GasExhausted`.
  A future whose body fails with it settles as `FutureState::MemoryLimitExceeded`, so `await` and `deref` re-raise the fault rather than a catchable thrown value; compiled async polls see `Readiness::MemoryLimitExceeded` and report it through the fault slot.
- Runtime faults: an infallible allocation (`GcPtr::new`, a region chunk, `charge_permanent`, `ByteBlob::new`, `SharedSlot::new`) that strict admission refuses raises the thread's pending runtime fault.
  The fault slot is a thread-local holding fixed-size data, separate from the compiled-code pending-exception slot.
  The allocation then takes its exact shortfall from the critical reserve and requests collection.
- `gc_safepoint` now returns `EvalResult<()>`.
  After it collects, it resolves the fault (`resolve_runtime_fault`): if committed bytes are back below the hard limit with headroom to spare it clears the fault and continues; otherwise it returns `EvalError::MemoryLimitExceeded`.
  The headroom is `min_headroom()` (at least 1 MiB), capped at an eighth of the hard limit.
  Without it, a program whose live data creeps up to the limit collects after every few allocations.
  A process-wide count of pending faults keeps the safepoint fast path at one extra relaxed load.
- Compiled code: the JIT entry seam (`tiered::apply`), the OSR seam, and the AOT harness check the fault slot after native code returns.
  `callback::invoke` maps `EvalError::MemoryLimitExceeded` to `ValueError::MemoryLimitExceeded` and confirms the fault in the slot, so a seam reports it even when compiled code drops the error.
  `value_error_to_eval_error` clears the slot when the error reaches the interpreter as an `EvalError`.
  The interpreter and compiled code therefore report the same `EvalError::MemoryLimitExceeded`.
- Critical allocation path: `enter_critical`/`with_critical` (`CriticalSection`), `GcPtr::new_critical`, and `reserve_critical`.
  `EvalError::to_error_value` and `value_error_to_eval_error` build error values with `new_critical`.
- Isolate channels return `CloneError::MemoryLimitExceeded` when the governor refuses a queue charge.
  `send_async` returns it at once.
- JIT code uses `reserve_reported`, which is never refused.
- `GcConfig::hard_limit_exceeded` is removed; the process governor is the only hard admission authority.
- `MemorySnapshot` adds the strict flag, rejected reservations and bytes, critical grants, reserve overruns, and runtime faults raised and cleared.
- Tests: governor unit tests (refusals leave counters unchanged, concurrent strict requests never exceed the hard limit, fault raise/resolve/confirm, critical sections, reserve overruns), `crates/cljrs-gc/tests/governor.rs` (`try_new`, `new`, `new_critical`, `try_static_alloc` against the process governor), and `crates/cljrs-runtime/tests/memory_limit.rs` (tree-walker and Tier-1 IR interpreter).

Phase 4 decisions that later phases must keep or replace:

- Strict mode stays opt-in. See [Rollout](#rollout).
- Most of the roughly 550 `GcPtr::new` call sites were not rewritten to `try_new`.
  A builtin holds unrooted values in Rust locals, so an allocation site cannot collect, and an immediate error at the first refusal would fail programs that one collection would let continue.
  These sites rely on the runtime-fault path instead: the critical reserve covers allocation until the next safepoint, which collects before it decides.
  `try_new` is for callers that can report an error at once.
- The allocation bridge does not return a sentinel.
  A refused `rt_*` allocation succeeds from the critical reserve, so compiled code continues with a valid value until the entry seam reports the fault.
- A long native loop with no safepoint (one builtin that builds a large collection) can exhaust the critical reserve.
  Infallible allocations then continue past `hard_limit + critical_reserve` and are counted as reserve overruns, instead of aborting.
  Moving large collection and buffer constructors to `try_new` closes this gap.
- Compiled loops call `rt_safepoint`, which parks but does not collect or resolve faults.
  A fault raised in compiled code is reported at the entry seam, after the native call returns.
- A fault that no safepoint resolves before an evaluation returns is resolved at the next evaluation's first safepoint.
- `Yellow` and `Red` collection requests are unchanged; a refusal latches `Red` only while a normal chunk does not fit.
- In `no-gc` builds `try_new` and `try_static_alloc` ask the governor only when the active region or the static arena must add a chunk (`Region::chunk_needed`, `StaticArena::try_alloc`); an object that fits in memory already charged is never refused.
- The final Clojure representation of `MemoryLimitExceeded` remains deferred; Clojure code cannot catch it.
- Persistent vectors, hash maps, hash sets, sorted maps, and sorted sets charge a derived version only its path-copied nodes (see "GC heap" under memory classes).
  A growing vector under strict mode therefore reaches the limit by retained data, and `crates/cljrs-runtime/tests/memory_limit.rs` retains one with `conj`.
- The heap shutdown pass is still not assigned to a phase.

#### Phase 2 benchmark

Measured on a 4-core Linux container, release builds, Phase 1 (`main` at `80cde63`) against Phase 2.

`crates/cljrs-gc/examples/credit_bench.rs` allocates 2 million 64-byte objects per thread:

| Chunk | Phase 1, 1 thread | Phase 2, 1 thread | Phase 1, 8 threads | Phase 2, 8 threads | Refills per MiB |
|---|---|---|---|---|---|
| 4 KiB | 92 ns | 79 ns | 728 ns | 173 ns | 256 |
| 16 KiB | 90 ns | 77 ns | 850 ns | 177 ns | 64 |
| 64 KiB | 88 ns | 73 ns | 813 ns | 146 ns | 16 |
| 256 KiB | 109 ns | 77 ns | 814 ns | 167 ns | 3.8 |
| 1 MiB | 88 ns | 82 ns | 797 ns | 153 ns | 0.8 |

Values are nanoseconds per allocation (8 threads oversubscribe 4 cores).
Phase 1 updated two process-wide `GC_STATS` atomics on every allocation.
Phase 2 removes them from the allocation path, which explains the multi-thread difference.
Chunk size has no measurable effect on throughput, so 64 KiB stays the default.
At 64 KiB, an idle isolate holds at most 128 KiB of free credit.

A dynamic-target variant replaced the per-heap soft limit with a target of twice the surviving bytes and a 32 MiB floor.
End-to-end interpreter runs (`cljrs run`, median of three) of that variant:

| Workload | Phase 1 | Dynamic targets | Collections (Phase 1 / dynamic) | Peak RSS (Phase 1 / dynamic) |
|---|---|---|---|---|
| `cljrs eval 1` | 0.06 s | 0.06 s | 0 / 0 | 12 MB / 9 MB |
| 1M two-element vectors | 1.53 s | 2.03 s | 0 / 3 | — |
| 400k `assoc` on a 1000-key map | 3.80 s | 4.83 s | 34 / 167 | 55 MB / 30 MB |
| 1.6M `assoc` on a 1000-key map | 18.5 s | 24.7 s | 138 / 353 | — |

With collection effectively disabled in both builds, the 400k-`assoc` run takes 3.3–3.6 s in Phase 1 and 3.2 s with chunked credit.
The mutator path therefore has no regression; the difference is collection policy.
Phase 1 without an explicit soft limit collected only near a third of physical memory.
Each collection marks the whole runtime, and more frequent collections also slow the mutator (more time in `malloc` and in persistent-map inserts between collections).

Floors from 1 MiB to 256 MiB were measured.
Floors below 32 MiB make small programs collect several times for little gain.
Larger floors did not speed up the `assoc` workload.
Phase 2 therefore keeps the Phase 1 collection triggers.
Phase 5 owns target selection from heap size and allocation rate; this result is its input.

### Phase 0: correct the current limit behavior

- Make the soft-only CLI option set the requested soft limit.
- Reject invalid soft and hard limit combinations.
- Document that the current hard limit is not enforced.
- Add a regression test that shows the current per-isolate multiplication.

This phase changes no allocation behavior.

### Phase 1: governor and observe-only accounting

- Add `ProcessMemoryGovernor`, `IsolateAccount`, and `IsolateControl`.
- Register the main runtime and each spawned isolate.
- Account for GC heap allocation and sweep.
- Add current and peak counters by isolate.
- Add pressure transitions and collection requests.
- Keep all allocation requests successful.

This phase measures policy before it rejects work.

### Phase 2: chunked allocation credit

- Add local credit to each isolate account.
- Refill credit from the process governor.
- Return excess credit after collection and shutdown.
- Replace per-isolate process-sized thresholds with dynamic collection targets (deferred to Phase 5).
- Benchmark credit chunk sizes.

This phase must keep the common path free of global synchronization.

### Phase 3: account for non-heap managed memory

- Charge region chunks and retired regions.
- Charge shared byte blobs and shared-atom storage.
- Add byte-bounded isolate channels.
- Charge queued serialized messages.
- Report static and code memory where the runtime knows their sizes.

This phase closes the largest budget bypasses.

### Phase 4: fallible allocation and hard admission

- Add `GcPtr::try_new` and `AllocError<T>`.
- Add fallible static and shared allocation operations.
- Add memory-limit variants to runtime error types.
- Add the pending runtime-fault path for compiled code.
- Migrate interpreter and builtin allocation sites.
- Migrate clone, async, JIT, and AOT allocation bridges.
- Reserve a fixed critical-allocation path.
- Enable strict hard-limit rejection after migration is complete.

This phase makes the hard limit enforceable for governed allocations.

### Phase 5: policy and platform pressure

- Replace per-isolate process-sized thresholds with collection targets selected from heap size and allocation rate (moved from Phase 2).
- Add rate limits for cross-isolate collection requests.
- Read cgroup and job-object limits where the platform provides them.
- Add optional RSS pressure as an emergency signal.
- Add load-shedding hooks for servers.

This phase improves behavior under external memory pressure.

## Test plan

### Unit tests

- Concurrent credit requests never commit more than the hard limit.
- A failed request leaves every counter unchanged.
- Charge drop returns its bytes once.
- Shared `Arc` clones do not create duplicate charges.
- Heap sweep returns the exact sum from freed headers.
- Region retirement keeps its charge.
- Region release returns its charge.
- Isolate shutdown returns used bytes and free credit.
- Invalid configuration returns a clear error.

### Integration tests

- Eight isolates share one hard limit without fixed partitions.
- One isolate can use idle capacity from the other seven isolates.
- `Yellow` requests collection from the requesting isolate.
- `Yellow` can request collection from the largest heap.
- `Red` rejects new governed allocation without a process abort.
- A byte-bounded channel applies sender backpressure.
- Dropping a channel releases all queued-message charges.
- Interpreter and compiled code report the same memory-limit error.
- Repeated isolate creation and shutdown has stable process memory.

### Stress tests

- Allocation and collection run on all isolate threads for ten minutes.
- Shared blobs cross isolates while other heaps collect.
- Queue producers race with receiver shutdown.
- Pressure repeatedly crosses `Green`, `Yellow`, and `Red`.
- Counter values never wrap or become negative.

### Performance tests

- Measure allocation throughput before and after the governor.
- Measure global atomic operations per allocated byte.
- Measure credit-refill rate for several chunk sizes.
- Measure tail latency during simultaneous isolate collection.
- Measure queue throughput with byte accounting enabled.

The common allocation benchmark target is less than a three percent regression.
The benchmark must also show no global atomic operation for each normal allocation.

## Rollout

Observe-only mode will be the default through Phase 3.
It will report the allocations that strict mode rejects.

Phase 4 will add an opt-in strict mode.
The test suite and soak tests must pass before strict mode becomes the default.

The deprecated GC environment variables will remain for one release cycle after strict mode becomes the default.

## Risks

### Incomplete accounting

Native libraries and Rust collections can allocate outside known governor paths.
RSS can therefore exceed the managed hard limit.

The documentation must call the value a managed-memory limit.
Platform pressure signals provide an emergency response for untracked memory.

### Temporary construction overshoot

A caller can build a large `Vec` before it wraps that value in a `GcPtr`.
The governor sees the size after this allocation exists.

Fallible constructors for large buffers will reserve before construction.
This work belongs in Phase 4.

### False collection pressure

Committed credit includes unused isolate capacity.
Large credit chunks can cause early `Yellow` transitions.

Metrics separate used bytes from free credit.
Benchmarks will select a chunk size that limits this error.

### Slow cooperative collection

An isolate can delay collection while native work runs without a safepoint.
The governor can stop new credit, but it cannot reclaim that heap immediately.

Worker-pool operations must not hold managed values.
Long native loops must add explicit safepoints or use a bounded native-memory reservation.

### Error-path allocation

A normal Clojure exception value requires managed memory.
That allocation can fail at the hard limit.

The runtime-fault path therefore carries fixed-size Rust data first.
It creates a Clojure value only after memory becomes available.

### Accounting contention

Small credit chunks cause frequent process-atomic operations.
Large chunks reserve too much unused memory.

The first implementation uses 64 KiB and measures both costs.

## Deferred decisions

The following decisions do not block Phases 0 through 4 (Phase 4 is implemented):

- The final Clojure representation of `MemoryLimitExceeded`
- Runtime-specific sub-budgets for embedding applications
- User-defined isolate weights or minimum guarantees
- A policy for intern-table growth
- Admission control for JIT code publication
- Limits for future shared persistent collections

## Completion criteria

Completion requires all these statements to be true:

1. One hard limit governs all registered isolate heaps.
2. Normal allocation uses no process-wide operation until local credit is empty.
3. Heap, region, shared-value, and queue bytes appear in current metrics.
4. Isolate channels cannot grow without a byte limit.
5. Strict mode rejects governed allocation before committed bytes exceed the allowed bound.
6. Interpreter, JIT, and AOT paths report the same runtime fault.
7. Isolate shutdown releases every private heap allocation and returns all credit.
8. Multi-isolate stress tests show no counter leaks or permanent credit loss.
9. The allocation throughput regression stays within the measured target.

## Relationship to immutable sharing

The governor does not require shared persistent collections.
It prepares the accounting model for them.

A future shared collection will own one `MemoryCharge` for each shared allocation block.
Arc clones will not change the process charge.

This rule keeps immutable sharing compatible with one process-wide memory budget.
