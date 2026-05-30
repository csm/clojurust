# cljrs-async

Async support for clojurust — `clojure.core.async` implemented via Tokio.

## Purpose

Provides CSP-style concurrency (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) and
the `^:async` / `await` function model, backed by a Tokio `current_thread` + `LocalSet`
executor. All Clojure values remain on a single thread, keeping GC pointers (`!Send`) safe.

## Status

**Phase G (GC safety for the async runtime)** — complete. GC safepoints are integrated at
cooperative yield points. `async_gc_collect()` is called before every `yield_now().await` in
`await_value`, and a background GC-service task (spawned by `init`) services GC requests between
poll cycles. Explicit GC root guards protect `task_future` in `spawn_future`, callee/env in
`run_async_fn`, and awaited futures/promises in `await_value`.

Done (Phases A–G):

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
| `src/lib.rs` | `init(globals)` entry point; registers `AsyncRuntimeImpl` and builds the `clojure.core.async` namespace |
| `src/runtime.rs` | `AsyncRuntimeImpl` — Tokio-backed `AsyncRuntime`; `spawn_async_call` spawns the body on the `LocalSet` via `spawn_future` |
| `src/eval_async.rs` | `eval_async` async tree-walker, `run_async_fn` driver, and the shared `spawn_future`/`settle_future`/`await_value` task helpers |
| `src/channel.rs` | `CljChannel` (buffered/rendezvous) and `CljMult` (broadcast multiplexer) exposed as `NativeObject`s |
| `src/builtins.rs` | native fns: `timeout`, `alts`, `chan`, `take!`, `put!`, `close!`, `poll!`, `offer!`, `async-spawn`, `join-all`, `thread-call`, `onto-chan!`, `to-chan!`, `mult`, `tap!`, `untap!`, `untap-all!`, `<!!`, `>!!` |
| `src/core_async.cljrs` | Clojure source for `clojure.core.async`: `go`, `alt`, `async-pmap`, `thread`, `merge`, `reduce`, `into` |
| `tests/async_fn.rs` | integration tests for dispatch, `await`, `deref` enforcement, `timeout`/`alts`/`alt`, channels, Phase F utilities, and `<!!`/`>!!` |

## Public API

```rust
/// Register the async runtime and load clojure.core.async.
/// Must be called inside a Tokio LocalSet context for spawned tasks to run.
pub fn init(globals: &Arc<GlobalEnv>);

/// Re-exports for sibling native crates (e.g. cljrs-io) that drive their own
/// work onto the shared LocalSet executor.
pub use eval_async::{await_value, spawn_future};

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
