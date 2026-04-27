# Async Support Plan for clojurust

## Feature Flag

All async support is gated behind a `async` Cargo feature. Tokio is a substantial dependency
(compile time, binary size, transitive deps) and embedders who only need the interpreter or AOT
compiler should not pay for it. The feature is **off by default**.

### Workspace `Cargo.toml`

```toml
[workspace.dependencies]
# Mark tokio as optional at the workspace level
tokio = { version = "1.50.0", optional = true, features = ["rt", "sync", "time", "task", "macros"] }
futures-util = { version = "0.3", optional = true }  # needed for select_all in alts
```

### Per-crate feature declaration

Every crate that touches async machinery declares the feature and gates tokio behind it:

```toml
# cljrs-value/Cargo.toml
[features]
async = ["dep:tokio"]

[dependencies]
tokio = { workspace = true, optional = true }
```

```toml
# cljrs-interp/Cargo.toml
[features]
async = ["dep:tokio", "cljrs-value/async"]

[dependencies]
tokio = { workspace = true, optional = true }
```

```toml
# cljrs-stdlib/Cargo.toml
[features]
async = ["dep:tokio", "dep:futures-util", "cljrs-interp/async"]
```

```toml
# cljrs (CLI binary) — enables the feature for end-users who want it
[features]
default = []          # async off by default
async = ["cljrs-interp/async", "cljrs-stdlib/async", "cljrs-value/async"]
```

The CLI binary can be built with `cargo build --features async` (or `cargo build -F async`). A
future `cljrs-full` meta-crate or a `full` feature alias can bundle it for convenience.

### Code-level gating

All async-only types, impls, and functions are wrapped in `#[cfg(feature = "async")]`:

```rust
// cljrs-value/src/value.rs
pub enum Value {
    Future(GcPtr<CljxFuture>),     // always present; internals differ by feature
    #[cfg(feature = "async")]
    Channel(GcPtr<CljxChannel>),
    // ...
}
```

```rust
// cljrs-value/src/types.rs
pub struct CljxFuture {
    #[cfg(not(feature = "async"))]
    inner: FutureThreadBased,       // existing Mutex<FutureState> + Condvar

    #[cfg(feature = "async")]
    inner: Arc<FutureShared>,       // tokio OnceCell + Notify
    #[cfg(feature = "async")]
    cancel: Option<tokio::task::AbortHandle>,
}
```

```rust
// cljrs-interp/src/special_forms.rs
#[cfg(feature = "async")]
SpecialForm::Await => { ... }

// Without the feature, `await` parses as an unknown symbol rather than a special form,
// giving a clear error: "await requires the async feature"
```

```rust
// cljrs/src/main.rs
fn main() {
    #[cfg(feature = "async")]
    {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all().build().unwrap();
        let local = tokio::task::LocalSet::new();
        rt.block_on(local.run_until(async_main()));
        return;
    }
    #[cfg(not(feature = "async"))]
    sync_main();
}
```

### What works without the feature

- All existing concurrency primitives (`future`, `promise`, `atom`, `agent`) keep their current
  thread-based implementations untouched
- `deref` on futures continues to work via `Mutex` + `Condvar`
- No tokio dependency, no LocalSet, no extra compile time

### What requires the feature

- `^:async` function metadata (runtime error if used without the feature)
- `await` special form
- `alt` / `alts` / `timeout`
- `chan`, `put!`, `take!`, `go`
- Upgraded `CljxFuture` internals (tokio-native)
- Upgraded `Agent` (tokio actor)
- `async-pmap`, `join-all`

---

## Current State

The codebase already has solid groundwork:
- **Tokio 1.50.0** is in workspace dependencies (unused)
- `Value::Future(GcPtr<CljxFuture>)` exists (thread-pool based, `Mutex<FutureState>` + `Condvar`)
- `Value::Promise(GcPtr<CljxPromise>)` exists (Condvar-based)
- `Value::Agent(GcPtr<Agent>)` exists (`std::sync::mpsc::SyncSender`, worker thread)
- `cljrs-stdlib/src/core_async.rs` has a commented-out skeleton (tokio `go`/channel sketch)
- **Name collision**: `await` already exists as an agent operation (`(await agent)`) — must be renamed to `await-agent` or `await-for`

