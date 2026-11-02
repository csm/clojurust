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

On native targets every concurrent body runs on a **fiber**: a lightweight
thread with its own stack, driven by the `LocalSet`. Blocking-looking calls —
`<!`, `>!`, `await`, `deref`, `sleep`, file and socket I/O — park the fiber and
let the others run. There is one evaluator, so any function can wait, however
deeply it is nested inside other calls.

| Construct | Returns | Body runs on |
|---|---|---|
| `go` | a result channel | a new fiber |
| `^:async` fn call | a future | a new fiber |
| `future`, `future-call` | a future | a new fiber |
| `thread`, `io-thread`, `thread-call` | a result channel | a new fiber |
| CLI, REPL, nREPL, tests, AOT `main` | — | the root fiber |

wasm32 has no stack switching. There, `go` and `^:async` bodies use a separate
cooperative evaluator, and only `await` (or `<!`/`>!`, macros over `await`)
directly in such a body can wait.

To run Clojure work on more than one core, the single-threaded executor is
instantiated multiple times as independent **isolates**; see the
[Worker isolation](isolation.md) chapter.

## `^:async` functions and `await`

Calling a function tagged `^:async` returns a `Future` *immediately*. The body
runs concurrently on its own fiber, with the ordinary evaluator. The tag no
longer selects a different evaluator on native targets; it only means "calls
run concurrently and return a future". Code written for the older colored model
keeps working unchanged.

```clojure
(defn ^:async fetch [url]
  (let [resp (await (http-get url))]   ; yields until the request resolves
    (:body resp)))

(fetch "https://example.com")          ; returns a Future right away
(await (fetch "https://example.com"))  ; yields until the body is ready
```

`await` is a **special form**. `(await x)` waits for a `Future` or `promise` and
returns its value. On a channel (such as a `go` result) it takes one value,
returning `nil` once the channel is closed and drained. Any other value is
returned unchanged.

- On a native fiber — any `go` or `^:async` body, and any code it calls —
  `(await x)` parks the fiber. It is the same as `(deref x)` for futures.
- On wasm32, `(await x)` inside a `go` or `^:async` body yields the executor.
- Outside a fiber, `(await x)` blocks the OS thread.

### `await` vs. `deref`

In a native fiber, `deref` / `@` parks until the future or promise settles.
CLI evaluation, REPL, nREPL, tests, and AOT programs start in root fibers.
An ordinary function can wait through nested calls without async annotations.

```clojure
(defn answer [] @(future (sleep 10) 42))
@(future (answer))
;; => 42
```

`future` and `future-call` convey dynamic bindings into each child fiber.
A child's binding changes stay private to that child. These fibers run
concurrently on one isolate thread. Isolates provide parallel Clojure execution.
`future-cancel` cancels the result without interrupting the body.

`go` and `^:async` bodies are fibers too, so `deref` works in them exactly like
`await`. A pending local future rejects carrier dereference outside a fiber.
Synchronous embedders use `fiber::run_root` or `Isolate::spawn_root`.
Manual futures with producers on other OS threads retain blocking dereference. On wasm32, callers retain the
async waiting rules.

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

The core.async parking operations take and put directly. On native targets they
park the calling fiber, in a `go` body or anywhere else:

| Operation | Meaning |
|---|---|
| `(<! ch)` | take the next value (or `nil` once closed and drained) |
| `(>! ch v)` | put `v`; returns `true`, or `false` if closed |
| `(alts! [ch [ch v] ...])` | complete exactly one take or put; returns `[value ch]` |

Future-returning variants compose with `await`, `alts`, and `join-all`:

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

The blocking spellings `<!!`, `>!!`, and `alts!!` are the same operations. On a
native fiber they park; outside any fiber they block the OS thread. A take of a
future or promise (such as an `^:async` call's result) waits for its value.

### `go` blocks

`go` runs its body on a new fiber and returns a result channel, as in core.async.
A non-nil result is put on the channel, which then closes. A nil result, or an
exception (which is printed), only closes it:

```clojure
(def in  (chan 1))
(def out (chan 1))

(go (let [v (<! in)]
      (>! out (* v 2))))

(>!! in 21)
(<!! out)    ; => 42

(<!! (go (+ 1 2)))     ; => 3
(await (go :done))     ; => :done — await takes from a channel
```

Because the result is a channel, it works with `alts!`, `merge`, and the other
channel operations.

The older spelling, `(await (take! in))` / `(await (put! out v))`, still works
and is the portable form for code that must also run on wasm32.

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
| `(thread body...)` / `(io-thread body...)` / `(thread-call f)` | run an ordinary body in a fiber and return a result channel |
| `(onto-chan! ch coll)` | put every element of `coll` onto `ch`, then close it |
| `(to-chan! coll)` | return a channel and seed it from `coll` in the background |
| `(merge chs)` | fan several channels into one |
| `(mult ch)` + `(tap! m ch)` / `(untap! m ch)` / `(untap-all! m)` | broadcast one source channel to many taps |
| `(reduce f init ch)` | fold over a channel until it closes |
| `(into coll ch)` | drain a channel into a collection |

## Garbage collection

Because every fiber shares the isolate thread, GC safepoints are cooperative:
the collector runs on the active fiber at a safepoint and traces every
suspended fiber's roots. A background GC-service task (spawned by the crate's `init`)
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
    let runtime = cljrs_runtime::Runtime::builder().build()?;
    cljrs_stdlib::install(&runtime);
    let globals = runtime.into_globals();
    cljrs_async::init(&globals);
    // ... evaluate code ...
}));
```

Code that can park must run on a fiber. From synchronous Rust, drive it with
`cljrs_async::fiber::run_root` (or `Isolate::spawn_root`). From async code
already on the `LocalSet`, `cljrs_async::fiber::eval_form(&form, &mut env).await`
evaluates one form on a new fiber.

The CLI links this crate when built with its default `async` feature; a minimal
binary can be produced with `cargo build -p cljrs --no-default-features`.

> **WASM note.** In the browser REPL, `init` may run before a `LocalSet` exists;
> the GC-service spawn no-ops in that case and should be re-invoked from inside a
> `LocalSet::run_until` block. `timeout` uses the browser's `setTimeout`
> (via `gloo-timers`) on `wasm32` instead of `tokio::time::sleep`.

A failed `thread` or `io-thread` body prints an error and closes its result channel.
Each call uses one fiber. The `io-thread` macro delegates to `thread`.


## Fairness and diagnostics on native targets

CPU loops cooperate with other fibers at function and loop safepoints.
`CLJRS_FIBER_SLICE_US` sets the time slice in microseconds (default 10000).
The driver yields whenever a slice expires, so other fibers and async tasks run.
Native functions must return to a safepoint or use parking operations to let
other fibers run.

If native code blocks a carrier while another fiber is ready, the watchdog
prints the fiber ID and native function name. `CLJRS_FIBER_STALL_MS` sets the
warning threshold in milliseconds (default 100). Each stall produces one warning.
Only waiting fibers are detected; a stall that delays only other async tasks is silent.

`--gc-stats` includes live and peak fiber counts, pooled stacks, and reserved
stack bytes. The memory governor charges active and pooled reservations,
including guard pages, as `fiber-stack`. These are virtual reservations, not
resident memory measurements. Strict memory budgets must include the root's
64 MiB stack and each child's stack (`CLJRS_FIBER_STACK_KB`, default 1024, minimum 512).

A recursion error reports the fiber ID and remaining stack space. Safepoints
keep a 256 KiB margin for interpreter frames and error construction. Native
recursion that bypasses safepoints can still hit the fatal guard page.
