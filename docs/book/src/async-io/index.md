# Async & I/O

clojurust ships asynchronous concurrency and non-blocking file I/O as two
optional crates that layer on top of the interpreter:

| Crate | Namespace | Provides |
|---|---|---|
| [`cljrs-async`](async.md) | `clojure.core.async` | CSP channels, `go` blocks, `^:async`/`await`, `timeout`, `alts`/`alt`, pipelines |
| [`cljrs-io`](io.md) | `clojure.rust.io.async` | non-blocking file reads and writes delivered over core.async channels |

Both are wired into the `cljrs` CLI by default (via its `async` feature) and
can be embedded by Rust programs that link the crates directly.

## Design at a glance

Async support is delivered as a **separate library**, mirroring how
`clojure.core.async` ships as its own JAR on the JVM. The core interpreter
crates contain no Tokio dependency and no `#[cfg(feature = "async")]` guards;
they expose a single hook trait (`AsyncRuntime`) that `cljrs-async` registers
into the environment at startup. The only conditional compilation lives in the
CLI binary.

The whole async tier runs on a Tokio **`current_thread` runtime driving a
`LocalSet`**. Every Clojure value stays on one thread, which keeps the
garbage collector's `!Send` pointers (`GcPtr<T>`) sound — no value ever crosses
a thread boundary inside async code. CPU-bound parallelism continues to use the
existing thread-based `future`, `pmap`, and `agent` primitives, which are
unchanged.

## Two ways to wait

clojurust distinguishes async waiting from blocking waiting:

- **`await`** is a special form. Inside an `^:async` function (or a `go`
  block), it *yields* the single executor thread to other tasks until the value
  it is given — a `Future`, a `promise`, or a channel operation — resolves.
- **`deref` / `@`** *blocks* the calling OS thread. Using `deref` on a future
  from within an `^:async` body is a runtime error that steers you to `await`.

Without `cljrs-async` loaded, `(await x)` falls back to a blocking deref, so the
form is still meaningful in purely synchronous code.

To scale Clojure work across CPU cores, that single-threaded executor is
instantiated multiple times as independent **isolates**, each with its own heap
and collector. Values move between isolates by an explicit copy rather than a
shared pointer. See the [Worker isolation](isolation.md) chapter for the model
and its rationale.

See the [core.async](async.md) chapter for the concurrency model, the
[Worker isolation](isolation.md) chapter for scaling across cores, and the
[Asynchronous I/O](io.md) chapter for the channel-oriented filesystem API.
