# cljrs-async

Async support for clojurust — `clojure.core.async` implemented via Tokio.

## Purpose

Provides CSP-style concurrency (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) and
the `^:async` / `await` function model, backed by a Tokio `current_thread` + `LocalSet`
executor. All Clojure values remain on a single thread, keeping GC pointers (`!Send`) safe.

## Status

**Phase B (async functions)** — `^:async` function dispatch is implemented. Calling a
function marked `^:async` (when the runtime is registered) spawns its body as a `LocalSet`
task and returns a `Value::Future` immediately; `eval_async` drives the body and yields the
executor at every `await`.

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

Not yet implemented (later phases):

- Phase E: `chan`, `put!`, `take!`, `close!`, `go`.

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
| `src/builtins.rs` | native `timeout` and `alts` functions |
| `src/core_async.cljrs` | Clojure source for the `clojure.core.async` namespace (the `alt` macro) |
| `tests/async_fn.rs` | integration tests for dispatch, `await`, `deref` enforcement, `timeout`/`alts`/`alt` |

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
