# Async Support Plan for clojurust

## Design Philosophy

Async support is delivered as **`cljrs-async`**, a separate Rust crate. The core crates
(`cljrs-value`, `cljrs-env`, `cljrs-interp`, `cljrs-eval`, `cljrs-builtins`) contain no
`#[cfg(feature = "async")]` guards and no Tokio dependency. They expose a thin hook trait that the
async library registers into at startup. The CLI binary links `cljrs-async` by default; embedders
opt in via `Cargo.toml`.

This mirrors how `clojure.core.async` ships as a separate JAR: the runtime knows nothing about
channels until the library is on the classpath.

---

## Crate Layout

```
cljrs-env          ← adds AsyncRuntime hook trait (always present, zero cost if unused)
cljrs-async        ← NEW: Tokio runtime, channels, eval_async, clojure.core.async namespace
cljrs (CLI)        ← feature "async" (default = on) links cljrs-async and calls init()
```

### Core hook — `cljrs-env`

A single trait object slot on `Env`. No Tokio import, no feature flag:

```rust
// cljrs-env/src/env.rs

pub trait AsyncRuntime: Send + Sync {
    /// Spawn an ^:async fn body as a LocalSet task. Returns Value::Future immediately.
    fn spawn_async(&self, body: Form, env: Env) -> Value;
    /// Yield to the executor until `val` (a Future or Promise) is resolved.
    /// Called by the `await` special form inside an async context.
    fn await_value<'a>(&'a self, val: Value) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, CljError>> + 'a>>;
}

pub struct Env {
    // ...existing fields unchanged...
    pub async_rt: Option<Arc<dyn AsyncRuntime>>,
}
```

No other core crate changes for the hook mechanism.

### `await` special form in core — `cljrs-interp`

`await` is a special form so it is syntactically detectable for IR analysis (Phase H). Its
implementation dispatches through the hook:

```rust
// cljrs-interp/src/special_forms.rs
SpecialForm::Await => {
    let val = eval(&args[0], env)?;
    match &env.async_rt {
        Some(rt) => {
            // Yielding path — inside an async context with cljrs-async loaded.
            // eval_async (in cljrs-async) handles the actual .await call;
            // the special form handler here just signals the async eval path.
            Ok(Value::AsyncYield(Box::new(val)))  // sentinel consumed by eval_async
        }
        None => {
            // Blocking fallback — no async runtime registered.
            // await on a Future/Promise blocks the OS thread (same as deref).
            blocking_deref(val, None)
        }
    }
}
```

Without `cljrs-async` loaded, `(await future)` is equivalent to `(deref future)` — it blocks
the calling thread. This is useful and correct for sync code that wants to force a future.

### The `cljrs-async` crate

```toml
# crates/cljrs-async/Cargo.toml
[package]
name = "cljrs-async"
version = "0.1.0"

[dependencies]
cljrs-env      = { workspace = true }
cljrs-value    = { workspace = true }
cljrs-interp   = { workspace = true }
cljrs-builtins = { workspace = true }
tokio          = { workspace = true, features = ["rt", "sync", "time", "task", "macros"] }
futures-util   = { workspace = true }
```

Exports one public entry point:

```rust
/// Register the async runtime with the interpreter environment and load
/// the clojure.core.async namespace into it.
pub fn init(env: &mut Env) { ... }
```

Internally contains:
- `AsyncRuntimeImpl` implementing `AsyncRuntime` (Tokio `LocalSet`-backed)
- `eval_async` — the Rust `async fn` evaluation path for `^:async` fn bodies
- `CljChannel` — channel type as a `NativeObject` (no `Value` variant needed in core)
- All `clojure.core.async` builtins registered as native functions
- Tokio runtime and `LocalSet` lifecycle management

### CLI — `cljrs`

```toml
# crates/cljrs/Cargo.toml
[features]
default = ["async"]
async   = ["dep:cljrs-async"]

[dependencies]
cljrs-async = { workspace = true, optional = true }
```

