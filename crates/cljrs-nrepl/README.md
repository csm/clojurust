# cljrs-nrepl

**Purpose** — nREPL server for clojurust: speaks the [nREPL protocol](https://nrepl.org) (bencode-encoded messages over TCP) so editors like CIDER, Calva, and Conjure can connect to a running interpreter.

**Status** — Phase 12 (REPL & Tooling). Implemented; exposed as the `cljrs nrepl` subcommand and usable as a library.

## Design

`GcPtr` (and therefore `Value`, `Env`, `GlobalEnv`) is not `Send`, so all interpreter state stays on the thread that created the `GlobalEnv`:

- The **network thread** (spawned by `start`) runs a current-thread tokio runtime with the TCP listener. Per-connection tasks decode bencode frames and answer `describe` and `interrupt` directly.
- Every other op becomes a `Job` — plain `Send` data (strings, a reply channel, a cancellation flag) — sent over an mpsc channel to the **interpreter thread**, which processes jobs in `Server::serve` / `Server::serve_with`.

Supported ops: `clone`, `close`, `describe`, `eval`, `interrupt`, `load-file`, `lookup`, `ls-sessions`, `completions`, plus these cider-nrepl ops:

- `macroexpand` — keys `code`, `ns`, `expander` (`macroexpand-1`, `macroexpand` (default) or `macroexpand-all`) and `display-namespaces` (`qualified` (default), `none` or `tidy`). `none` drops every namespace. `tidy` drops a namespace only when the bare name resolves in `ns` to the same var (defined there or referred), so the expansion still evaluates as shown; any other symbol is shortened to an alias `ns` has for its namespace, or left as written. Replies `{"expansion": <printed form>}` then `done`; a read or expansion failure replies `err` with status `["macroexpand-error" "done"]`. Uses the runtime's own `cljrs_runtime::interp::macros` expanders.
- `analyze-last-stacktrace` (also answered under the legacy name `stacktrace`) — one message per cause of the session's `*e`, outermost first, with `class`, `message`, `data` (the printed `ex-data`, when present) and `stacktrace` (a list of frames); then `done`. With no `*e` the reply is `["no-error" "done"]`.

`eval` and `load-file` stream `out` messages while a form runs: printed text is sent at each newline, whenever 1 KiB is pending without one, and when the form ends. This holds for `serve`, `serve_with` and `Poller` alike, since all three run the same engine.

Each session has its own `Env` (namespace) and its own `*1`/`*2`/`*3`/`*e`, bound via the dynamic-binding stack around each request. Retained values are interned in the hidden `cljrs.nrepl.session-state` namespace so the GC traces them between evals.

### Limitations

- **Interrupt is cooperative.** While an `eval`/`load-file` runs, its cancel flag is installed on the runtime's execution-credit (gas) meter (`cljrs_runtime::env::gas::InterruptGuard`). An `interrupt` sets the flag from the network thread, and the running code stops at its next checkpoint: every tree-walker eval step, every IR basic block, and the JIT's credit checks. So a `(loop [] (recur))`, a non-terminating function, or a deep non-tail recursion is stopped mid-form, a `try`/`catch` cannot swallow it, and the session keeps working afterwards. This applies equally to `serve`, `serve_with` and the `Poller` (the eval runs inside `poll`, and the network thread interrupts it there). The eval answers `status ["interrupted"]` and then `["done"]`. The `interrupt` request answers `["done"]`, `["session-idle","done"]` when nothing is in flight in that session, or `["interrupt-id-mismatch","done"]` when `interrupt-id` names no in-flight request. A queued request that is interrupted is dropped without being evaluated.
  **Not interruptible:** time spent inside a single native call that does not return to evaluated code: blocking I/O (socket/file/stdin reads), sleeping, waiting on a lock, promise or future, and long-running Rust builtins (for example sorting or hashing a huge collection in one call). The interrupt takes effect when the call returns and evaluation reaches its next checkpoint. The flag is thread-local to the evaluating thread, so work the eval hands to other threads (`future`, agents, or tasks a `serve_with` evaluator runs on a different thread) keeps running; only the eval itself is stopped. Async tasks on the eval's own thread are scoped the same way: a task stops only when the eval that spawned it is interrupted, not when it happens to be polled while a different, interrupted eval drives the executor. Work already done is not rolled back: side effects and `def`s made before the interrupt remain.
- **No `err` stream for user output.** cljrs has no stderr writer for user code (`*err*` is unbound and every print goes to `*out*`), so only `out` is streamed; `err` carries evaluation errors only. Text printed inside `with-out-str` is captured by it and not streamed.
- **`macroexpand`:** `print-meta` is ignored.
- **`analyze-last-stacktrace` frames are always empty.** cljrs records no stack frames on its exceptions, so `stacktrace` is an empty list rather than invented JVM frames. `class` is `clojure.lang.ExceptionInfo` for an exception carrying `ex-data` (CIDER keys its data view off that name) and otherwise cljrs's own error kind (`WrongType`, `ArityError`, `Other`, …); a thrown non-exception value reports its type name.
- Requests without a session share a single `"default"` session rather than receiving a transient one.

## File layout

| File | Purpose |
|---|---|
| `src/lib.rs` | Public API: `Config`, `start`, `Server`, `ShutdownHandle`, the `Job` bridge type |
| `src/bencode.rs` | Hand-rolled bencode codec (the nREPL subset) with incremental decoding for TCP framing |
| `src/protocol.rs` | `Request` decoding and the `Response` builder |
| `src/server.rs` | Network thread: accept loop, per-connection reader/writer tasks, `describe`/`interrupt`, in-flight registry |
| `src/engine.rs` | Interpreter thread: session registry, eval with streamed output and `*1`/`*2`/`*3`/`*e`, completions, lookup, macroexpand, analyze-last-stacktrace |
| `tests/nrepl_server.rs` | End-to-end test: full stdlib env + scripted bencode client over TCP |

## Public API

- `struct Config { addr: SocketAddr, port_file: Option<PathBuf> }` — bind address (port 0 = OS-assigned) and optional `.nrepl-port` file; `Default` binds `127.0.0.1:0`.
- `fn start(config: Config, globals: Arc<GlobalEnv>) -> miette::Result<Server>` — binds the listener, writes the port file, spawns the network thread. Must be called on the thread that owns `globals`.
- `Server::port(&self) -> u16`
- `Server::serve(self) -> miette::Result<()>` — blocks the calling (interpreter) thread processing requests until shutdown; evaluates with `cljrs_runtime::tiered::eval`.
- `Server::serve_with(self, eval_form: impl EvalForm) -> miette::Result<()>` — like `serve`, but each top-level form goes through the supplied evaluator (the CLI passes its async `LocalSet` driver).
- `Server::shutdown_handle(&self) -> ShutdownHandle` — `Send + Clone`; `shutdown()` stops the network thread and ends `serve`.
- `trait EvalForm: FnMut(&Form, &mut Env) -> Result<Value, EvalError>` — evaluator signature for `serve_with`.
- `mod bencode` — `Bencode`, `encode`, `encode_to_vec`, `decode` (public for tests/clients).
- `mod protocol` — `Request`, `Response`.

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | **on** | Forwards `regex-full` to this crate's workspace dependencies — `Value::Pattern` uses the `regex` crate. |
| `small-regex` | off | Forwards `small-regex` instead: `regex-lite`, which trades Unicode character classes for ~277 KB of text. |
| `deps` | **on** | Pass-through for `cljrs-runtime/deps` — git-backed dependency and versioned-var support. |

Every workspace dependency of this crate is taken with default features off (see
the note in the root `Cargo.toml`), so these pass-throughs are what put back what
those crates' defaults used to provide. `default` enables all of them, so a plain
build is unchanged.

`regex-full` wins when both regex features are enabled, so selecting the small
engine means turning default features off **on this crate** and re-adding what
you want:

```toml
cljrs-nrepl = { version = "0.1", default-features = false, features = ["small-regex"] }
```

Adding a second, direct dependency on `cljrs-runtime` with
`default-features = false` would not undo it — Cargo unions features across every
edge to a package, so one edge left at its defaults re-enables `regex-full` for
the whole graph. `deps` has to be off as well for the size win to land, since
`cljrs-project/vcs` pulls `regex` in through `pgp`. See
[cljrs-value's README](../cljrs-value/README.md#features).


The `no-gc` feature forwards GC-less allocation to runtime, value, and allocator dependencies.
Each request owns a temporary read scope. Sessions retain independent leases for `*1`, `*2`, `*3`, and `*e`.
Closing a session releases these leases and clears its internal retained-value roots.
