# core.async

The `clojure.core.async` namespace provides CSP-style concurrency — channels,
`go` blocks, and the `^:async`/`await` function model — implemented on a Tokio
`current_thread` runtime and `LocalSet`. It is provided by the `cljrs-async`
crate and loaded automatically by the `cljrs` CLI.

```clojure
(require '[clojure.core.async :refer [chan go take! put! close! timeout alts]])
```

## The execution model

All async tasks cooperate on a **single executor thread**. This is a deliberate
choice: Clojure values are managed by a tracing GC whose pointers are `!Send`,
so they cannot be moved between threads. Running every async task on one thread
via `LocalSet` keeps those pointers sound and keeps garbage collection simple —
"stop the world" just means "finish the current poll, collect, resume."

Two tiers coexist:

| Tier | Runs on | Used by |
|---|---|---|
| Async (single thread) | Tokio `LocalSet` | `^:async` fns, `go`, channels, `timeout`, `alts` |
| Parallel (thread pool) | OS threads | `future`, `pmap`, `agent` (unchanged core primitives) |

Both tiers produce `Value::Future`, so a caller can `await` or `deref` either.

To run Clojure work on more than one core, the single-threaded executor is
instantiated multiple times as independent **isolates**; see the
[Worker isolation](isolation.md) chapter.

## `^:async` functions and `await`

A function tagged `^:async` runs its body as an async task. Calling it returns
a `Future` *immediately*; the body runs cooperatively, yielding the executor at
every `await`.

```clojure
(defn ^:async fetch [url]
  (let [resp (await (http-get url))]   ; yields until the request resolves
    (:body resp)))

(fetch "https://example.com")          ; returns a Future right away
(await (fetch "https://example.com"))  ; yields until the body is ready
```

`await` is a **special form**, not a function — it is syntactically detectable
so the compiler can recognise yield points. Its behaviour depends on context:

- Inside an `^:async` body (driven by the async evaluator), `(await x)` yields
  the executor thread until `x` — a `Future`, a `promise`, or a channel op —
  resolves, then returns the resolved value.
- Outside any async context, `(await x)` falls back to a **blocking** deref of
  the value, so the form still works in synchronous code.

`^:async` is *viral*: a function that uses `await` should itself be `^:async`.

### `await` vs. `deref`

`deref` / `@` always blocks the calling OS thread. Calling `deref` on a future
**inside** an `^:async` body is a runtime error:

```clojure
(defn ^:async bad [f]
  @f)            ; error: use (await ...) instead of deref inside an ^:async function
```

This enforcement (via the interpreter's async-context flag) prevents the
classic deadlock where a task blocks the only executor thread waiting on a
future that can only be resolved by that same thread.

## Channels

Channels are CSP conduits between tasks. They are implemented as `NativeObject`s
(`CljChannel`) rather than a dedicated `Value` variant, keeping the core value
model free of async concerns.

```clojure
(chan)      ; unbuffered — a rendezvous channel
(chan 0)    ; same as (chan)
(chan 10)   ; buffered, capacity 10
```

- An **unbuffered** (rendezvous) channel hands a value directly from a `put!` to
  a `take!`: the `put!` resolves `true` only once a taker consumes the value.
- A **buffered** channel accepts `put!`s while it has room and serves `take!`s
  while it is non-empty.
- A **closed** channel drains any buffered values, then `take!` yields `nil` and
  `put!` resolves `false`.

### Channel operations

Operations that can block return a `Value::Future`, so they are used with
`await` inside an async context:

| Operation | Meaning |
|---|---|
| `(take! ch)` | await the next value (or `nil` once closed and drained) |
| `(put! ch v)` | await acceptance of `v`; resolves `true`, or `false` if closed |
| `(close! ch)` | close the channel (idempotent) |

Non-blocking variants act synchronously and return immediately:

| Operation | Meaning |
|---|---|
| `(poll! ch)` | take a buffered value now, or `nil` if none is ready |
| `(offer! ch v)` | put `v` now if there is buffer room → `true`/`false` |

Blocking variants park the OS thread and are meant for the REPL, tests, and
other synchronous contexts — **not** for use inside an `^:async` body or from
the single-threaded executor thread (they deadlock there):

| Operation | Meaning |
|---|---|
| `(<!! ch)` | blocking take |
| `(>!! ch v)` | blocking put |

### `go` blocks

`go` spawns its body as an anonymous async task and returns a `Future`:

```clojure
(def in  (chan 1))
(def out (chan 1))

(go (let [v (await (take! in))]
      (await (put! out (* v 2)))))

(await (put! in 21))
(await (take! out))    ; => 42
```

## Selection: `timeout`, `alts`, `alt`

`timeout` returns a future that delivers `nil` after a delay:

```clojure
(timeout 5000)   ; => Future resolving to nil after 5 s
```

`alts` waits on a vector of futures (or channel ops) and returns
`[value index]` for whichever resolves first:

```clojure
(let [[v i] (await (alts [(take! ch) (timeout 1000)]))]
  (if (= i 1) (println "timed out") (println "got" v)))
```

`alt` is a macro that pairs each future with a handler, awaits `alts`, and
dispatches to the matching handler:

```clojure
(alt
  (take! ch1)   (fn [v] (println "ch1:" v))
  (take! ch2)   (fn [v] (println "ch2:" v))
  (timeout 500) (fn [_] (println "timed out")))
```

## Higher-level utilities

| Function | Description |
|---|---|
| `(join-all futs)` | await a seq of futures, returning a vector of results (like `Promise.all`) |
| `(async-pmap f coll)` | spawn `f` over `coll` concurrently and await all results |
| `(thread f)` / `(thread-call f)` | run `f` on a real OS thread (`spawn_blocking`); deliver its result over a channel |
| `(onto-chan! ch coll)` | put every element of `coll` onto `ch`, then close it |
| `(to-chan! coll)` | return a channel and seed it from `coll` in the background |
| `(merge chs)` | fan several channels into one |
| `(mult ch)` + `(tap! m ch)` / `(untap! m ch)` / `(untap-all! m)` | broadcast one source channel to many taps |
| `(reduce f init ch)` | fold over a channel until it closes |
| `(into coll ch)` | drain a channel into a collection |

## Garbage collection

Because the async tier is single-threaded, GC safepoints are cooperative: the
runtime collects between poll cycles, and `await` invokes a safepoint before
each yield. A background GC-service task (spawned by the crate's `init`)
services collection requests, and explicit root guards keep spawned task
futures and their captured environments reachable while they are in flight.

## Embedding from Rust

`cljrs-async` is a standalone crate. Call `init` from inside a `LocalSet`
context, then evaluate code as usual:

```rust
let rt = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
let local = tokio::task::LocalSet::new();
rt.block_on(local.run_until(async {
    let globals = cljrs_stdlib::standard_env();
    cljrs_async::init(&globals);
    // ... evaluate code ...
}));
```

The CLI links this crate when built with its default `async` feature; a minimal
binary can be produced with `cargo build -p cljrs --no-default-features`.

> **WASM note.** In the browser REPL, `init` may run before a `LocalSet` exists;
> the GC-service spawn no-ops in that case and should be re-invoked from inside a
> `LocalSet::run_until` block. `timeout` uses the browser's `setTimeout`
> (via `gloo-timers`) on `wasm32` instead of `tokio::time::sleep`.