```rust
// crates/cljrs/src/main.rs
fn main() {
    #[cfg(feature = "async")]
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        rt.block_on(local.run_until(async_main()));
        return;
    }
    #[cfg(not(feature = "async"))]
    sync_main();
}

#[cfg(feature = "async")]
async fn async_main() {
    let mut env = cljrs_env::Env::new();
    cljrs_async::init(&mut env);   // ← one call, wires everything up
    // ... run file / repl / eval as before
}
```

The `#[cfg(feature = "async")]` appears **only** in `crates/cljrs/`. Every other crate is clean.
A minimal CLI binary can be built with `cargo build -p cljrs --no-default-features`.

### Rust embedders

```toml
# user's Cargo.toml
[dependencies]
cljrs       = "0.1"
cljrs-async = "0.1"   # opt in explicitly
tokio       = { version = "1", features = ["rt", "macros"] }
```

```rust
#[tokio::main]
async fn main() {
    let local = tokio::task::LocalSet::new();
    local.run_until(async {
        let mut env = cljrs::Env::new();
        cljrs_async::init(&mut env);
        cljrs::run_file(&mut env, "main.cljrs").await;
    }).await;
}
```

---

## Current State

The codebase already has solid groundwork:
- **Tokio 1.50.0** is in workspace dependencies (unused — stays that way in core)
- `Value::Future(GcPtr<CljxFuture>)` exists (thread-pool based, `Mutex<FutureState>` + `Condvar`)
- `Value::Promise(GcPtr<CljxPromise>)` exists (Condvar-based)
- `Value::Agent(GcPtr<Agent>)` exists (`std::sync::mpsc::SyncSender`, worker thread)
- `cljrs-stdlib/src/core_async.rs` has a commented-out skeleton — migrated into `cljrs-async`
- **Name collision**: `await` already exists as an agent operation (`(await agent)`) — must be
  renamed to `await-agent` before the `await` special form is added

### What stays unchanged in core

- All existing concurrency primitives (`future`, `promise`, `atom`, `agent`) keep their
  thread-based implementations exactly as-is
- `CljxFuture` internals (`Mutex<FutureState>` + `Condvar`) are not modified
- `CljxPromise` stays Condvar-based
- `Agent` stays on its own OS thread with `std::sync::mpsc`
- `deref` on futures continues to block via `Mutex` + `Condvar`
- No Tokio dependency anywhere in core

### What `cljrs-async` adds

- `^:async` function call dispatch (spawning on LocalSet)
- `await` special form yielding path (via the `AsyncRuntime` hook)
- `chan`, `put!`, `take!`, `close!`, `go`, `timeout`, `alts`, `alt`
- `async-pmap`, `join-all`
- `CljChannel` as a `NativeObject`

---

## The Core Problem: GC + Async

`GcPtr<T>` is a raw pointer (`NonNull<GcBox<T>>`), which is `!Send`. Rust's multi-threaded Tokio
executor requires futures to be `Send`. We cannot naively `tokio::spawn` a task holding Clojure
values.

**Decision: Use `tokio::task::LocalSet` as the async executor.** All Clojure async tasks run on a
single-threaded local executor; `spawn_local` requires no `Send`. This is correct for I/O-bound
async code. CPU-bound parallelism stays on the thread pool (existing `future` behavior).

Because all async tasks are single-threaded on the LocalSet, GC stop-the-world remains simple:
"finish the current poll cycle, collect, resume." No safepoint handshake protocol needed.

---

## Execution Model

```
Thread 1 (main / LocalSet)          Thread Pool (existing, unchanged)
──────────────────────────           ─────────────────────────────────
tokio LocalSet executor              std::thread per future
├─ ^:async fn calls                  ├─ (future ...) macro
├─ await / alt / alts                ├─ pmap (parallel CPU work)
├─ channel put!/take!                └─ agent actions
├─ go blocks
└─ timeout / sleep

GC safepoints: finish current poll → check GC flag → collect → resume
```

---

## Value Model

`Value::Channel` is **not** added to `cljrs-value`. Channels live as `NativeObject`s inside
`cljrs-async`, keeping the core `Value` enum free of async concerns:

