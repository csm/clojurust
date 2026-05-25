# cljrs-async

Async support for clojurust — `clojure.core.async` implemented via Tokio.

## Purpose

Provides CSP-style concurrency (`go`, `chan`, `put!`, `take!`, `timeout`, `alts`, `alt`) and
the `^:async` / `await` function model, backed by a Tokio `current_thread` + `LocalSet`
executor. All Clojure values remain on a single thread, keeping GC pointers (`!Send`) safe.

## Status

**Phase A (foundation)** — skeleton crate. `init()` registers the async runtime hook with the
interpreter. No actual async operations work yet; those are implemented in later phases:

- Phase B: `^:async` fn dispatch, `eval_async`, `await` yielding
- Phase C: `deref` enforcement in async context
- Phase D: `timeout`, `alts`, `alt`
- Phase E: `chan`, `put!`, `take!`, `close!`, `go`

## File layout

| File | Description |
|---|---|
| `src/lib.rs` | `init(globals)` entry point |
| `src/runtime.rs` | `AsyncRuntimeImpl` — Tokio-backed `AsyncRuntime` impl |

## Public API

```rust
/// Register the async runtime and load clojure.core.async.
/// Must be called inside a Tokio LocalSet context.
pub fn init(globals: &Arc<GlobalEnv>);
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
