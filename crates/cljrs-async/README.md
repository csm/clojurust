# cljrs-async

Async support for clojurust — `clojure.core.async` implemented via Tokio.

## Purpose

Provides CSP-style concurrency (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) and
the `^:async` / `await` function model, backed by a Tokio `current_thread` + `LocalSet`
executor. All Clojure values remain on a single thread, keeping GC pointers (`!Send`) safe.

## Status

**Phase B1 (per-isolate heaps)** — complete. `Isolate` gives each OS thread its own GC heap,
`current_thread` Tokio runtime, and `LocalSet`. Collections are fully independent with no
cross-isolate coordination. `GcPtr`'s `!Send` bound makes sharing across isolates a compile error.

Done (Phases A–H, A2, B1):

- Phase A: `init()` registers the async runtime hook with the interpreter.
- Phase B: `^:async` fn dispatch via the `AsyncRuntime` hook; `eval_async` tree-walker;
  cooperative `await` of futures/promises.
- Phase C: `deref`/`@` of a future inside an `^:async` body is a runtime error that steers
  callers to `await` (enforced in `cljrs_runtime::builtins` and `cljrs_runtime::interp` via the
  `cljrs_runtime::env::callback::current_is_async` context flag).
- Phase D: `timeout`, `alts`, and the `alt` macro, in a `clojure.core.async` namespace built
  at `init` time. `timeout` and `alts` are native fns that return a `Value::Future`; `alt` is
  a Clojure macro that `await`s `alts` and dispatches to the matching handler.
- Phase E: `chan`, `take!`, `put!`, `close!`, `poll!`, `offer!`, `async-spawn`, and the `go`
  macro. Channels are `CljChannel` `NativeObject`s (buffered or unbuffered/rendezvous).
- Phase F: `join-all` awaits a seq of futures and returns a vector of results. `thread-call`
  runs a thunk and delivers its result to a buffered channel. `onto-chan!` seeds a channel from
  a collection and closes it; `to-chan!` does the same but returns the channel before seeding
  finishes (background task). `mult` broadcasts a source channel to all registered tap channels
  (`tap!`/`untap!`/`untap-all!`). Clojure-level: `async-pmap`, `thread` macro, `merge`,
  `reduce`, `into`. `eval_loop_async` and `eval_recur_async` enable proper `await` yielding
  inside `loop`/`recur`, including in a `recur` *argument*.
- Phase G: GC safepoints at async yield points via `cljrs_runtime::env::gc_roots::async_gc_collect()`,
  called before each `yield_now().await` in `await_value`. Background GC-service task spawned
  by `init()`. Explicit GC root guards for `task_future` in `spawn_future`, callee/env in
  `run_async_fn`, and awaited futures/promises in `await_value`. `eval_loop_async` runs a
  GC safepoint per iteration and scopes each iteration in an alloc frame, released when
  the iteration recurs without having yielded, so dead intermediates are collectable.
- Phase A2: `WorkerPool` singleton — multi-thread Tokio runtime for `Send` byte-level tasks
  (`WorkerPool::global()`, `offload`, `handle`). Pool tasks carry only `Vec<u8>`, `String`, and
  `Send` channel types; `GcPtr`/`Value` construction is confined to LocalSet bridge tasks.
  wasm32 stub runs futures locally.
- Phase B1: `Isolate` type — per-isolate OS thread with independent GC heap (`ISOLATE_HEAP`
  thread-local), `current_thread` Tokio runtime, and `LocalSet`. GC collections are fully
  parallel and independent; no cross-isolate STW coordination.