```rust
// cljrs-async/src/channel.rs
pub struct CljChannel {
    sender:   tokio::sync::mpsc::Sender<Value>,
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Value>>,
    capacity: Option<usize>,   // None = rendezvous
    closed:   AtomicBool,
}

impl NativeObject for CljChannel {
    fn type_tag(&self) -> &str { "Channel" }
    fn as_any(&self) -> &dyn Any { self }
}

impl Trace for CljChannel {
    fn trace(&self, _: &mut MarkVisitor) {}  // no GcPtr fields
}
```

`(chan)` returns `Value::NativeObject(GcPtr<NativeObjectBox>)`. Channel protocol functions
downcast via `as_any().downcast_ref::<CljChannel>()`.

The async-upgraded `CljxFuture` internals also live in `cljrs-async` as a wrapper/extension
rather than replacing the core type.

---

## Phase A — Foundation

**Crates touched**: `cljrs-env`, `cljrs-interp`, `cljrs-builtins`, new `cljrs-async`, `cljrs`

1. **Add `AsyncRuntime` trait and `async_rt` slot to `Env`** in `cljrs-env`. No feature flag,
   no conditional compilation. The slot is `Option<Arc<dyn AsyncRuntime>>` — `None` by default.

2. **Add `await` as a special form** in `cljrs-interp/src/special_forms.rs`. Dispatches through
   `env.async_rt` if present; falls back to blocking deref of `Value::Future`/`Value::Promise`
   if not. This makes `(await future)` useful in sync code too.

3. **Rename agent's `await`** to `await-agent` in `cljrs-stdlib` and `cljrs-builtins`. Update
   any bootstrap Clojure code that uses `(await agent)`.

4. **Create `cljrs-async` crate skeleton**: `Cargo.toml`, `src/lib.rs` with `pub fn init(env)`,
   `AsyncRuntimeImpl` stub, `CljChannel` type, empty `clojure.core.async` namespace registration.

5. **Wire up the CLI**: add optional `cljrs-async` dep to `cljrs/Cargo.toml` with
   `default = ["async"]`, update `main.rs` with the two-path entry point shown above.

6. **Workspace `Cargo.toml`**: mark `tokio` and `futures-util` as non-optional at workspace level
   (they are always available for crates that want them); `cljrs-async` uses them directly without
   needing workspace-level optional gating since it is itself an optional dep.

---

## Phase B — Async Functions (`^:async`)

**Crate**: `cljrs-async`

### Metadata Propagation

The reader already handles metadata maps. `^:async` desugars to `^{:async true}`. No reader
changes needed. The interpreter checks `fn.meta().get(":async") == Some(true)` at call time.

### Async Context Flag

```rust
// cljrs-interp (or cljrs-env) — already has EvalCtx
pub struct EvalCtx {
    pub is_async: bool,   // add this field
    // ...existing fields
}
```

`is_async` tells `await` whether it is in a context where yielding is valid.

### Dual Eval Path

```rust
// cljrs-async/src/eval_async.rs

// Sync — existing, unchanged, in cljrs-interp
pub fn eval(form: &Form, env: &mut Env) -> Result<Value, CljError>

// Async — lives in cljrs-async; delegates to eval for non-yield forms
pub async fn eval_async(form: &Form, env: &mut Env) -> Result<Value, CljError>
```

`eval_async` calls `eval` for all forms. When it encounters a `Value::AsyncYield` sentinel from
the `await` special form handler, it performs the actual `.await` call against the Tokio future.

### Calling an `^:async` fn

```rust
// cljrs-async/src/runtime.rs — inside AsyncRuntimeImpl::spawn_async
let captured_env = env.clone_for_async(); // clone env, set is_async = true
let jh = tokio::task::spawn_local(async move {
    eval_async(&fn_body, &mut captured_env).await
});
Value::Future(CljxFuture::from_join_handle(jh))
```

The call returns a `Value::Future` immediately. `CljxFuture::from_join_handle` wraps the
`JoinHandle` using a Tokio-aware future implementation defined in `cljrs-async`.

### User-facing syntax

```clojure
(defn ^:async fetch [url]
  (let [resp (await (http/get url))]
    (:body resp)))

;; Calling (fetch url) returns a Future immediately.
;; (await (fetch url)) yields in async context.
;; (deref (fetch url)) blocks in sync context.
```