---

## The Core Problem: GC + Async

`GcPtr<T>` is a raw pointer (`NonNull<GcBox<T>>`), which is `!Send`. Rust's multi-threaded tokio
executor requires futures to be `Send`. We cannot naively `tokio::spawn` a task holding Clojure
values.

**Decision: Use `tokio::task::LocalSet` as the async executor.** All Clojure async tasks run on a
single-threaded local executor; `spawn_local` requires no `Send`. This is correct for I/O-bound
async code. CPU-bound parallelism stays on the thread pool (existing `future` behavior). A
multi-threaded async tier can be added later with explicit GC-aware synchronization.

---

## Architecture

### Execution Model

```
Thread 1 (main / LocalSet)          Thread Pool (existing)
──────────────────────────           ──────────────────────
tokio LocalSet executor              std::thread per future
├─ ^:async fn calls                  ├─ (existing) future macro
├─ await / alt / alts                ├─ pmap (parallel CPU work)
├─ channel put!/take!                └─ agent actions (upgraded below)
├─ go blocks
└─ timeout / sleep

GC safepoints: pause the LocalSet (cooperative yield points after each poll cycle)
```

### Value Additions

```rust
// cljrs-value/src/value.rs
pub enum Value {
    // UPGRADE: CljxFuture internals changed; variant name kept
    Future(GcPtr<CljxFuture>),

    // NEW:
    Channel(GcPtr<CljxChannel>),
}
```

```rust
// cljrs-value/src/types.rs — upgrade CljxFuture
pub struct CljxFuture {
    inner: Arc<FutureShared>,
    cancel: Option<tokio::task::AbortHandle>,
}

struct FutureShared {
    // Supports multiple concurrent readers (deref from multiple threads)
    result: tokio::sync::OnceCell<Result<Value, Value>>,
    notify: tokio::sync::Notify,
    state: AtomicU8,  // Running / Done / Failed / Cancelled
}

impl CljxFuture {
    // Yields to the executor (async context only)
    pub async fn await_value(&self) -> Result<Value, Value> { ... }
    // Parks the OS thread (sync context only)
    pub fn blocking_deref(&self, timeout: Option<Duration>) -> Result<Value, Value> { ... }
}
```

```rust
// New channel type
pub struct CljxChannel {
    sender: tokio::sync::mpsc::Sender<Value>,
    // Mutex because only one task should take! at a time
    receiver: tokio::sync::Mutex<tokio::sync::mpsc::Receiver<Value>>,
    capacity: Option<usize>,  // None = rendezvous (see Phase E)
    closed: AtomicBool,
}
```

---

## Phase A — Foundation

**Crates**: `cljrs-value`, `cljrs-interp`, `cljrs-eval`, `cljrs-stdlib`, `cljrs`, root `Cargo.toml`

1. **Add the `async` feature** to workspace and all affected crates as described in the Feature Flag
   section above. Mark tokio and futures-util as `optional = true` at the workspace level.

2. **Global runtime + LocalSet** in the CLI entry point (`cljrs`), gated on `#[cfg(feature = "async")]`:
   ```rust
   static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();

   fn main() {
       let rt = RUNTIME.get_or_init(|| {
           tokio::runtime::Builder::new_current_thread()
               .enable_all()
               .build()
               .unwrap()
       });
       let local = tokio::task::LocalSet::new();
       rt.block_on(local.run_until(async_main()));
   }
   ```
   The same pattern (already sketched in `core_async.rs`) applies.

3. **Upgrade `CljxFuture`** internals conditionally. Without the feature, the existing
   `Mutex<FutureState>` + `Condvar` implementation is kept exactly as-is. With the feature, the
   internals switch to `tokio::sync::OnceCell` + `Notify`. The external Rust API (`blocking_deref`,
   `await_value`, `future-done?`, `future-cancel`) is identical in both cases; only the storage
   changes.

