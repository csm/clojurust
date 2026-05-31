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
  callers to `await` (enforced in `cljrs-builtins` and `cljrs-interp` via the
  `cljrs_env::callback::current_is_async` context flag).
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
  `reduce`, `into`. `eval_loop_async` enables proper `await` yielding inside `loop/recur`.
- Phase G: GC safepoints at async yield points via `cljrs_env::gc_roots::async_gc_collect()`,
  called before each `yield_now().await` in `await_value`. Background GC-service task spawned
  by `init()`. Explicit GC root guards for `task_future` in `spawn_future`, callee/env in
  `run_async_fn`, and awaited futures/promises in `await_value`.
- Phase A2: `WorkerPool` singleton — multi-thread Tokio runtime for `Send` byte-level tasks
  (`WorkerPool::global()`, `offload`, `handle`). Pool tasks carry only `Vec<u8>`, `String`, and
  `Send` channel types; `GcPtr`/`Value` construction is confined to LocalSet bridge tasks.
  wasm32 stub runs futures locally.
- Phase B1: `Isolate` type — per-isolate OS thread with independent GC heap (`ISOLATE_HEAP`
  thread-local), `current_thread` Tokio runtime, and `LocalSet`. GC collections are fully
  parallel and independent; no cross-isolate STW coordination.
- Phase H: `<!!` (blocking take) and `>!!` (blocking put) for synchronous / REPL / test
  contexts. Both use `Condvar`-based parking (with a 1 ms poll-interval fallback so they
  remain non-deadlocking when called from the LocalSet executor thread). Errors with a
  clear message if called inside an `^:async` function body. `CljChannel` condvars also
  replace the previous spin-poll in the IR interpreter's `ChanTake`/`ChanPut` opcodes.

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

**Known limitation:** `eval_async` does not yet evaluate `try`/`catch` with *yielding* — it
delegates them to the synchronous evaluator — so an `await`/`<?` inside a `try` (and therefore
inside a `go-try` body) takes the synchronous `await` path. That resolves an already-ready value
but is fragile when the awaited value is not yet available. Yielding `try`/`catch` in
`eval_async` is the remaining follow-up.

### `await` and the single-thread executor

`await` only yields when evaluated by `eval_async` (i.e. inside an `^:async` function body or
another async driver). The synchronous `await`/`deref` fallback in `cljrs-interp` blocks the OS
thread on a condvar; doing that to an *async-spawned* future from the `LocalSet` driver thread
deadlocks, because the task that would resolve the future cannot run while the only executor
thread is parked. In Phase B, await async results from within async context. A top-level
blocking bridge is a later phase.

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `init(globals)` entry point; registers `AsyncRuntimeImpl`, loads `clojure.rust.error`, and builds the `clojure.core.async` namespace |
| `src/runtime.rs` | `AsyncRuntimeImpl` — Tokio-backed `AsyncRuntime`; `spawn_async_call` spawns the body on the `LocalSet` via `spawn_future` |
| `src/eval_async.rs` | `eval_async` async tree-walker, `run_async_fn` driver, and the shared `spawn_future`/`settle_future`/`await_value` task helpers |
| `src/channel.rs` | `CljChannel` (buffered/rendezvous) and `CljMult` (broadcast multiplexer) exposed as `NativeObject`s |
| `src/builtins.rs` | native fns: `timeout`, `alts`, `chan`, `take!`, `put!`, `close!`, `poll!`, `offer!`, `async-spawn`, `join-all`, `thread-call`, `onto-chan!`, `to-chan!`, `mult`, `tap!`, `untap!`, `untap-all!`, `<!!`, `>!!` |
| `src/core_async.cljrs` | Clojure source for `clojure.core.async`: `go`, `alt`, `async-pmap`, `thread`, `merge`, `reduce`, `into`, and the `<?` family (`<?`, `<??`, `go-try`) |
| `src/clojure_rust_error.cljrs` | Clojure source for `clojure.rust.error`: in-band error helpers `error?`, `ok?`, `throw-err` |
| `src/isolate.rs` | `Isolate` — per-isolate execution context: dedicated OS thread, `current_thread` Tokio runtime + `LocalSet`, and independent GC heap (thread-local). `Isolate::spawn` initializes GC state and runs the entry-point future |
| `src/worker_pool.rs` | `WorkerPool` singleton: multi-thread Tokio runtime (`new_multi_thread`) for `Send` pool tasks; wasm32 stub; `offload` bridges pool results to LocalSet via oneshot; `handle` for direct multi-task spawning |
| `tests/error_propagation.rs` | integration tests for the `<?` family and `clojure.rust.error` helpers |
| `tests/async_fn.rs` | integration tests for dispatch, `await`, `deref` enforcement, `timeout`/`alts`/`alt`, channels, Phase F utilities, and `<!!`/`>!!` |
| `tests/worker_pool.rs` | Phase A2 integration tests: offload, concurrent tasks, handle spawning, LocalSet context, singleton invariant, byte processing round-trip |

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
    }
}

/// Re-exports for sibling native crates (e.g. cljrs-io) that drive their own
/// work onto the shared LocalSet executor.
pub use eval_async::{await_value, spawn_future};

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

pub mod eval_async {
    /// Spawn `task` on the current LocalSet and return a `Value::Future` that
    /// settles when it completes. The shared delivery point for async primitives;
    /// public so other native crates can produce results through the same path.
    pub fn spawn_future<F>(task: F) -> Value
    where
        F: Future<Output = Result<Value, EvalError>> + 'static;

    /// Drive an ^:async fn body to completion, yielding at every await.
    pub async fn run_async_fn(callee: Value, args: Vec<Value>, base: &Env)
        -> Result<Value, EvalError>;

    /// Asynchronously evaluate a single form. Handles await/do/if/let and
    /// function-call arguments with yielding; delegates other forms to the
    /// synchronous evaluator.
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
        pub async fn put(&self, v: Value) -> bool;
        /// Close the channel (idempotent). Buffered values still drain to takers.
        pub fn close(&self);
        /// Block the calling OS thread until a value is available (or channel closes → nil).
        /// Uses Condvar with a 1 ms timeout to avoid deadlock on the LocalSet thread.
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
    let globals = cljrs_stdlib::standard_env();
    cljrs_async::init(&globals);
    // ... eval code ...
}));
```

**WASM (browser REPL):** `init` may be called before a `LocalSet` context exists
(e.g. in `Repl::new()`); `spawn_gc_service` silently no-ops via `catch_unwind` in that
case. Re-call `init` from inside a `LocalSet::run_until` block to start the GC service.
`timeout` uses `gloo_timers::future::sleep` on `wasm32` instead of `tokio::time::sleep`.

**Timer portability:** On `wasm32` the `time` feature of tokio is present but
`platform_sleep` (used internally by `timeout`) delegates to `gloo-timers` so that
the browser's `setTimeout` is used instead of a non-functional OS-level clock.