---

## Phase C — `await` and `deref`

**Crates**: `cljrs-interp` (already done in Phase A), `cljrs-builtins`

### `deref` / `@` for Futures (Sync Context)

```clojure
(deref future)                     ; blocks thread indefinitely
(deref future 5000 :timeout-val)   ; blocks with timeout
```

```rust
// cljrs-builtins — deref dispatch on Value::Future (unchanged from today)
Value::Future(f) => {
    if ctx.is_async {
        return Err(runtime_err(
            "use (await ...) instead of deref inside an ^:async function"
        ));
    }
    f.blocking_deref(timeout).map_err(into_cljrs_err)
}
```

`blocking_deref` on `CljxFuture` parks the OS thread via `Condvar` (existing implementation) or,
if a Tokio runtime handle is available on this thread, via `Handle::block_on`. The core
implementation is unchanged; `cljrs-async` may override via the `AsyncRuntime` hook if needed.

---

## Phase D — `timeout`, `alts`, and `alt`

**Crate**: `cljrs-async`

### `timeout`

```clojure
(timeout ms)  ; => Future that delivers nil after ms milliseconds
```

```rust
// cljrs-async/src/builtins.rs
fn clj_timeout(ms: i64) -> Value {
    let jh = tokio::task::spawn_local(async move {
        tokio::time::sleep(Duration::from_millis(ms as u64)).await;
        Ok(Value::Nil)
    });
    Value::Future(CljxFuture::from_join_handle(jh))
}
```

### `alts` — Dynamic Future Selection

```clojure
(alts [f1 f2 (timeout 5000)])
; => [value index]   ; index = which future completed first
```

`tokio::select!` requires statically-known branches. Use `futures_util::future::select_all` for
dynamic dispatch:

```rust
async fn clj_alts(futures: Vec<Value>) -> Value {
    let indexed: Vec<_> = futures
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            Box::pin(async move {
                let result = async_await_value(v).await;
                (result, i)
            })
        })
        .collect();
    let ((val, idx), _, _) = futures_util::future::select_all(indexed).await;
    Value::Vector(pvec![val.unwrap_or(Value::Nil), Value::Long(idx as i64)])
}
```

Returns a two-element vector `[value index]`.

### `alt` — Pattern-Matched Selection (Macro)

```clojure
(alt
  f1            (fn [v] (println "f1 got" v))
  f2            (fn [v] (println "f2 got" v))
  (timeout 500) (fn [_] (println "timed out")))
```

`alt` is a Clojure macro in `clojure.core.async`. It evaluates all future expressions, calls
`alts`, then dispatches to the matching handler:

```clojure
;; Macroexpansion:
(let [__futs    [f1 f2 (timeout 500)]
      __handlers [(fn [v] ...) (fn [v] ...) (fn [_] ...)]
      [__val __idx] (alts __futs)]
  ((nth __handlers __idx) __val))
```

`alt` also handles channel operations encoded as tagged vectors:

```clojure
(alt
  [ch1 :take]    (fn [v] ...)
  [ch2 :put val] (fn [_] ...))
```

The runtime converts `:take`/`:put` entries into the appropriate futures before calling `alts`.

---

## Phase E — Channel Primitives

**Crate**: `cljrs-async/src/channel.rs` and `cljrs-async/src/core_async.cljrs`

```clojure
;; Creation
(chan)          ; unbuffered (rendezvous)
(chan 10)       ; buffered, capacity 10
(chan 10 xf)    ; buffered with transducer

;; Async operations (^:async / go context)
(take! ch)      ; await value; parks coroutine until value available
(put! ch val)   ; send value; parks if buffer full
(close! ch)     ; close; pending takes receive nil

;; Blocking operations (sync context)
(take!! ch)     ; parks OS thread
(put!! ch val)  ; parks OS thread

;; Non-blocking (returns nil / false immediately)
(poll! ch)
(offer! ch val)
```

**Implementation notes**:
- Buffered: `tokio::sync::mpsc::channel(capacity)`
- Unbuffered (rendezvous): capacity-1 channel with a semaphore for sender backpressure, or a
  dedicated rendezvous struct using paired oneshots
