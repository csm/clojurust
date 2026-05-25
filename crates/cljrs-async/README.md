# cljrs-async

Async support for clojurust — `clojure.core.async` implemented via Tokio.

## Purpose

Provides CSP-style concurrency (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) and
the `^:async` / `await` function model, backed by a Tokio `current_thread` + `LocalSet`
executor. All Clojure values remain on a single thread, keeping GC pointers (`!Send`) safe.

## Status

**Phase E (channels)** — CSP channels are implemented. `(chan)` returns a `CljChannel`
wrapped as a `Value::NativeObject`; `take!`/`put!` return a `Value::Future` that parks
(yields) until the operation completes, and `close!`/`poll!`/`offer!` act synchronously. The
`go` macro spawns its body as an async task via the native `async-spawn`.

Done:

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
`true`/`false`) act synchronously and return immediately. Blocking sync-context variants
(`take!!`/`put!!`) are deferred along with the top-level blocking bridge (see below).

Not yet implemented (later phases):

- Phase F+: `take!!`/`put!!` (blocking sync ops), `async-pmap`, `join-all`, GC safepoints,
  IR support, and the wider `clojure.core.async` surface (`thread`, `pipeline`, `mult`, …).

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
| `src/channel.rs` | `CljChannel` — a buffered/rendezvous CSP channel exposed as a `NativeObject` |
| `src/builtins.rs` | native fns: `timeout`, `alts`, `chan`, `take!`, `put!`, `close!`, `poll!`, `offer!`, `async-spawn` |
| `src/core_async.cljrs` | Clojure source for the `clojure.core.async` namespace (the `alt` and `go` macros) |
| `tests/async_fn.rs` | integration tests for dispatch, `await`, `deref` enforcement, `timeout`/`alts`/`alt`, and channels |

## Public API

```rust
/// Register the async runtime and load clojure.core.async.
/// Must be called inside a Tokio LocalSet context for spawned tasks to run.
pub fn init(globals: &Arc<GlobalEnv>);

pub mod eval_async {
    /// Drive an ^:async fn body to completion, yielding at every await.
    pub async fn run_async_fn(callee: Value, args: Vec<Value>, base: &Env)
        -> Result<Value, EvalError>;

    /// Asynchronously evaluate a single form. Handles await/do/if/let and
    /// function-call arguments with yielding; delegates other forms to the
    /// synchronous evaluator.
    pub async fn eval_async(form: &Form, env: &mut Env) -> Result<Value, EvalError>;
}

pub mod channel {
    /// A CSP channel (buffered or rendezvous) exposed as a `Value::NativeObject`.
    /// `(chan)` constructs one; the channel builtins downcast to it.
    pub struct CljChannel { /* ... */ }
    impl CljChannel {
        /// Create a channel. `capacity == 0` is an unbuffered rendezvous channel.
        pub fn new(capacity: usize) -> Self;
    }
}
```

## Integration

The `cljrs` CLI links this crate when built with the `async` feature (on by default).
Rust embedders add `cljrs-async` to their `Cargo.toml` and call `init` manually:

```rust
let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
let local = tokio::task::LocalSet::new();
rt.block_on(local.run_until(async {
    let globals = cljrs_stdlib::standard_env();
    cljrs_async::init(&globals);
    // ... eval code ...
}));
```