4. **Rename agent's `await`** to `await-agent` in `cljrs-stdlib` and `cljrs-builtins`. This frees
   the `await` name for the new async primitive.

5. **Add `Value::Channel`** variant (gated with `#[cfg(feature = "async")]`) and `CljxChannel`
   type (in a `#[cfg(feature = "async")]` module block).

---

## Phase B — Async Functions (`^:async`)

**Crates**: `cljrs-reader`, `cljrs-interp`, `cljrs-eval`

### Metadata Propagation

The reader already handles metadata maps. `^:async` desugars to `^{:async true}`. No reader
changes are needed. The interpreter checks `fn.meta().get(":async") == Some(true)`.

### Async Context Flag

```rust
// cljrs-interp (eval context / env)
pub struct EvalCtx {
    pub is_async: bool,
    // ...existing fields
}
```

### Dual Eval Path

```rust
// Sync — existing
pub fn eval(form: &Form, env: &mut Env) -> Result<Value, CljError>

// Async — for bodies of ^:async fns; can co-await other futures
pub async fn eval_async(form: &Form, env: &mut Env) -> Result<Value, CljError>
```

`eval_async` delegates to `eval` for all forms except `await`, which it handles by yielding.

### Calling an `^:async` fn

```rust
// cljrs-interp/src/apply.rs
if fn_is_async(callee) {
    let captured_env = env.clone();
    let future = tokio::task::spawn_local(async move {
        eval_async(&fn_body, &mut captured_env.with_async(true)).await
    });
    return Ok(Value::Future(CljxFuture::from_abort_handle(future)));
}
```

The call returns a `Value::Future` immediately; the body runs concurrently on the LocalSet.

### `defn` and `fn` Macros

No macro changes needed. `^:async` metadata flows through to the `Fn` value; the interpreter
checks it at call time.

```clojure
(defn ^:async fetch [url]
  (let [resp (await (http/get url))]
    (:body resp)))

;; Calling (fetch url) returns a Future immediately.
;; Caller must (await (fetch url)) or (deref (fetch url)).
```

---

## Phase C — `await` and `deref`

**Crates**: `cljrs-interp` (special forms), `cljrs-builtins`

### `await` as a Special Form

`await` must be a special form (not a function) because it syntactically marks a yield point for
the IR and must be detectable at compile time for Phase H.

```rust
// cljrs-interp/src/special_forms.rs
SpecialForm::Await => {
    if !ctx.is_async {
        return Err(runtime_err(
            "await can only be used inside an ^:async function"
        ));
    }
    let val = eval_async(&args[0], env).await?;
    match val {
        Value::Future(f)  => f.await_value().await.map_err(into_cljrs_err),
        Value::Promise(p) => p.await_value().await.map_err(into_cljrs_err),
        other => Ok(other),  // already realized; pass through
    }
}
```

`await` on a non-future is a no-op (passes the value through), consistent with JS `await`.

### `deref` / `@` for Futures (Sync Context)

```clojure
(deref future)                     ; blocks thread indefinitely
(deref future 5000 :timeout-val)   ; blocks with timeout
```

```rust
// cljrs-builtins — deref dispatch on Value::Future
Value::Future(f) => {
    if ctx.is_async {
        return Err(runtime_err(
            "use await instead of deref inside an ^:async function"
        ));
    }
    f.blocking_deref(timeout).map_err(into_cljrs_err)
}
```

`blocking_deref` parks the OS thread using `tokio::runtime::Handle::block_on` or a `Notify`
parking loop. It never blocks the LocalSet executor.

---

## Phase D — `timeout`, `alts`, and `alt`

**Crates**: `cljrs-builtins`, `cljrs-stdlib`

### `timeout`

```clojure
(timeout ms)  ; => Future that delivers nil after ms milliseconds
```

```rust
fn clj_timeout(ms: i64) -> Value {
    let fut = CljxFuture::from_async(async move {
        tokio::time::sleep(Duration::from_millis(ms as u64)).await;
        Ok(Value::Nil)
    });
    Value::Future(fut)
}
```

### `alts` — Dynamic Future Selection