- `close!`: set `closed` flag; sender drops cause receivers to observe channel close
- `take!!`/`put!!`: use `tokio::runtime::Handle::block_on` to park the OS thread

### `go` Macro

```clojure
(go
  (let [v (take! ch)]
    (put! result-ch (* v 2))))
; => Future<nil or block return value>
```

`go` is syntactic sugar for spawning an anonymous `^:async` fn:

```clojure
;; Macroexpansion:
(clojure.core.async/async-spawn (fn ^:async [] (let [v (take! ch)] (put! result-ch (* v 2)))))
```

`async-spawn` is a native function in `cljrs-async` that calls `AsyncRuntimeImpl::spawn_async`.

---

## Phase F — Higher-Level Async Utilities

**Crate**: `cljrs-async`

### `async-pmap` and `join-all`

Existing `pmap` (thread-pool) is unchanged in core. `cljrs-async` adds:

```clojure
(defn ^:async async-pmap [f coll]
  (let [futs (map (fn [x] (async-spawn (fn ^:async [] (await (f x))))) coll)]
    (await (join-all futs))))
```

`join-all` is a native builtin that awaits a sequence of futures and returns a vector of results
(similar to `Promise.all`).

### Agent and Promise — No Changes

The existing `Agent` (OS thread + `std::sync::mpsc`) and `CljxPromise` (Condvar) are not
modified. They work correctly with the async tier: agent actions that are `^:async` fns can be
`await`-ed by callers. If a future demand for async-native agents arises, `async-agent` can be
added to `cljrs-async` as a separate type without touching the core `Agent`.

---

## Phase G — GC Integration

**Crates**: `cljrs-gc`, `cljrs-async` (eval loop)

### Safepoints at Async Yields

With `LocalSet`, all async tasks cooperate on one thread. Stop-the-world means "finish the current
poll, then check for GC before the next poll":

```rust
// cljrs-async/src/runtime.rs — LocalSet driver loop
loop {
    local_set.run_until_stalled().await;
    if GC_REQUESTED.load(Ordering::Acquire) {
        gc::collect_now();  // safe: no tasks running, single thread
        GC_REQUESTED.store(false, Ordering::Release);
    }
}
```

### Root Scanning for Async Closures

Values captured in `spawn_local` closures (Rust compiler-generated async state machines) must be
reachable as GC roots. Start with conservative stack scanning of the LocalSet thread, consistent
with the existing stop-the-world approach. Precise tracking (register `GcPtr`s on spawn, deregister
on completion) can be added later if needed.

---

## Phase H — IR and Compiler Support

**Crates**: `cljrs-ir`, `cljrs-eval`, `cljrs-async`

### IR Additions

```rust
// cljrs-ir/src/lib.rs
pub enum IrInstr {
    // ...existing
    Await  { src: Reg, dst: Reg },                    // yield point in async IrFunctions
    Spawn  { fn_reg: Reg, args: Vec<Reg>, dst: Reg }, // spawn_local
    ChanTake { chan: Reg, dst: Reg },
    ChanPut  { chan: Reg, val: Reg },
}

pub struct IrFunction {
    pub is_async: bool,
    // ...existing
}
```

### IR Interpreter Strategy

- **Short-term**: async IR functions fall back to tree-walking (`eval_async` in `cljrs-async`)
  rather than the IR interpreter. This avoids building a full async state machine in IR.
- **Long-term (Phase 10 JIT)**: async functions compile to Cranelift state machines with explicit
  resume points at `Await` instructions, matching Rust's generated async code structure.

### Static Async Checking (Phase H+)

Once IR lowering tracks `is_async`, the compiler can:
- Error at compile time if `await` appears outside an `^:async` function
- Warn if an `^:async` function never actually awaits anything
- Infer `^:async` on functions that call other `^:async` functions (opt-in, off by default)

---

## Additional `clojure.core.async` Functions to Implement

All of these live in `cljrs-async`.

