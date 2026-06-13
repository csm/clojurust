# cljrs-nrepl

**Purpose** — nREPL server for clojurust: speaks the [nREPL protocol](https://nrepl.org) (bencode-encoded messages over TCP) so editors like CIDER, Calva, and Conjure can connect to a running interpreter.

**Status** — Phase 12 (REPL & Tooling). Implemented; exposed as the `cljrs nrepl` subcommand and usable as a library.

## Design

`GcPtr` (and therefore `Value`, `Env`, `GlobalEnv`) is not `Send`, so all interpreter state stays on the thread that created the `GlobalEnv`:

- The **network thread** (spawned by `start`) runs a current-thread tokio runtime with the TCP listener. Per-connection tasks decode bencode frames and answer `describe` and `interrupt` directly.
- Every other op becomes a `Job` — plain `Send` data (strings, a reply channel, a cancellation flag) — sent over an mpsc channel to the **interpreter thread**, which processes jobs in `Server::serve` / `Server::serve_with`.

Supported ops: `clone`, `close`, `describe`, `eval`, `interrupt`, `load-file`, `lookup`, `ls-sessions`, `completions`.

Each session has its own `Env` (namespace) and its own `*1`/`*2`/`*3`/`*e`, bound via the dynamic-binding stack around each request. Retained values are interned in the hidden `cljrs.nrepl.session-state` namespace so the GC traces them between evals.

### Limitations

- **Interrupt is best-effort.** A queued request is dropped, and a multi-form request stops between forms, but a single form that loops forever cannot be cancelled (the interpreter has no preemption hook).
- **Output is batched per form**, not streamed incrementally: `out` is delivered when the printing form finishes (it reuses the interpreter's thread-local output-capture stack from `cljrs-builtins`).
- Requests without a session share a single `"default"` session rather than receiving a transient one.

## File layout

| File | Purpose |
|---|---|
| `src/lib.rs` | Public API: `Config`, `start`, `Server`, `ShutdownHandle`, the `Job` bridge type |
| `src/bencode.rs` | Hand-rolled bencode codec (the nREPL subset) with incremental decoding for TCP framing |
| `src/protocol.rs` | `Request` decoding and the `Response` builder |
| `src/server.rs` | Network thread: accept loop, per-connection reader/writer tasks, `describe`/`interrupt`, in-flight registry |
| `src/engine.rs` | Interpreter thread: session registry, eval with output capture and `*1`/`*2`/`*3`/`*e`, completions, lookup |
| `tests/nrepl_server.rs` | End-to-end test: full stdlib env + scripted bencode client over TCP |

## Public API

- `struct Config { addr: SocketAddr, port_file: Option<PathBuf> }` — bind address (port 0 = OS-assigned) and optional `.nrepl-port` file; `Default` binds `127.0.0.1:0`.
- `fn start(config: Config, globals: Arc<GlobalEnv>) -> miette::Result<Server>` — binds the listener, writes the port file, spawns the network thread. Must be called on the thread that owns `globals`.
- `Server::port(&self) -> u16`
- `Server::serve(self) -> miette::Result<()>` — blocks the calling (interpreter) thread processing requests until shutdown; evaluates with `cljrs_eval::eval`.
- `Server::serve_with(self, eval_form: impl EvalForm) -> miette::Result<()>` — like `serve`, but each top-level form goes through the supplied evaluator (the CLI passes its async `LocalSet` driver).
- `Server::shutdown_handle(&self) -> ShutdownHandle` — `Send + Clone`; `shutdown()` stops the network thread and ends `serve`.
- `trait EvalForm: FnMut(&Form, &mut Env) -> Result<Value, EvalError>` — evaluator signature for `serve_with`.
- `mod bencode` — `Bencode`, `encode`, `encode_to_vec`, `decode` (public for tests/clients).
- `mod protocol` — `Request`, `Response`.