```clojure
(alts [f1 f2 (timeout 5000)])
; => [value index]   ; index = which future completed first
```

`tokio::select!` requires statically-known branches. Use `futures::future::select_all` (add
`futures = "0.3"` to workspace dependencies) for dynamic dispatch:

```rust
async fn clj_alts(futures: Vec<GcPtr<CljxFuture>>) -> Value {
    let indexed: Vec<_> = futures
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let f = f.clone();
            Box::pin(async move { f.await_value().await.map(|v| (v, i)) })
        })
        .collect();
    let ((val, idx), _, _) = futures_util::future::select_all(indexed).await;
    Value::Vector(vec![val.unwrap_or(Value::Nil), Value::Long(idx as i64)])
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

`alt` is a macro that evaluates all future expressions, calls `alts`, then dispatches:

```clojure
;; Macroexpansion:
(let [__futs [f1 f2 (timeout 500)]
      __handlers [(fn [v] ...) (fn [v] ...) (fn [_] ...)]
      [__val __idx] (alts __futs)]
  ((nth __handlers __idx) __val))
```

`alt` also handles channel operations encoded as tagged tuples:

```clojure
(alt
  [ch1 :take]    (fn [v] ...)
  [ch2 :put val] (fn [_] ...))
```

The runtime converts `:take`/`:put` entries into the appropriate futures before calling `alts`.

---

## Phase E — Channel Primitives

**Crate**: `cljrs-stdlib/src/core_async.rs` (uncomment and expand)

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

### `go` Macro

```clojure
(go
  (let [v (take! ch)]
    (put! result-ch (* v 2))))
; => Future<nil or block return value>
```

`go` spawns a `spawn_local` task and returns a `Value::Future`. It is semantically equivalent to
`(future-async (fn ^:async [] ...))` but uses the traditional core.async surface syntax.

```clojure
;; Macroexpansion:
(clojure.core/async-spawn (fn ^:async [] (let [v (take! ch)] (put! result-ch (* v 2)))))
```

---

## Phase F — Upgrade Existing Concurrency Primitives

### Agent → tokio actor

Replace the per-agent OS thread + `std::sync::mpsc::SyncSender` with a `spawn_local` task and
`tokio::sync::mpsc::Sender`:

```rust
pub struct Agent {
    state: Arc<tokio::sync::RwLock<Value>>,
    error: Arc<tokio::sync::Mutex<Option<Value>>>,
    sender: tokio::sync::mpsc::Sender<AgentMsg>,
    watches: Arc<tokio::sync::Mutex<Vec<(Value, Value)>>>,
}
```

The worker loop runs as a `spawn_local` task. Agent actions that are `^:async` fns can now yield
properly without blocking the thread pool.

### Promise → `tokio::sync::oneshot`

```rust
pub struct CljxPromise {
    sender: Mutex<Option<tokio::sync::oneshot::Sender<Value>>>,
    // Cache delivered value so multiple derefs work
    cached: tokio::sync::OnceCell<Value>,
}
```

`deliver` fires the oneshot; subsequent `deref`/`await` reads from `cached`.

### `pmap` + new `async-pmap`

Existing `pmap` (thread-pool) is unchanged. Add:

```clojure
(defn ^:async async-pmap [f coll]
  (let [futs (map (fn [x] (async-spawn (fn ^:async [] (await (f x))))) coll)]
    (await (join-all futs))))
