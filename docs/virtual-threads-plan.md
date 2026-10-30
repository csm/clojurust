# Virtual Threads Plan — Stackful Fibers per Isolate

## Why this document exists

Today, code that waits on something in clojurust has two choices, and neither is good:

- **Colored async.** `^:async` functions and `go` bodies run through a separate tree-walker
  (`cljrs-async/src/eval_async.rs`) and must `await` every channel op, future, and socket read.
  Ordinary functions — and every native HOF (`map`, `reduce`, `swap!`, …) — cannot suspend, so
  async-ness spreads through call graphs, and `deref` inside an `^:async` body is a runtime error.
- **Blocking the isolate.** Synchronous code calls `<!!`/`>!!` (a `Condvar` wait with a 1 ms
  re-poll, `cljrs-async/src/channel.rs`) or `deref`, which stalls the *one* OS thread the isolate
  owns. The `future-call` docstring has to warn that `(deref f)` "blocks the one thread the task
  needs in order to finish".

The JVM solved the same problem with virtual threads (Project Loom), and core.async now leans on
them: `io-thread` runs a body that "may do blocking I/O but must not do extended computation" on a
virtual thread when available, and recent core.async releases can run `go` blocks on virtual
threads without the IOC transform. This plan gives clojurust the equivalent: **lightweight
threads where blocking code parks the thread instead of blocking the carrier, with no async
syntax.**

Read `async-worker-pool-plan.md` (the isolate model) and `isolate-boundary-plan.md` first. This
plan lives *inside* one isolate and does not change the cross-isolate model.

---

## The decision: stackful fibers, M:1 per isolate

A **fiber** is a coroutine with its own machine stack. Parking a fiber saves its registers and
switches to another stack; everything above the park point — tree-walker frames, native builtins
calling back into Clojure, Cranelift JIT frames, AOT code — is suspended as-is.

Why stackful rather than another compiler transform:

1. **The interpreter is a recursive Rust tree-walker.** A CPS / state-machine transform can only
   suspend code it rewrites. It cannot reach through `reduce` calling a Clojure fn calling
   `slurp`, or through a JIT frame calling `rt_call`. That limit is exactly why `^:async` needs its
   own evaluator today, and why `TODO.md` Phase H ("emit Cranelift state machines for `^:async`")
   exists. Stack switching suspends all tiers uniformly and for free.
2. **GC roots are precise shadow stacks, not conservative stack scans.** `ENV_ROOTS`,
   `VALUE_ROOTS`, `OPTION_VALUE_ROOTS` (`cljrs-runtime/src/env/gc_roots.rs`) and `ALLOC_ROOTS`
   (`cljrs-gc`) record root addresses explicitly. A suspended fiber's stack never moves, so its
   recorded root pointers stay valid; the collector just needs to visit every fiber's root set
   rather than only the running one. No stack-scanning machinery is required.
3. **Fibers never migrate between OS threads.** `GcPtr` is honestly `!Send`, so a fiber is pinned
   to its isolate's thread. That is a constraint (no work stealing — parallelism still comes from
   isolates), but also a large simplification: Rust `thread_local!` addresses stay valid across a
   switch, and only the *logical* per-mutator state has to be swapped.

So the model is **M fibers : 1 isolate thread**, with N isolates for parallelism, unlike Loom's
M:N over a carrier pool.

### Integration with the existing executor

Each isolate already runs a Tokio `current_thread` runtime + `LocalSet`. A fiber is driven as an
ordinary `spawn_local` task:

```rust
// Sketch — names are provisional.
struct FiberTask { co: Coroutine<Resume, Park, FiberResult>, state: MutatorState, pending: Option<ParkFuture> }

impl Future for FiberTask {
    type Output = FiberResult;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<FiberResult> {
        // 1. If parked on a future, poll it; still Pending → return Pending.
        // 2. Swap this fiber's MutatorState into the thread-locals.
        // 3. co.resume(...) — runs Clojure until it parks or returns.
        // 4. Swap MutatorState back out.
        // 5. Park(fut) → store fut, poll it once with cx, return Pending.
        //    Return(v) → Poll::Ready(v).
    }
}
```

Inside a fiber, a blocking primitive calls `fiber::park_on(fut)`, which suspends the coroutine and
hands `fut` to the outer `poll`. This is the pattern Wasmtime uses (`wasmtime-fiber`) to run
synchronous wasm against async host functions. It reuses the existing reactor, timers,
`WorkerPool` offload, and every Tokio-based resource in `cljrs-net` without change.