- Phase B2: `isolate_channel` — cross-isolate copy boundary. `IsolateSender::send` serializes
  a `Value` to a `Send + Sync` wire form (`SerializedValue`, defined in `cljrs-value::clone`);
  `IsolateReceiver::recv`/`try_recv` deserializes it into the receiver's GC heap. Non-shareable
  values (mutable state, closures, native resources) are rejected at send time with a typed
  `CloneError`. Phase B3 (issue #171): a `Var` with a *promotable* (or unbound) root crosses by
  sharing its `shared_root` cell, so a value `def`'d in one isolate is observable by value in
  another; a var bound to a non-promotable root (closure/resource) stays isolate-local and is
  rejected like any other closure. The sender may be cloned (multi-producer); the receiver must stay on its isolate
  thread (single-consumer). `GcPtr`'s `!Send` bound makes pointer-sharing a compile error —
  the channel is the only sanctioned crossing point. Each accepted send is
  **metered** into the process-global `cljrs_gc::GC_STATS` via
  `record_boundary_crossing` (estimated bytes copied + serialize time), so a
  silent fan-out deep-copy is observable in `--gc-stats` rather than mystery
  latency — one of the four visibility guarantees in
  `docs/isolate-boundary-plan.md`. Rejected (non-shareable) sends copy nothing
  and are not metered.
- Managed-memory governor Phase 3 (`docs/managed-memory-governor-plan.md`): the
  channel is **byte-bounded**. Each queued message holds a governor
  `MemoryCharge` (`MemoryClass::MessageQueue`) for its
  `SerializedValue::wire_size`, taken before enqueue and returned after the
  receiver deserializes it. `send`/`try_send` return `CloneError::Full` when the
  message does not fit; `send_async` waits for room (recorded as a queue wait in
  the governor snapshot). Phase 4: when the strict governor refuses the queue
  charge, every send returns `CloneError::MemoryLimitExceeded` at once (the
  governor has no capacity wakeup to wait on), and `isolate-put!` raises the
  uncatchable `ValueError::MemoryLimitExceeded`. The default limit is the governor's `queue_limit`
  (`CLJRS_ISOLATE_QUEUE_LIMIT_MB`); `isolate_channel_with_limit` sets one per
  channel. Dropping or closing the receiver releases every queued charge.
- Phase B2 (Clojure surface): `isolate_builtins` exposes the copy boundary to
  Clojure as a **distinct constructor** (`(isolate-chan) → [tx rx]`), honoring
  the plan's "distinct-at-construction, not per-message" rule — you know a send
  copies because the *target* you hold is an isolate-channel end, not an
  in-isolate `(chan)`. `(isolate-put! tx v)` deep-copies `v` across the boundary
  (returns `true`, or `false` if the receiver is gone) and raises a **located**
  error at the put site if `v` cannot cross. `(isolate-poll! rx)` is a
  non-blocking take; `(isolate-take! rx)` returns a `Future` for use with
  `await`. The *distinct parallel primitive* (`pfuture`/`spawn`) that would run
  Clojure on another isolate is deferred — it needs B3's shared code arena so a
  worker can see the running program's code without copying the world.
- Phase H: `<!!` (blocking take) and `>!!` (blocking put) support synchronous callers.
  Native fibers park on channel futures. Outside fibers, callers block with a thread waker.
  Colored async callers receive an error unless they run inside a fiber.
  The IR interpreter's `ChanTake`/`ChanPut` opcodes use the same paths.

### Channel model

`(chan)` (or `(chan 0)`) is an unbuffered **rendezvous** channel: a `put!` resolves `true`
only once a `take!` consumes its value. `(chan n)` is **buffered**: `put!` succeeds while the
buffer has room, `take!` while it is non-empty. A closed channel drains any buffered values,
then `take!` yields `nil` and `put!` resolves `false`.

Channel operations that can block return a `Value::Future`, so they are used with `await`
inside an async context:

```clojure
(require '[clojure.core.async :refer [chan take! put! close! go]])

(def in  (chan 1))
(def out (chan 1))
(go (let [v (await (take! in))]
      (await (put! out (* v 2)))))   ; go spawns the body as an async task
(await (put! in 21))
(await (take! out))                  ; => 42
```

`poll!` (non-blocking take → value or `nil`) and `offer!` (non-blocking buffered put →
`true`/`false`) act synchronously and return immediately. `<!!` and `>!!` are the
blocking sync-context equivalents, suitable for REPL use and tests (see Phase H above).

### Isolate channels — the copy boundary in Clojure

An **isolate** channel is a *distinct constructor* from `(chan)`. The point is
source-visibility (see `docs/isolate-boundary-plan.md`): sending through an
isolate channel deep-copies the value across an isolate boundary, and you know
that because the thing you hold is an isolate-channel end — you do not tag each
send.

```clojure
(require '[clojure.core.async :refer [isolate-chan isolate-put! isolate-poll! isolate-take!]])

(let [[tx rx] (isolate-chan)]
  (isolate-put! tx {:a 1 :b [2 3]})  ; deep-copies the map across the boundary → true
  (isolate-poll! rx))                ; => {:a 1 :b [2 3]} (an independent copy), or nil if empty

;; in an async body, park until a value arrives:
(go (await (isolate-take! rx)))
```

`isolate-put!` returns `true` on success, `false` if the receiver is gone, and
**throws at the put site** if the value holds isolate-local state (a closure,
atom, native resource, …) that cannot cross. Every accepted send is metered
into `--gc-stats` (bytes copied + time). The sender is multi-producer
(value-copyable); the receiver is single-consumer and deserializes into the
heap of whichever isolate holds it. Until the parallel primitive lands, both
ends usually live on one isolate, so this behaves as a byte-bounded queue that
still pays — and meters — the honest deep copy. `(isolate-chan limit-bytes)`
sets the channel's byte limit; `isolate-put!` throws when a message does not
fit.

### Error propagation: the `<?` family

Channel APIs deliver failures **in band** — an error value (a `Value::Error`, the same
thing `throw` raises and `try`/`catch` catches) is put on the channel instead of a result.
`clojure.rust.error` holds the predicates and the propagation primitive:

- `(error? x)` — true if `x` is an error value.
- `(ok? x)` — true if `x` is non-nil and not an error.
- `(throw-err x)` — return `x`, unless it is an error, in which case `throw` it. This is the
  `?` primitive; the short-circuit is an exception that unwinds to the nearest `try`.

`clojure.core.async` builds the take-and-propagate sugar on top, mirroring Rust's `?`:

| Macro | Expansion | Use in |
|---|---|---|
| `(<? ch)` | `(throw-err (await (take! ch)))` | `go` / `^:async` body |
| `(<?? ch)` | `(throw-err (<!! ch))` | sync / REPL / tests |
| `(go-try body…)` | `(go (try body… (catch Exception e e)))` delivered on a 1-buffer result chan | wraps a body so a thrown error becomes an in-band error on the returned channel |

`<?` is unwrap-or-short-circuit; `go-try` is the boundary that turns a thrown error back into
an in-band error value, so a pipeline of channels propagates errors automatically (each stage
`<?`s its input and is wrapped in `go-try`).

**Error-value fidelity:** a failed future stores the thrown Clojure value
(`FutureState::Failed(Value)`), and `await`/`deref` re-raise it as `EvalError::Thrown`, so
`ex-message`/`ex-data`/`ex-cause` survive across an `await` boundary (and through `<?`/`go-try`).
Gas exhaustion uses the distinct `FutureState::GasExhausted` state and is
re-raised as `EvalError::GasExhausted`, so it cannot be intercepted by a
user-level catch. A memory-limit fault (strict governor mode) likewise settles as
`FutureState::MemoryLimitExceeded` and is re-raised as
`EvalError::MemoryLimitExceeded`; `check_ready` reports it as
`Readiness::MemoryLimitExceeded`, and a compiled poll that sees it confirms
the fault in the runtime-fault slot, which `CompiledAsyncTask::poll` checks
after every poll. Fiber cancellation settles as `FutureState::Interrupted`, is
re-raised as `EvalError::Cancelled`, and reaches compiled polls as
`Readiness::Cancelled` / `POLL_CANCELLED`, so it is never reported as gas exhaustion. Every `spawn_future` captures the complete active meter stack
and reinstalls it only for each task poll, preserving nested budgets without
leaking thread-local state across sibling `LocalSet` tasks.

**Which forms yield:** `eval_async` has a yielding arm for `await`, `do`, `if`, `let`/`let*`,
`loop`/`loop*`, `recur`, `try` (body, `catch` handlers and `finally`), `def`, `defonce`, `and`,
`or`, `throw`, `set!` (value and `(.-field inst)` target), `letfn` (body), `binding` (inits and
body), `with-out-str` (body), collection literals, and function-call arguments. Every *other* special form is delegated to the synchronous evaluator,
which evaluates its sub-expressions on the blocking `await` path. On the single-threaded
`LocalSet` that deadlocks as soon as the awaited future is not already settled, because the task
that would settle it cannot run while the executor thread is parked. So a special form that can
carry an `await` anywhere inside it needs its own arm here; a fallthrough is a latent hang, not a
slow path. The remaining delegated special forms evaluate nothing an `await` could sit in at the
point they run: `quote`, `var`, `fn`/`fn*`, `defn`/`defn-`, `defmacro`, `ns`, `in-ns`, `require`,
`alias`, `load-file`, `protocol*` and `deftype*` either take unevaluated forms or register a body
that runs later through the apply path. `.` is also delegated; the sync evaluator currently
rejects it ("interop not yet implemented"), and it needs an arm once it evaluates its operands.

`binding` and `with-out-str` rely on thread-local state (the dynamic-binding stack and the output
capture buffer), and every `LocalSet` task shares the thread. Their async arms therefore install
that state only while their own task is being polled and take it back off before yielding
(`poll_scoped`): a task that runs while a `binding` body is suspended does not see its frame, and
output such a task prints is not captured by a suspended `with-out-str`. A `set!` of a bound var
inside the body persists across yields.

### Blocking style on native fibers

`future`, `future-call`, `thread`, `thread-call`, and `io-thread` run ordinary
functions in fibers on the isolate thread. Each child receives a snapshot of
the caller's dynamic bindings. Changes in a child stay in that child.

```clojure
(require '[clojure.core.async :as a])

@(future @(future 42))
(a/<!! (a/io-thread (slurp "data.edn")))

(let [ch (a/chan)]
  (a/io-thread (a/>!! ch "hello"))
  (a/<!! ch))
```

`io-thread` and `thread` return a channel. The channel receives a non-nil result,
then closes. A nil result or an exception closes the channel without a value.
`thread-call` accepts a zero-argument function. Each call uses one body fiber.
Body failures print a diagnostic before the channel closes.
A failed future raises its error
when a caller dereferences it. `future-cancel` cancels the result and leaves the body active.
The experimental future names remain available for compatibility.

The CLI drives `run`, `eval`, REPL, nREPL, and test forms in root fibers.
The AOT program and test harnesses also use root fibers. Ordinary functions can
use `@`, `<!!`, `>!!`, and `sleep` through nested calls without async annotations.
Execution is concurrent on one isolate. Isolates provide parallel Clojure execution.
CPU-heavy bodies can delay other fibers until a park point.

`go` and `^:async` retain the async evaluator until Phase 5. In those bodies,
`await` remains the supported wait operation. On wasm32, the surface uses
LocalSet tasks and retains the async waiting rules.

## Native fiber core

`cljrs_async::fiber` provides stackful fibers on native targets. Each fiber stays on its
isolate thread. Tokio's `LocalSet` drives the fiber and polls its parked futures.

```rust,ignore
let task = cljrs_async::fiber::spawn(|| {
    cljrs_async::fiber::park_on(tokio::time::sleep(std::time::Duration::from_millis(1)));
    Ok(cljrs_value::Value::Long(42))
})?;
let value = task.await?.into_result()?;
```

`run_root(rt, local, carrier, body) -> EvalResult` drives a borrowed top-level
computation on a 64 MiB stack. It transfers the host mutator state into the root
and restores that state on return. The caller retains `carrier: MutatorState`
between drives on the same LocalSet. This state holds roots for pending async tasks.
The caller also installs this carrier state while destroying the LocalSet.

`park_on` can borrow fiber stack locals and suspend through synchronous native frames.
`current()` returns the running fiber ID. During future polls, it returns `None`.
Panics in the body or parked future become `EvalError::Runtime`.
Cancellation unwinds the fiber with its mutator state installed before stack reuse.

The default stack reservation is 1 MB. `CLJRS_FIBER_STACK_KB` sets the reservation, with a
512 KB minimum. Smaller values are rejected before stack allocation. A process-wide pool
keeps up to 32 finished stacks for any carrier.
Each active or pooled stack charges `MemoryClass::FiberStack` with a 64 KiB commit
estimate plus guard pages, not its full reservation. Strict admission can reject
construction with `EvalError::MemoryLimitExceeded`. A strict admission of any memory
class that would exceed the hard limit first releases every idle stack, and memory
pressure also drains the pool. Stacks are not returned to the pool under pressure.
`--gc-stats` and the governor snapshot show live and peak fiber counts, pooled stack
counts, and reserved stack bytes. The root's 64 MiB reservation uses the same accounting.

At interpreter, IR, JIT, and AOT safepoints, a fiber yields after its time slice when
another fiber is ready. `CLJRS_FIBER_SLICE_US` defaults to 10000 microseconds.
Clock checks are batched every 64 safepoints. Yielding waits for runtime locks, borrowed mutator slots, and
governor critical sections to end in every build. It also defers during Rust unwinding.
Root guards protect environments only during an actual yield.
Cancellation at an automatic yield uses a private resume token. It returns through
native frames using the uncatchable resource-error path, then resumes Rust unwinding
at the outer computation boundary. Cleanup cannot suspend a cancelled fiber.
Other panic payloads retain their original message and type.

A process watchdog thread sleeps on a condition variable while no fiber is running, and otherwise checks once per stall interval. If a fiber misses safepoints for
`CLJRS_FIBER_STALL_MS` (default 100 ms) while another fiber is ready, it prints the
fiber ID and innermost native function name once for that stall. Parked fibers do not
trigger warnings. Both timing settings accept positive integers; invalid values use defaults.

Safepoints reject recursion when fewer than 256 KiB remain above the stack limit.
The check reads the machine stack register; ASan can place local variables on a
separate fake stack, so a local variable's address cannot measure this margin.
The error names the fiber and reports reserved and remaining bytes. The margin covers
interpreter frames and error construction; very small reservations cannot run deep Clojure code.
Native recursion without safepoints can still hit the fatal guard page.
The driver does not replace platform signal handlers. Native diagnostic names are
cached per isolate; warm calls update a frame token and an atomic name index without
locking the watchdog mutex or cloning the name.

Saved mutator states retain precise GC roots. Suspended JIT frames keep their code epochs live.

For captured managed values, use `spawn_rooted(values, body)` or `Fiber::with_roots(values, body)`.
These values remain rooted before the first poll and until completion.
For future captures, use `spawn_future_with_roots(values, task)`.
The task leases declared captures without inheriting unrelated temporary reads.
For task locals, use the runtime's GC root guards.
GC builds retire active regions through the existing async poison fallback.
In `no-gc` builds, task captures lease active scratch regions until completion or cancellation.
Task results retain their backing scopes for the destination's lifetime.
Publication preserves lazy sequences without copying or realizing them.
Typed isolated allocation invocations cannot spawn Clojure fibers.
Spawned results stay rooted in `FiberResult` until consumption or drop.
In no-gc builds, queued results hold scope leases until the consumer takes ownership.
After `into_result`, the caller owns the normal root-registration responsibility.

The native parking primitives and Clojure fiber surface are available.
The existing async evaluator still drives `go` and `^:async`.
The fiber module and corosensei dependency are excluded on `wasm32`.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `init(globals)` entry point; registers `AsyncRuntimeImpl`, loads `clojure.rust.error`, and builds the `clojure.core.async` namespace |
| `src/runtime.rs` | `AsyncRuntimeImpl`: async task dispatch and fiber call dispatch with binding, gas, and interrupt conveyance |
| `src/state_machine.rs` | Compiled-async runtime (Phase H): `CljxStateMachine` (resume state + GC-rooted live-value slots + `pending` + the `eval_ctx` installed around each poll so the poll fn's global-lookup/call bridges work while detached on the executor), the C-ABI `PollFn` type and `POLL_PENDING`/`POLL_READY`/`POLL_THREW`/`POLL_GAS_EXHAUSTED`/`POLL_CANCELLED` codes, `check_ready` resume helper, and `CompiledAsyncTask`/`spawn_state_machine` which drive a compiled poll function on the `LocalSet` while keeping its slots traced. Also the poll-fn registry that `AsyncRuntimeImpl::spawn_async_call` consults to route an `^:async` call to the native state machine instead of `eval_async`'s `run_async_fn`: `register_poll_fn`/`lookup_poll_fn`/`mark_compile_attempted` (keyed by `ir_arity_id`, used by the JIT hook which compiles once on first call) and `register_poll_fn_named`/`lookup_poll_fn_named` (keyed by `(ns, name, arity)`, registered by the AOT harness for `cljrs compile`d binaries) |
| `src/fiber.rs` | Native stackful fibers, governed stack pool, parking, panic delivery, cancellation, and rooted results |
| `src/fiber/scheduler.rs` | Safepoint time slices, ready-task wakers, stack-margin checks, native-frame names, and carrier watchdog |
| `src/eval_async.rs` | `eval_async` async tree-walker, `run_async_fn` driver, and the shared `spawn_future`/`settle_future`/`await_value` task helpers |
| `src/channel.rs` | `CljChannel` (buffered/rendezvous) and `CljMult` (broadcast multiplexer) exposed as `NativeObject`s |
| `src/builtins.rs` | native fns: `timeout`, `alts`, `chan`, `take!`, `put!`, `close!`, `poll!`, `offer!`, `async-spawn`, `join-all`, `thread-call`, `onto-chan!`, `to-chan!`, `mult`, `tap!`, `untap!`, `untap-all!`, `<!!`, `>!!` |
| `src/core_async.cljrs` | Clojure source for `clojure.core.async`: `go`, `alt`, `async-pmap`, `thread`, `merge`, `reduce`, `into`, and the `<?` family (`<?`, `<??`, `go-try`) |
| `src/clojure_rust_error.cljrs` | Clojure source for `clojure.rust.error`: in-band error helpers `error?`, `ok?`, `throw-err` |
| `src/isolate.rs` | `Isolate` — per-isolate execution context: dedicated OS thread, `current_thread` Tokio runtime + `LocalSet`, and independent GC heap (thread-local). `Isolate::spawn` initializes GC state and runs the entry-point future |
| `src/isolate_channel.rs` | `IsolateSender` / `IsolateReceiver` / `isolate_channel()` — cross-isolate copy boundary (Phase B2): structured-clone of `Value` through a `SerializedValue` wire form over a byte-bounded queue whose messages hold governor `MessageQueue` charges; `isolate_channel_with_limit`, `try_send`/`send_async`, `IsolateReceiver::close`; `IsolateRecv`/`try_recv_status` distinguish empty from disconnected for the Clojure builtins |
| `src/isolate_builtins.rs` | Clojure-level surface for the copy boundary: `CljIsolateTx`/`CljIsolateRx` native objects and the `isolate-chan` (optional byte limit) / `isolate-put!` / `isolate-poll!` / `isolate-take!` builtins |
| `src/worker_pool.rs` | `WorkerPool` singleton: multi-thread Tokio runtime (`new_multi_thread`) for `Send` pool tasks; wasm32 stub; `offload` bridges pool results to LocalSet via oneshot; `handle` for direct multi-task spawning |
| `tests/error_propagation.rs` | integration tests for the `<?` family and `clojure.rust.error` helpers |
| `tests/async_fn.rs` | integration tests for dispatch, `await`, `deref` enforcement, `timeout`/`alts`/`alt`, channels, Phase F utilities, and `<!!`/`>!!` |
| `tests/worker_pool.rs` | Phase A2 integration tests: offload, concurrent tasks, handle spawning, LocalSet context, singleton invariant, byte processing round-trip |
| `tests/isolate_channel_clj.rs` | Clojure-level Phase B2 tests: `isolate-chan` pair, put/poll round-trip, FIFO order, located error on a non-shareable value, async `isolate-take!`, byte-limited channel reports `Full` at the put site |
| `tests/no_gc_lifetimes.rs` | No-GC lifetime tests for task results, lazy sequences, top-level accumulators, and replacement of cell contents |
| `tests/thread_limits.rs` | Resource failures bypass catch clauses through every channel consumer. Unread thread failures report once at destruction or teardown |
| `tests/fiber_surface.rs` | Blocking Clojure surface, nested futures, binding conveyance, channel closure, root state restoration, and GC across root drives |
| `tests/future_family.rs` | `cljrs.core.experimental`'s `future`/`future-call`/`future?`/`future-done?`/`future-cancelled?`/`future-cancel` on the isolate executor: cooperative start, interleaving, throwing bodies, sticky cancellation, and cancelled-error parity between the tree-walking and compiled await paths |
| `tests/toplevel_interop.rs` | `(.method target)` / `(.-field target)` under the legacy `eval_async` driver |
| `tests/async_root_order.rs` | two `LocalSet` tasks whose async loops hold value roots across `await` and finish out of LIFO order keep their roots through collections |
| `tests/loop_garbage.rs` | collections during `loop*`/`recur` free dead intermediates in the tree-walker, the IR tier, and `eval_async` (csm/clojurust#428) |
| `tests/reader_conditional_parity.rs` | property + example tests that `#?`/`#?@` resolve identically in `eval_async` and the sync evaluator (containers, `let*`/`loop*` binding vectors) |

## Public API

```rust
/// Register the async runtime and load clojure.core.async.
/// Must be called inside a Tokio LocalSet context for spawned tasks to run.
pub fn init(globals: &Arc<GlobalEnv>);

pub mod isolate {
    /// An independent execution context with its own Tokio `current_thread`
    /// runtime, `LocalSet`, and per-isolate GC heap (thread-local).
    ///
    /// Isolates share no heap pointers; the `!Send` bound on `GcPtr` enforces
    /// this at compile time. Values cross isolate boundaries via copy or
    /// structured-clone (Phase B2).
    pub struct Isolate;
    impl Isolate {
        /// Create a new isolate with the given debug name.
        pub fn new(name: impl Into<String>) -> Self;

        /// Spawn on a dedicated OS thread; `f()` is the entry-point future.
        /// The thread registers with the GC, configures per-isolate limits, and
        /// runs a `current_thread` + `LocalSet` executor until `f()` completes.
        /// Not available on wasm32.
        #[cfg(not(target_arch = "wasm32"))]
        pub fn spawn<F, Fut>(self, f: F) -> std::thread::JoinHandle<()>
        where
            F: FnOnce() -> Fut + Send + 'static,
            Fut: Future<Output = ()> + 'static;

        /// Return root evaluation errors through the join result.
        #[cfg(not(target_arch = "wasm32"))]
        pub fn spawn_root<F>(self, body: F) -> std::thread::JoinHandle<Result<(), RootError>>
        where
            F: FnOnce() -> EvalResult + Send + 'static;
    }
    pub enum RootError {
        GasExhausted,
        Cancelled,
        MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
        Evaluation(String),
    }
}

/// Re-exports for sibling native crates (e.g. cljrs-io) that drive their own
/// work onto the shared LocalSet executor.
pub use eval_async::{await_value, spawn_future, spawn_future_with_roots};

pub mod worker_pool {
    /// Global Send-only worker pool backed by a Tokio multi-thread runtime.
    /// On wasm32 this is a stub that runs futures locally.
    pub struct WorkerPool { /* singleton */ }
    impl WorkerPool {
        /// Get (or initialize) the global singleton.
        pub fn global() -> &'static Self;

        /// Offload a Send future to the pool; the returned future can be awaited
        /// on the LocalSet thread without blocking.
        pub fn offload<F, T>(&self, f: F) -> impl Future<Output = T> + 'static
        where F: Future<Output = T> + Send + 'static, T: Send + 'static;

        /// Direct handle to the pool runtime for multi-task spawning (non-wasm only).
        #[cfg(not(target_arch = "wasm32"))]
        pub fn handle(&self) -> &tokio::runtime::Handle;
    }
}

pub mod isolate_channel {
    /// Sending end of a cross-isolate channel (cloneable, Send).
    #[derive(Clone)]
    pub struct IsolateSender;
    impl IsolateSender {
        /// Serialize `v`, meter the crossing (bytes + time into GC_STATS), and
        /// enqueue it without waiting. Returns CloneError for non-shareable
        /// values (located at the send site), `Disconnected` if the receiver
        /// has been dropped, `Full` if the message does not fit, or
        /// `MemoryLimitExceeded` if the strict governor refuses its charge.
        /// Compatibility form of `try_send`.
        pub fn send(&self, v: &Value) -> Result<(), CloneError>;
        /// Same as `send`.
        pub fn try_send(&self, v: &Value) -> Result<(), CloneError>;
        /// Enqueue, waiting while the channel is full. A message larger than
        /// the whole limit returns `Full` at once.
        pub async fn send_async(&self, v: &Value) -> Result<(), CloneError>;
        /// The channel's byte limit.
        pub fn limit(&self) -> usize;
        /// Wire bytes currently queued.
        pub fn queued_bytes(&self) -> usize;
    }

    /// Receiving end of a cross-isolate channel. Must remain on the destination
    /// isolate thread so `deserialize` allocates into the correct GC heap.
    pub struct IsolateReceiver;
    impl IsolateReceiver {
        /// Async receive: deserialize the next value into the current isolate's heap.
        /// Returns `None` when all senders have been dropped.
        pub async fn recv(&mut self) -> Option<Value>;
        /// Non-blocking receive attempt. Returns `None` if the channel is empty.
        pub fn try_recv(&mut self) -> Option<Value>;
        /// Non-blocking receive that distinguishes `Empty` (keep waiting) from
        /// `Disconnected` (all senders gone). Backs the Clojure builtins.
        pub fn try_recv_status(&mut self) -> IsolateRecv;
        /// Close the channel and release every queued message's charge.
        /// Dropping the receiver does the same.
        pub fn close(&mut self);
        pub fn limit(&self) -> usize;
        pub fn queued_bytes(&self) -> usize;
    }

    /// Outcome of `try_recv_status`: `Value(v)`, `Empty`, or `Disconnected`.
    pub enum IsolateRecv { Value(Value), Empty, Disconnected }

    /// Create a linked `(IsolateSender, IsolateReceiver)` pair with the
    /// governor's default queue limit.
    pub fn isolate_channel() -> (IsolateSender, IsolateReceiver);
    /// Create a pair that holds at most `limit` wire bytes (panics on zero).
    pub fn isolate_channel_with_limit(limit: usize) -> (IsolateSender, IsolateReceiver);
}

pub mod eval_async {
    /// Spawn `task` on the current LocalSet and return a `Value::Future` that
    /// settles when it completes. The shared delivery point for async primitives;
    /// public so other native crates can produce results through the same path.
    pub fn spawn_future<F>(task: F) -> Value
    where
        F: Future<Output = Result<Value, EvalError>> + 'static;

    /// Spawn a task with its captured managed values.
    pub fn spawn_future_with_roots<F>(roots: Vec<Value>, task: F) -> Value
    where
        F: Future<Output = Result<Value, EvalError>> + 'static;

    /// Drive an ^:async fn body to completion, yielding at every await.
    pub async fn run_async_fn(callee: Value, args: Vec<Value>, base: &Env)
        -> Result<Value, EvalError>;

    /// Asynchronously evaluate a single form. Handles await/do/if/let and
    /// function-call arguments with yielding; delegates other forms to the
    /// synchronous evaluator. A call whose head is a form-intercepted native
    /// (`cljrs_interp::apply::is_form_intercepted` — `apply`, `swap!`, `eval`,
    /// …) is handed to the synchronous evaluator whole, since those need the
    /// unevaluated forms.
    pub async fn eval_async(form: &Form, env: &mut Env) -> Result<Value, EvalError>;

    /// Cooperatively await a Clojure value inside a LocalSet context.
    /// Futures and promises yield until resolved; any other value is returned as-is.
    /// Used by the WASM REPL for implicit top-level await.
    pub async fn await_value(val: Value) -> Result<Value, EvalError>;
}

pub mod channel {
    /// A CSP channel (buffered or rendezvous) exposed as a `Value::NativeObject`.
    /// `(chan)` constructs one; the channel builtins downcast to it.
    pub struct CljChannel { /* ... */ }
    impl CljChannel {
        /// Create a channel. `capacity == 0` is an unbuffered rendezvous channel.
        pub fn new(capacity: usize) -> Self;
        /// Async put: yield to the LocalSet until the value is accepted (buffered
        /// or handed off). `true` on success, `false` if the channel is closed.
        /// The building block other crates use to stream produced values.
        /// Enqueued values pass through the Phase 10.5 publish barrier
        /// (cljrs_value::publish::publish_value): the taker may outlive any
        /// bump-region scope active on the putting side.
        pub async fn put(&self, v: Value) -> bool;
        /// Close the channel (idempotent). Buffered values still drain to takers.
        pub fn close(&self);
        /// Park the current fiber, or block the OS thread outside a fiber.
        /// Returns nil when the channel closes and drains.
        pub fn take_blocking(&self) -> Value;
        /// Block the calling OS thread until the value is accepted or the channel closes.
        /// Returns `true` on success, `false` if the channel was closed.
        pub fn put_blocking(&self, v: Value) -> bool;
    }

    /// A broadcast multiplexer. `(mult src-ch)` creates one; values from `src-ch`
    /// are forwarded to all registered tap channels via `tap!`/`untap!`/`untap-all!`.
    pub struct CljMult { /* ... */ }
    impl CljMult {
        pub fn new() -> Self;
    }
}
```

## Integration

**Native (CLI):** The `cljrs` CLI links this crate when built with the `async` feature (on by default).
Rust embedders call `init` from within a Tokio `LocalSet` context:

```rust
let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
let local = tokio::task::LocalSet::new();
rt.block_on(local.run_until(async {
    let runtime = cljrs_runtime::Runtime::builder().build()?;
    cljrs_stdlib::install(&runtime);
    let globals = runtime.into_globals();
    cljrs_async::init(&globals);
    // ... eval code ...
}));
```

**Calling `init` outside a `LocalSet`:** `init` is also called with no Tokio runtime
at all — by the AOT compiler (through the host's `ExtensionSet`, which registers
`clojure.core.async` only so that `require`/`go`/`await` resolve during
macro-expansion) and by unit tests, as well as
before a `LocalSet` context exists on WASM (e.g. in `Repl::new()`). `spawn_gc_service`
probes `tokio::runtime::Handle::try_current()` and silently no-ops when there is no
runtime, so these callers see no spurious panic. Re-call `init` from inside a
`LocalSet::run_until` block to start the GC service.
`timeout` uses `gloo_timers::future::sleep` on `wasm32` instead of `tokio::time::sleep`.

**Build features:** `no-gc` (default off) forwards the no-GC build to
`cljrs-gc`, `cljrs-value`, and `cljrs-runtime`, so a `no-gc` workspace build
can include the async extension.
The `cljrs` CLI forwards it weakly (`cljrs-async?/no-gc`), i.e. only when its
own `async` feature pulls this package in.

**Timer portability:** On `wasm32` the `time` feature of tokio is present but
`platform_sleep` (used internally by `timeout`) delegates to `gloo-timers` so that
the browser's `setTimeout` is used instead of a non-functional OS-level clock.

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`, which trades Unicode character classes for ~277 KB of text. |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |
| `fiber-sanitizer` | off | Enables corosensei's native stack-switch hooks for AddressSanitizer. Requires an ASan-instrumented build. |

Every workspace dependency of this crate is taken with default features off (see
the note in the root `Cargo.toml`), so these pass-throughs are what put back what
those crates' defaults used to provide. `default` enables all of them, so a plain
build is unchanged.

`regex-full` wins when both regex features are enabled, so selecting the small
engine means turning default features off **on this crate** and re-adding what
you want:

```toml
cljrs-async = { version = "0.1", default-features = false, features = ["small-regex"] }
```

Adding a second, direct dependency on `cljrs-runtime` with
`default-features = false` would not undo it — Cargo unions features across every
edge to a package, so one edge left at its defaults re-enables `regex-full` for
the whole graph. `deps` has to be off as well for the size win to land, since
`cljrs-project/vcs` pulls `regex` in through `pgp`. See
[cljrs-value's README](../cljrs-value/README.md#features).

## Blocking operations inside native fibers

Inside a native fiber, `<!!` and `>!!` park on channel futures. Other fibers on the isolate can run during the wait.
Outside a fiber, these operations keep their blocking behavior. Colored async callers still use `await`.

`alts!!` accepts channels for takes and `[channel value]` pairs for puts.
It returns `[value channel]` for a take or `[accepted? channel]` for a put.
`:default value` returns `[value :default]` without a transfer if no operation is ready.
`:priority true` checks operations in input order. Otherwise, selection shuffles the input order.
Options accept key/value arguments or one map. Nil put values are rejected.
Shared selection tokens commit exactly one operation, including rendezvous handoffs between two selectors.
Cancellation removes uncommitted registrations. Losing operations do not transfer values.

Future and promise dereferences also park inside fibers, including timed dereferences.
`sleep` and `Thread/sleep` use the Tokio timer. `locking` uses reentrant monitors with a separate owner for each fiber.
Delay and lazy sequence waiters receive the settled result after the first fiber finishes realization.
Cancellation releases monitors and retracts pending channel operations.
An unconsumed committed take returns its value to the channel in FIFO order.
Channel transfers wake only the selected tasks. Pending buffered puts fill free slots in registration order.
Interrupted thunks return to their pending state so realization waiters can retry.
Legacy async tasks reject explicit monitor calls, including calls through synchronous helpers.
Carrier waits on a fiber-owned monitor or realization on the same thread fail before they can block the executor.
Channel waiters register their wakers before readiness checks. Channel closure wakes every pending operation.

Core `slurp` and `spit` run file work on the blocking worker pool.
The stdlib reader, writer, and process operations use the same parking hooks.
The existing `cljrs-io` and `cljrs-net` APIs return channels, so `<!!` and `>!!` also provide their parking paths.
Managed values stay on the isolate. Native Rust code can use `cljrs_value::parking::blocking` for owned, `Send` work.

Clojure spawn functions and native CLI entry points create fibers.
Rust embedders can use `fiber::spawn` or `Fiber::with_roots`.
`(blocking & calls)` and `cljrs.fiber/blocking` accept typed native call forms.
Arguments and their conversions stay on the isolate. Only Rust inputs and outputs with `Send + 'static` cross to workers.
The form rejects Clojure closures, variadic managed-value exports, and native functions without an owned-work adapter.
Fixed-arity `#[export]` functions provide adapters automatically. The calls run sequentially and return the last result.
An empty form returns nil. Outside a fiber, execution is synchronous.

Heap-isolation fixtures that assert traced heap counts run only in GC builds.
Notification and parking regressions also run in `no-gc` builds.

Synchronous embedders can use `Isolate::spawn_root(body)` or `fiber::run_root`
with a borrowed Form and Env. Root evaluation preserves the original Env on unwind
and avoids a deep Form clone. `Isolate::spawn` remains an async entry point.
A pending local future rejects synchronous dereference on its carrier thread.
Such code uses `await` or moves synchronous evaluation into a fiber.
Settled futures and futures produced by another OS thread retain normal blocking behavior.

`fiber::run_root_owned(rt, local, body)` owns one executor drive and destroys its local tasks with the carrier state installed.
`fiber::drop_local_with_carrier(local, carrier)` provides the same teardown for executors retained between root drives.
Both helpers restore caller state if task destruction unwinds.

`Isolate::spawn_root` returns `JoinHandle<Result<(), RootError>>`.
Root errors remain ordinary results. `RootError` preserves typed gas and memory-limit details and carries diagnostic text for other errors.
Managed Values remain on their isolate thread. An OS-thread panic still appears in the outer join result.

`CljChannel::take_result()` preserves resource-limit failures from a thread producer.
Clojure blocking takes, future takes, polls, and channel alternatives propagate those failures without making them catchable.
Ordinary thread-body throws are reported and close the result channel.
Channels publish values only after a put commits. Failed offers and losing or cancelled alternatives do not publish their candidates.

Buffered channel values pass publication once, when the put commits.
A take transfers ownership to the consumer without another publication scan.
Direct handoffs publish the accepted value before either participant resumes.
Channels report resource failures only when the failure remains unread at destruction or executor teardown.
A read marks the report observed. The internal delivery future also counts as observed, so collection produces no duplicate warning.
`channel::report_unobserved_thread_failures()` flushes the current executor registry after all consumers finish.
The carrier and its fibers share this registry. Another executor's teardown cannot flush these reports.
Native root drivers and both isolate entry points call this helper during teardown.
Other embedders, including wasm hosts, can call it when their executor closes.
Channel reports contain no managed Values.

Each no-GC queue entry and completed selection owns a cached payload lease.
A take moves that lease to the selection. Cancellation restores it with the queued value.
The consumer acquires ownership before the selection releases its lease.
Permanent and regional channels release consumed payloads after their last consumer releases them.
Channel ownership tracing skips replaceable queue contents, which manage their own leases.


No-GC task polls install their own `region::ReadContext` and restore the polling caller's context afterwards.
Read leases therefore remain with a suspended task when its caller's form ends.
Public async evaluation and awaiting use the same poll-scoped ownership rule.
Fiber results use a handoff lease while mutator state is restored.
A root result borrows into the restored caller's read scope, when present.
Owned `FiberResult` values keep their leases without permanent carrier publication.


`tests/fairness.rs` covers CPU-loop fairness in both interpreters, collection during
automatic yields, stack accounting and pool reuse, cancellation, strict admission,
stall diagnostics, and recursion errors. These tests also run with `no-gc`.