```

`join-all` is a new builtin: awaits a sequence of futures and returns a vector of results.

---

## Phase G — GC Integration

**Crates**: `cljrs-gc`, eval loop

### Safepoints at Async Yields

Every `await` point is a potential GC safepoint. With `LocalSet`, all async tasks cooperate on one
thread, so stop-the-world is "finish the current poll, then check for GC before the next poll":

```rust
// In the LocalSet driver loop (cljrs / main eval loop):
loop {
    local_set.run_until_stalled().await;
    if GC_REQUESTED.load(Ordering::Acquire) {
        gc::collect_now();  // safe: no tasks running, single thread
        GC_REQUESTED.store(false, Ordering::Release);
    }
}
```

### Root Scanning for Async Closures

Values captured in `spawn_local` closures (the compiler-generated async state machines) must be
reachable as GC roots. Options:

1. **Conservative scanning**: scan the stack of the LocalSet thread during collection
2. **Precise tracking**: register captured `GcPtr`s when a task is spawned; deregister on
   completion (more work, more correct)

Start with conservative stack scanning (consistent with the existing stop-the-world approach).

---

## Phase H — IR and Compiler Support

**Crates**: `cljrs-ir`, `cljrs-eval`

### IR Additions

```rust
// cljrs-ir/src/lib.rs
pub enum IrInstr {
    // ...existing
    Await  { src: Reg, dst: Reg },       // yield point; used in async IrFunctions
    Spawn  { fn_reg: Reg, args: Vec<Reg>, dst: Reg },  // spawn_local
    ChanTake { chan: Reg, dst: Reg },
    ChanPut  { chan: Reg, val: Reg },
}

pub struct IrFunction {
    pub is_async: bool,
    // ...existing
}
```

### IR Interpreter Strategy

- **Short-term**: async IR functions fall back to tree-walking (`eval_async`) rather than the IR
  interpreter. This avoids building a full async state machine in IR.
- **Long-term (Phase 10 JIT)**: async functions compile to Cranelift state machines with explicit
  resume points at `Await` instructions, matching Rust's generated async code structure.

### Static Async Checking (Phase H+)

Once IR lowering tracks `is_async`, the compiler can:
- Error at compile time if `await` appears outside an `^:async` function
- Warn if an `^:async` function never actually awaits anything
- Infer `^:async` on functions that call other `^:async` functions (opt-in, off by default)

---

## Additional `clojure.core.async` Functions to Implement

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

1. **`await` is a special form, not a function.** It must be syntactically detectable to emit IR
   yield points and for static async checking in Phase H.

2. **`LocalSet` throughout async.** No `Value` crosses thread boundaries in async code.
   `spawn_blocking` (for CPU-bound work) communicates results back via channels or `Future`.

3. **`deref` is always blocking.** It parks the OS thread, never the async executor. Using `deref`
   on a future inside `^:async` is a runtime error (eventually a compile-time error in Phase H).

4. **`^:async` is viral.** A function calling `await` must itself be `^:async`. The interpreter
   enforces this at call time; the compiler enforces it statically in Phase H.

5. **Existing `future` macro is unchanged.** It continues to spawn OS threads. Both thread-futures
   and async-futures are unified at the `CljxFuture` level and are deref-able / await-able.

6. **GC stop-the-world is safe with `LocalSet`.** Single-threaded cooperative scheduling means
   "stop the world" = "finish current poll, run GC, resume." No locks or safepoint handshake
   protocol required for the async tier.

---

## Implementation Phases Summary

| Phase | Deliverable | Key Files |
|---|---|---|
| A | `async` feature flag; optional tokio dep; upgrade `CljxFuture` (feature-conditional); rename `await-agent` | `Cargo.toml` (workspace + crates), `cljrs-value/types.rs`, `cljrs/main.rs`, `cljrs-builtins` |
| B | `^:async` fn dispatch; dual eval path; `await` special form | `cljrs-interp/special_forms.rs`, `cljrs-interp/apply.rs` |
| C | `deref` blocking await; async/sync context enforcement | `cljrs-builtins/deref.rs` |
| D | `timeout`, `alts`, `alt` macro | `cljrs-stdlib/core_async.rs`, `cljrs-interp/macros.rs` |
| E | `chan`, `put!`/`take!`, `close!`, `go` macro | `cljrs-stdlib/core_async.rs`, `cljrs-value/types.rs` |
| F | Agent → tokio actor; promise → oneshot; `async-pmap` | `cljrs-value/types.rs`, `cljrs-builtins` |
| G | GC safepoints at await yields; async root scanning | `cljrs-gc`, eval loop |
| H | IR `Await`/`Spawn` instructions; static async checking | `cljrs-ir`, `cljrs-eval/ir_interp.rs` |