Outside a fiber (`fiber::current().is_none()`), the same primitives keep today's behavior.

### Crate choice

Use **`corosensei`** (maintained, used by Wasmer; x86_64, aarch64, riscv64, arm, and Windows;
supports unwinding and backtraces through the coroutine boundary). `wasmtime-fiber` is the fallback.
Do not adopt `may` or another full green-thread runtime: we need our own scheduler on top of
the existing `LocalSet`, not a second executor.

Stacks are `mmap`-reserved with a guard page and committed lazily by the OS, so a large
reservation is cheap in physical memory. The tree-walker is stack-hungry (macro-heavy loading
already needs 16 MB test threads), so the default reservation is configurable
(`CLJRS_FIBER_STACK_KB`, default 1 MB for `io-thread`/`go`; the root fiber uses the main
thread's size). Stacks are pooled and reused.

---

## What fights it (must be migrated)

### 1. Per-mutator thread-local state

About 30 `thread_local!` sites exist. Every one that holds the state of *a running computation*
(as opposed to the isolate as a whole) must become per-fiber. Audit list:

| State | Location | Per-fiber? | Notes |
|---|---|---|---|
| `ENV_ROOTS`, `VALUE_ROOTS`, `OPTION_VALUE_ROOTS` | `cljrs-runtime/src/env/gc_roots.rs` | **yes** | LIFO guards; interleaving fibers on one stack corrupts them |
| `ALLOC_ROOTS` | `cljrs-gc/src/lib.rs` | **yes** | same |
| `REGION_STACK` | `cljrs-gc/src/region.rs` | **yes** | regions are LIFO bump scopes; must never interleave |
| `RETIRED_REGIONS` | `cljrs-gc/src/region.rs` | no | isolate-wide root set |
| `ALLOC_CTX` | `cljrs-gc/src/alloc_ctx.rs` (`no-gc`) | **yes** | scratch/static/invocation context stack |
| `BINDING_STACK` | `cljrs-runtime/src/env/dynamics.rs` | **yes** | dynamic `binding`; new fibers get conveyed frames like `future` |
| `EVAL_CONTEXT` | `cljrs-runtime/src/env/callback.rs` | **yes** | includes `current_is_async` |
| `CALL_DEPTH` | `cljrs-runtime/src/env/depth.rs` | **yes** | limit scaled to the fiber's stack size |
| `ACTIVE`/`EXHAUSTED`/`INTERRUPTS` (gas) | `cljrs-runtime/src/env/gas.rs` | **yes** | gas meters are per computation |
| `TRANSACTION_DEPTH` / `TRANSACTION_GENSYM` | `cljrs-runtime/src/env/policy.rs` | **yes** | STM transaction nesting |
| `OUTPUT_CAPTURE`, `BIG_DECIMAL_SCALE` | `cljrs-runtime/src/builtins/builtins.rs` | **yes** | `with-out-str`, `with-precision` |
| `WATCH_ERROR` | `cljrs-runtime/src/interp/apply.rs` | **yes** | |
| `IR_LOWERING_ACTIVE` | `cljrs-runtime/src/tiered/apply.rs` | **yes** | |
| `MY_FRAMES` (JIT active frames) | `cljrs-runtime/src/tiered/jit_state.rs` | **yes** | see GC/JIT below |
| `PENDING_EXCEPTION` | `cljrs-compiler/src/rt_abi.rs` | **yes** | thrown-value slot between native frames |
| `CRITICAL_DEPTH`, `FAULT` | `cljrs-gc/src/governor.rs` | review | critical sections must not park (assert) |
| `ISOLATE_HEAP`, `CURRENT` account, `ISOLATE_CANCELLATION` | `cljrs-gc` | no | isolate-wide |
| `TAP`, `ASYNC_DRIVER`, IC tracer flag, wasm DOM state | various | no | isolate/process-wide |

Approach: gather the per-fiber rows into one `MutatorState` struct owned by each fiber, and
have the scheduler swap it in and out around `resume`. For `Vec`-backed stacks a swap is a
pointer exchange (`std::mem::swap` on the vectors), so a context switch stays O(number of
fields), not O(depth). The root fiber (main thread / REPL eval) owns the state that exists today,
so code that never spawns a fiber behaves as before.

A unit test enumerates the `thread_local!` sites (a `grep`-driven list checked in CI) so a new
thread-local added later must be classified as per-fiber or per-isolate.

### 2. Locks and borrows held across a park

A fiber that parks while holding a `std::sync::Mutex` guard (namespace, protocol table, var
roots, channel internals) or an active `RefCell` borrow from `X.with(|x| …)` will deadlock or
panic the *next fiber on the same thread*. This is stricter than Loom's "pinning", which only
loses concurrency.

- Runtime locks must be held only for short critical sections, never around evaluation of user
  code or a blocking primitive. Audit `Namespace`, `Multimethod`, protocol registry, `Var`,
  `LazySeq` realization (realizing a lazy seq runs user code — it must not hold its `Mutex` while
  doing so; use a "realizing" state + parked waiters instead).
- `locking` / `monitor-enter` gets a fiber-aware monitor (owner = fiber id, waiters parked).
- Debug builds keep a per-thread "guards held" counter, incremented by a wrapper around the
  runtime's lock types; `park_on` asserts it is zero.

### 3. GC root tracing across fibers

`GcHeap::collect` (and `async_gc_collect`) traces the current thread's root stacks. It must also
trace every suspended fiber's `MutatorState`: root stacks, binding frames, eval contexts,
retained region roots, and the fiber's pending result. The isolate keeps a registry of live
fibers; collection walks it. A fiber that has finished drops out of the registry before its stack
returns to the pool.

Safepoints are unchanged: collection only happens on the running fiber at a safepoint, and
suspended fibers are by construction stopped at a park point, which is a safepoint.

### 4. JIT epoch reclamation

`code_cache::reclaim_at_stw` frees stale code epochs with no live JIT frame, using per-thread
active-frame tracking. With fibers, a suspended fiber may hold JIT frames from an old epoch, so
`live_epochs` must union over all fibers in the isolate. This rides on making `MY_FRAMES`
per-fiber.

---

## Parking-aware primitives

Each primitive checks `fiber::current()`: inside a fiber it parks; outside it behaves as today.

| Primitive | In a fiber | Notes |
|---|---|---|
| `<!!`, `>!!`, `alts!!` | park on the channel's take/put future | removes the 1 ms Condvar re-poll for fiber callers |
| `deref` / `@` of future, promise | park until settled | fixes the "deref blocks the thread the task needs" deadlock |
| `deref` of delay / lazy-seq being realized by another fiber | park on the realizer | see §2 |
| `deref` with timeout | park on `select(settle, sleep)` | |
| `Thread/sleep`, `sleep` | park on `tokio::time::sleep` | |
| `cljrs-net` sockets (TCP, UDP, Unix, TLS, QUIC, h2/h3) | park on the existing Tokio futures | sync-looking wrappers become possible |
| `cljrs-io` file I/O, `slurp`/`spit` | park on `WorkerPool::offload` | bytes cross the pool, values are built on the isolate |
| process wait / pipes (`cljrs-stdlib/src/process.rs`) | park on offloaded wait | |
| `locking`, STM `ref` retry waits, agent `await` | park on fiber-aware wait queues | |
| native interop that blocks inside Rust | **pins the isolate** | see below |

**Pinning.** A Rust function that blocks the OS thread (a `std::net` call in a user crate, a
blocking FFI call) stalls every fiber on the isolate. Mitigations:

- `(blocking & calls)` / `cljrs.fiber/blocking` offloads typed native call bodies on `WorkerPool` and parks the calling fiber.
  Arguments and results cross only after conversion to Rust types that implement `Send + 'static`.
  The form rejects arbitrary Clojure closures and unsupported native adapters.
  `#[cljrs::export(blocking)]` offloads an exported Rust function automatically without the Clojure form.
- A stall detector: the GC-service task notes when the isolate hasn't reached a safepoint for
  more than `CLJRS_FIBER_STALL_MS` (default 100 ms) while fibers are runnable, and logs the
  offending native frame once. This is the equivalent of Loom's `jdk.tracePinnedThreads`.

---

## Clojure surface

```clojure
(require '[clojure.core.async :as a])

;; io-thread: blocking I/O allowed, extended computation discouraged (core.async contract)
(a/io-thread
  (let [body (slurp "data.edn")]          ; parks, does not block the isolate
    (a/>!! out (read-string body))))      ; parks

;; plain blocking style in any fiber — no ^:async, no await
(defn fetch-all [urls]
  (->> urls
       (map #(a/io-thread (http/get %)))  ; one fiber per request
       (mapv a/<!!)))                     ; parks on each result
```

- **`io-thread`** (new, core.async-compatible): spawn a fiber running `body`, return a channel
  that receives its result. Exceptions are delivered as with `thread`.
- **`thread` / `thread-call`**: become fiber-backed. A truly parallel `thread` would need to run
  closures on another isolate, which waits on the deferred `pfuture` / B3 shared code arena. Until
  then, `thread` documents that it is concurrent, not parallel.
- **`future` / `future-call`**: fiber-backed, so `@(future …)` parks instead of deadlocking.
- **Root fiber**: `cljrs run`, the REPL, nREPL evals, `cljrs test`, and the AOT `main` preamble
  run user code inside a root fiber, so top-level code may block-park freely.
- **`go`**: in Phase 5, `go` may run its body directly on a fiber with `<!` ≡ `<!!`, matching
  core.async's virtual-thread mode. `^:async`/`await` remain supported and become sugar
  (`await` = park) rather than a second evaluator.
- **Introspection**: `(cljrs.fiber/current)`, `(cljrs.fiber/count)`, fiber name for
  diagnostics; fiber counts and stack bytes in `--gc-stats`.

`.cljc` code written for JVM core.async (`io-thread`, `<!!`, `thread`) runs unchanged under
`#?(:cljrs …)`-free code paths.

---

## Scheduling and fairness

- **Run queue.** Runnable fibers are just ready `LocalSet` tasks; Tokio's local scheduler is the
  run queue. No separate scheduler data structure in Phase 1.
- **Cooperative preemption.** The interpreter already reaches safepoints at calls and loop
  back-edges (GC safepoints, `gas` checks). Add a time-slice check there: if the current fiber
  has run longer than `CLJRS_FIBER_SLICE_US` (default 10 ms) and other fibers are runnable,
  `park_on(yield_now())`. Cranelift-compiled code already emits `rt_safepoint` polls (`cljrs-compiler/src/codegen.rs`), so this
  covers all tiers. This keeps a CPU-heavy fiber from starving `io-thread` fibers, which a pure
  cooperative model would not.
- **Priorities** are out of scope.

---

## Stack overflow

- Each fiber's `CALL_DEPTH` limit is derived from its stack reservation, so deep recursion
  produces the normal clojurust stack-overflow error before hitting the guard page.
- A guard-page hit (deep native recursion) is fatal in Phase 1, as on the main thread today.
  Phase 4 investigates a `sigaltstack` handler that reports which fiber overflowed.

---

## Platform scope

- **Supported:** Linux, macOS, Windows on x86_64 and aarch64 (corosensei targets).
- **wasm32:** no stack switching until the wasm stack-switching proposal or JSPI is usable.
  `cljrs-wasm` keeps the current colored model. `io-thread`/`thread`/`future` there fall back to
  LocalSet tasks, and the parking primitives keep today's in-`^:async` behavior. Code is gated with
  `#[cfg(not(target_arch = "wasm32"))]`.
- **`no-gc` build:** works; `ALLOC_CTX` becomes per-fiber, and each fiber's scratch regions are
  released when it finishes.

---

## Phases

### Phase 0 — `MutatorState` consolidation (no behavior change)

- Move every per-fiber row in the table above behind one `MutatorState` with
  `save() -> MutatorState` / `restore(MutatorState)`.
- Add the thread-local classification test.
- Add the debug "guards held" counter on runtime lock wrappers.
- **Exit:** full test suite green; `save`/`restore` round-trips under a test that swaps state
  between two hand-built evaluations at a safepoint.

**Status: implemented.**

- `cljrs_gc::mutator` holds the registry. Each per-fiber thread-local stays where it is and is
  registered at link time with `cljrs_gc::mutator_slot!` (via `inventory`), so lower crates
  need no dependency on the crates above them. `MutatorState` holds one value per slot.
  `save`/`restore`/`swap_with_current` call `mem::swap` on each one, and a slot whose `RefCell`
  is borrowed panics instead of swapping.
- `MY_FRAMES` originally swapped the contents of the registered frame stack. Phase 1 now
  swaps the registered stack identity, so `live_epochs` also sees suspended fibers.
- `PENDING_EXCEPTION` and `RT_REGION_STACK` (`cljrs-compiler`) and `POISON_WATERMARK`
  (`cljrs-gc`, a depth into `REGION_STACK`) are per-fiber too. The audit table above missed
  them.
- `crates/cljrs/tests/thread_local_classification.rs` is the classification test. It scans
  `crates/*/src` and also checks that every per-fiber row is a registered slot.
- `TrackedMutex` replaces `std::sync::Mutex` on the lock fields of `Var`, `Atom`, `Namespace`,
  `MultiFn`, `Protocol`, `LazySeq`, `Delay`, `Volatile`, and on agent watches.
  `mutator::assert_parkable` checks two things: no counted guard is held, and the thread is not
  in a governor critical section.
- `crates/cljrs-runtime/tests/mutator_state.rs` is the exit test. It parks evaluation A inside
  a native fn and runs evaluation B on the same thread. Bindings, `with-out-str` capture and
  eval context stay separate.
- Phase 1 adds tracing for saved states. Their root addresses remain valid until the
  scheduler restores the state and unwinds the owning stack.

### Phase 1 — Fiber core (`cljrs-async::fiber`)

- `corosensei`-based `Fiber`, stack pool, `FiberTask: Future`, `fiber::spawn`, `fiber::park_on`,
  `fiber::current`.
- Per-isolate fiber registry; GC traces all suspended fibers; JIT `live_epochs` unions over
  fibers.
- Panics caught at the fiber boundary and delivered as `EvalError`.
- **Exit:** GC stress test — 5,000 fibers interleaving allocation-heavy code with a
  collection forced at every park and regions active, in both GC and `no-gc` builds, plus an
  AddressSanitizer run on Linux CI.

**Status: implemented; Linux AddressSanitizer validation awaits CI.**

- `cljrs_async::fiber` provides `Fiber`, `FiberTask`, `spawn`, `spawn_rooted`, `park_on`, and
  `current` on native targets. Tokio's `LocalSet` drives each fiber on its isolate thread.
- `park_on` keeps the future pinned on the fiber stack. The future can borrow local values.
  The driver polls it with the fiber's mutator state installed.
- Stacks default to 1 MB. `CLJRS_FIBER_STACK_KB` accepts at least 64 KB. Each isolate pools
  up to 32 finished stacks. Root entry points remain unchanged until Phase 3.
- Each `MutatorState` registers weakly with its isolate. Slot callbacks trace saved shadow
  roots, allocation roots, regions, bindings, eval contexts, watch errors, and pending exceptions.
  Collection also sees the saved carrier state during a fiber resume.
- Each JIT frame stack retains its registration while saved. `live_epochs` includes suspended
  fibers across all isolates.
- Panics from fiber bodies or parked futures become `EvalError::Runtime`. Cancellation restores
  the fiber state and unwinds the stack before pooling it.
- `spawn_rooted` retains captured values before the first poll. Completed `spawn` results stay
  rooted in `FiberResult` until the caller consumes or drops them. Rust callers must root managed
  values held by opaque futures and closures. GC builds use the existing region poison fallback
  for opaque captures. In `no-gc` builds, captures must come from a static allocation context.
- `tests/fiber.rs` covers 5,000 fibers with allocation-heavy workloads, active regions, and
  collection at every park. It also checks borrowed waits, cancellation, panic delivery, JIT
  epochs, queued results, carrier roots, bindings, and output capture.
- Local stress checks and macOS AddressSanitizer checks pass in GC and `no-gc` builds.
  The macOS checks use `-Cllvm-args=-asan-globals=0` to avoid the Apple linker error on
  inventory constructors. `.github/workflows/ci.yml` adds Linux AddressSanitizer jobs for both
  modes with corosensei's sanitizer hooks enabled.

### Phase 2 — Parking primitives

- `<!!`, `>!!`, `alts!!`, `deref` (future/promise/delay/timeout), `sleep`, `locking`.
- `cljrs-net` and `cljrs-io` parking paths; `blocking` escape hatch;
  `#[cljrs::export(blocking)]`.
- Lazy-seq / delay realization without holding the realization lock across user code.
- **Exit:** a 10k-connection echo server written in plain blocking style on one isolate.

**Status: implemented, including the restricted native-call `blocking` form.**

- `cljrs_value::parking` provides executor-independent hooks and notification events.
  Native fibers install the hooks once. Core crates do not depend on Tokio.
- `<!!` and `>!!` park on the existing channel futures. `alts!!` supports takes, put alternatives, `:default`, and `:priority`.
  Shared selection tokens commit one operation, including rendezvous handoffs between selectors.
  Losing operations do not transfer values. Cancellation removes their registrations.
  Cancellation also restores an unconsumed committed take in FIFO order.
  Ordered waiter queues preserve pending-put order and wake only the selected tasks.
- Future and promise dereferences share notification waits across the builtin, reader dereference,
  and synchronous `await` paths. Timed waits use a deadline and preserve settled errors.
- `sleep` and `Thread/sleep` park on Tokio timers. The `locking` macro uses reentrant monitors
  whose owners distinguish fibers on the same OS thread. Final guard/waiter destruction removes the registry entry without a table scan.
- Delay and lazy sequence states distinguish pending, realizing, settled, and failed computations.
  The realizing state retains the traced thunk. Competing fibers wait without a realization lock.
  Cancellation restores the original pending thunk and wakes waiters to retry. Normal thunk errors remain cached.
  Monitor acquisition, delay forcing, and `try_realize` reject carrier waits on same-thread fibers.
  Equality, hashing, and formatting use a private realization error signal.
  Evaluation and boxed ABI boundaries convert this signal to catchable Clojure errors.
  The IR interpreter uses one boundary per function execution.
  Lexical scope guards remove frames on destructuring errors and unwinds.
  Ordinary panics and fiber cancellation remain distinct.
- Core `slurp` and `spit` offload owned file work to the blocking worker pool.
  The existing channel interfaces provide parking paths for `cljrs-io` and `cljrs-net`.
  Stdlib reader/writer operations and process waits offload owned host data and park their calling fiber.
- Rust `parking::blocking` and `#[cljrs_interop::export(blocking)]` offload owned, `Send` work.
  Argument conversion and result conversion stay on the isolate. Variadic managed-value exports
  cannot use the attribute.
- Rendezvous cancellation retracts an unconsumed offer. Blocking and async channel callers
  wake each other through the same selection registrations.
- `cljrs-async/tests/parking.rs` covers contention, timeouts, errors, GC during realization,
  cancellation, monitor ownership, worker execution, and blocking fallback in GC and `no-gc` builds.
  The tests pass locally under AddressSanitizer in GC and `no-gc` builds, through both interpreter modes.
  Linux ASan CI includes these tests. `cljrs-interop/tests/blocking_export.rs` covers export conversion.
- `cljrs-net/tests/fiber_echo.rs` provides a normal 128-connection test and an explicit
  10,000-connection stress test. Every connection uses a plain Clojure blocking-style handler.
  The stress test requires a file-descriptor limit of at least 20,000.
  Both cases pass locally on macOS. The 10,000 connections remain open together before any client sends data.

- The five direct realization paths in the compiler ABI use `try_realize`.
  Count, first, rest, apply, and fused into-map operations report realization errors through the pending-exception slot.
  Keyword argument conversion uses the same error path.
- `value_to_seq_vec` returns `ValueResult<Vec<Value>>` and roots its source during realization.
  Destructuring, variadic recur, syntax quote, apply, async helpers, and string joining propagate its errors.
  The conversion walks nested lazy sequences iteratively.
- Carrier regressions cover sequences with parked realizers in both interpreter modes.
  A subprocess exercises sequence, equality, hash, and print ABI paths and checks that rejected waits preserve the thunk.
  These regressions pass locally in GC and `no-gc` builds.
  An AOT regression checks catchable realization errors in a compiled binary and passes locally in a GC build.

- Direct `monitor-enter` and `monitor-exit` functions retain explicit acquisitions in per-fiber state.
  The collector traces their objects. Fiber completion and cancellation release unmatched acquisitions.
  Legacy async tasks reject explicit monitors because those tasks share the root mutator.
- Scalar transient keys use in-place updates and direct lookups without snapshots.
  Other transient updates and object-array hashing use traced snapshots outside collection locks.
  Snapshots use a per-mutator shadow stack without managed allocation.
  Object-array hashing copies its vector once. Transient-vector hashing shares its existing storage.
  Concurrent transient changes cause an update to retry. Realization errors do not mutate or poison these collections.
- `tests/fiber_io.rs` checks isolate progress during blocked file operations and process waits with FIFO gates.

- `(blocking & calls)` and `cljrs.fiber/blocking` offload typed native calls, one call at a time.
  Each body form must call a native function with an owned-work adapter.
  Argument evaluation and `FromValue` conversion stay on the isolate.
  The worker receives only Rust inputs and outputs that implement `Send + 'static`.
  `IntoValue` conversion runs on the isolate after the wait.
- Fixed-arity `#[export]` functions provide these adapters automatically.
  An unsupported input or output type returns an error before the Rust body runs.
  Variadic managed-value exports and Clojure closures are rejected.
  This restricted form supplies the native escape hatch without moving Clojure closures or fibers between isolates.
- `NativeFn::with_blocking` supports explicit adapters in Rust embedders.
  Ordinary calls retain their original behavior. `#[export(blocking)]` still offloads automatically without the Clojure form.
  Outside a fiber, calls run synchronously. Cancellation stops the wait but does not interrupt an active host operation.
- `tests/blocking_form.rs` covers worker liveness, argument evaluation, errors, panics, unsupported captures, and cancellation in both interpreter modes.
  `blocking_export.rs` checks that conversions stay on the caller thread and rejects unsupported inputs and outputs before execution.
  An AOT fixture compiles and runs both macro spellings with typed native exports.

The Phase 2 native escape hatch is complete under this restricted contract.
Arbitrary Clojure closure offload remains outside this phase because it requires a separate cross-isolate code and capture design.

### Phase 3 — Clojure surface

- `io-thread`; fiber-backed `thread`, `thread-call`, `future`.
- Root fiber for `run`, REPL, nREPL, `test`, AOT `main`.
- Binding conveyance into new fibers.
- Docs in `docs/book`, `cljrs-async/README.md` updated.
- **Exit:** core.async's `io-thread` examples run unchanged; `@(future …)` at top level and
  inside `future` bodies no longer deadlocks.

**Status: implemented.**

- Native `future` and `future-call` run ordinary functions in fibers.
  The full future family now lives in `clojure.core`. Experimental names remain compatible.
- `thread`, `thread-call`, and `io-thread` convey dynamic bindings and return result channels.
  A non-nil result enters the channel before closure. Nil results only close the channel.
  Exceptions print a diagnostic before closure. Each call uses one body fiber.
- Children receive binding snapshots, active gas meters, and interrupt flags.
  Binding changes stay private to each child.
- `fiber::run_root` drives top-level evaluation on a 64 MiB stack.
  It transfers and restores host mutator state, including output capture and nREPL bindings.
  A separate carrier state retains legacy async roots between drives and during executor destruction.
- CLI `run`, `eval`, REPL, nREPL, and tests use root evaluation.
  Generated AOT program and test harnesses also use root fibers.
  The AOT preamble, namespace initializers, compiled entry, and `-main` can park.
- Macro expansion traverses reader dereference, so `@(future ...)` compiles correctly.
  Compiled closure adapters propagate pending exceptions. Future dereference preserves thrown data.
  Native closure adapters explicitly trace managed captures through `NativeFn::captures`.
- `no-gc` scratch scopes register their live regions through tail evaluation.
  Task captures lease scratch scopes until completion or cancellation.
  Regional publications trace actual backing scopes. Static publications copy proven finite immutable data without realizing lazy sequences.
  Results transfer scope ownership to their destination. Completed transient results can then release their storage.
  Monitor entry does not retire no-gc scratch scopes. Ordinary scopes allocate lease metadata only when shared.
  Bounded isolated allocation invocations cannot spawn Clojure fibers.
- `fiber_surface.rs` covers nested futures, ordinary function waits, bindings, result-channel closure,
  cancellation, errors, forced collection, and host state restoration in both interpreter modes.
  The tests pass in GC and `no-gc` builds.
  `fiber_entrypoints.rs` covers CLI and nREPL evaluation, including streamed output and session bindings.
  AOT regressions cover the compiled surface and generated test harness.
- The book and affected crate READMEs document the native surface.
  `go`, `^:async`, and wasm32 retain the legacy async evaluator until Phase 5.

Review fixes preserve resource-limit errors in a separate per-fiber slot.
Compiled catch clauses cannot consume gas exhaustion or memory faults.
Synchronous embeddings can borrow their Env and Form through `run_root`.
`Isolate::spawn_root` provides a synchronous isolate entry point.
Pending local futures reject carrier dereference instead of deadlocking.
Native embedders declare managed captures with `NativeFn::with_captures`.

### Phase 4 — Fairness and observability

- Time-slice preemption at safepoints.
- Stall detector; fiber counts and stack bytes in `--gc-stats` and the governor snapshot
  (fiber stacks charged as a new `MemoryClass::FiberStack`, so the governor can refuse new fibers
  under pressure).
- Fiber-aware overflow reporting.

### Phase 5 — Converge async models

- `go` bodies on fibers; `<!`/`>!` become parks.
- `await` = park; `^:async` becomes a no-op marker kept for compatibility; `eval_async` retired
  once the async test corpus passes on fibers.
- Re-evaluate `TODO.md` Phase H (async JIT state machines); fibers make it unnecessary for
  correctness and likely for performance.

---

## Costs and risks

- **Memory per fiber.** Tree-walker frames are large; expect tens of KB committed per idle
  `io-thread` fiber, more than a Loom virtual thread. JIT/AOT tiers reduce it. Measure in Phase 1
  and set the default stack reservation from data.
- **Context-switch cost.** Register save/restore (~tens of ns) plus the `MutatorState` swap.
  Keep `MutatorState` to pointer-sized swaps; never copy root stacks.
- **Unsafe surface.** Stack switching interacts with unwinding, TLS, debuggers, and sanitizers.
  Confine all of it to `cljrs-async::fiber`; nothing else touches corosensei.
- **Hidden thread-local state** added in future code. Mitigated by the classification test.
- **Lock-across-park bugs** are the most likely source of real-world hangs. Mitigated by the debug
  guard counter and by making realization/monitor waits fiber-aware.
- **Two async models during transition** (Phases 1–4). Accepted. Phase 5 removes the duplication.

## Non-goals

- Moving fibers between isolates or OS threads (forbidden by `!Send`).
- Shared-heap multi-threading. Parallelism stays isolate-based.
- Preemption by signal (Go-style asynchronous preemption). Safepoint preemption is enough
  because every tier already polls safepoints.

The Phase 3 review regressions cover dynamic return allocation, lazy and infinite sequence publication,
large transient task results, monitor entry, dynamic-binding updates, gas-only exhaustion signals, and finally-only exceptions.
Generated AOT resource failures return exit status 1 after scope cleanup and GC-statistics output.
Executor teardown uses the shared carrier-state helpers.

Third-pass review regressions cover all callback bridges, uncaught AOT top-level throws,
failed channel publication, static publication of finite data, and cycles between scope owners.
Root-isolate failures use a thread-safe result type. Thread result channels preserve resource-limit errors.
Ownership edges are weak. Cleanup follows affected paths and stops at live owners.

Fourth-pass review regressions cover top-level accumulators, replacement of opaque cell contents,
consumer aliases, failed CAS publication, stale weak edges, and fused fallback failures.
Thread result channels preserve typed resource failures. Unread failures report at channel destruction or executor teardown.
Buffered takes avoid a second publication scan.
Recur values use trampoline leases. Permanent atom and volatile contents use replaceable leases.
Program-lifetime opaque publications without a replaceable owner still retain their actual backing scopes.

Fifth-pass review regressions cover returned update aliases, cached reads without value traversal,
repeated top-level reads and Var rebindings, and release of consumed opaque channel values.
Compiled atom updates share interpreter validators and watches. Native closures share one capture allocation.
Handled thread resource failures produce no warning. Unread failures report once, including after collection.


Sixth-pass review regressions cover replaceable regional ownership, auxiliary atom slots, and publication identity.
Read scopes end with each call or iteration. Stores retain ownership independently of these scopes.
Global Var reads reuse cached leases without allocating a snapshot for every lookup.
Declared task captures exclude unrelated read leases. Executor report registries stay with their carrier and fibers.
Compiled atom callback errors use the shared conversion helper, including runtime memory-fault confirmation.
Publication preserves identity. Small immutable scalars use compact arenas with stable logical identities.
Other live payloads can retain unrelated allocations in their source arena until their last owner releases it.


Seventh-pass review fixes share result publication across definition errors, returns, and recur.
Body statements share their enclosing scratch scope; discarded source forms use a dedicated evaluation entry point.
Arrays, transients, lazy cells, delays, promises, futures, multimethods, and dynamic bindings release replaced field ownership.
Async tasks retain their own read contexts across polls. Unscoped Rust readers use owning snapshots or explicit read scopes.
Root handoff restores caller ownership before releasing the computation state.
Small immutable scalar relocation preserves Clojure identity, including compiled identity checks.
CI fixes cover the newer async error-size lint and diagnostic logging of isolate records.

Eighth-pass review fixes protect array-update aliases and cover identity for all boxed variants.
Compact scalar storage uses representation-sized chunks and reuses existing compact copies.
Transient conversion shares its initial collection. Writes use the collection's existing insertion IDs for ownership slots.
Managed persistence borrows its containing region; Rust stack persistence protects its result graph.
Protocol and multimethod ownership caches avoid formatted keys during dispatch.
Definition data and metadata use compact immutable storage. Function and mutable-cell roots use separate small regions.
Durable regional dependencies use counted edges. Raw program-lifetime publication is explicit.
IR and AOT scans scope their function reads. Test-runner reads end with each namespace.

Standalone macro expansion and IR lowering use temporary storage. Compiled constant caches keep permanent values outside bounded invocations.
First cache fills inside an invocation remain invocation-local. Cached compiled protocol calls borrow implementation ownership.
Exception tracing retains data held in other regions.