| Function | Priority | Rust Backend | Notes |
|---|---|---|---|
| `thread` | High | `spawn_blocking` | real OS thread; returns channel with result |
| `thread-call` | High | `spawn_blocking` | lower-level `thread` |
| `pipeline` | High | `spawn_local` + channels | parallel transducer pipeline |
| `pipeline-async` | High | `spawn_local` tasks | async transducer pipeline |
| `pipeline-blocking` | Medium | `spawn_blocking` | CPU-bound pipeline |
| `merge` | Medium | fan-in with `alts` | merge N channels into 1 |
| `onto-chan!` | Medium | write coll to chan | seed a channel from a collection |
| `to-chan!` | Medium | lazy-seq → chan | streaming source |
| `mult` / `tap` / `untap` | Medium | `broadcast::channel` | one-to-many fanout |
| `pub` / `sub` / `unsub` | Medium | broadcast + topic filter | topic-based pub/sub |
| `mix` | Low | custom fan-in | weighted/muted source mixing |
| `reduce` | Low | fold over channel | aggregate until closed |
| `into` | Low | drain channel to coll | materialize channel to vector |
| `map<` / `map>` | Low | channel transducer | transform values on a channel |
| `filter<` / `filter>` | Low | channel transducer | filter values on a channel |

---

## Key Design Constraints

1. **No `#[cfg(feature = "async")]` in core crates.** The only `#[cfg(feature = "async")]`
   appears in `crates/cljrs/src/main.rs` and `crates/cljrs/Cargo.toml`.

2. **`cljrs-async` is a real, standalone crate.** It can be used by Rust embedders who never
   touch the CLI. The CLI feature flag is a convenience wrapper around this crate, not the
   implementation itself.

3. **`await` is a special form, not a function.** It must be syntactically detectable to emit IR
   yield points and for static async checking in Phase H.

4. **`LocalSet` throughout async.** No `Value` crosses thread boundaries in async code.
   `spawn_blocking` (for CPU-bound work) communicates results back via channels or `Value::Future`.

5. **`deref` is always blocking.** It parks the OS thread, never the async executor. Using `deref`
   on a future inside `^:async` is a runtime error (eventually a compile-time error in Phase H).

6. **`^:async` is viral.** A function using `await` must itself be `^:async`. The interpreter
   enforces this at call time; the compiler enforces it statically in Phase H.

7. **Existing `future` macro is unchanged.** It continues to spawn OS threads. Thread-futures and
   async-futures are both `Value::Future`; callers can `deref` or `await` either.

8. **Core concurrency primitives are unchanged.** `Agent`, `CljxPromise`, and `CljxFuture` keep
   their `std`-based implementations. No Tokio types leak into `cljrs-value`.

9. **Channels are `NativeObject`s.** `Value::Channel` is not added to the core `Value` enum.
   Channel functions downcast via `NativeObject::as_any()`.

10. **GC stop-the-world is safe with `LocalSet`.** Single-threaded cooperative scheduling means
    "stop the world" = "finish current poll, run GC, resume." No locks or safepoint handshake
    protocol required for the async tier.

---

## Implementation Phases Summary

| Phase | Deliverable | Key Crates |
|---|---|---|
| A | `AsyncRuntime` hook in `Env`; `await` special form (with blocking fallback); rename `await-agent`; `cljrs-async` skeleton; CLI wiring | `cljrs-env`, `cljrs-interp`, `cljrs-builtins`, new `cljrs-async`, `cljrs` |
| B | `^:async` fn dispatch via hook; `eval_async`; `EvalCtx::is_async` | `cljrs-async`, `cljrs-interp` |
| C | `deref` blocking enforcement; async/sync context error | `cljrs-builtins` |
| D | `timeout`, `alts`, `alt` macro | `cljrs-async` |
| E | `chan`, `put!`/`take!`, `close!`, `go` macro; `CljChannel` NativeObject | `cljrs-async` |
| F | `async-pmap`, `join-all`; higher-level utilities | `cljrs-async` |
| G | GC safepoints at async yield; async root scanning | `cljrs-gc`, `cljrs-async` |
| H | IR `Await`/`Spawn` instructions; `IrFunction::is_async`; static async checking | `cljrs-ir`, `cljrs-eval`, `cljrs-async` |
