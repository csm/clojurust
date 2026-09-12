# Extensions & native code

A freshly built runtime knows `clojure.core` and nothing else. Everything
beyond it — the standard library, `core.async`, file I/O, sockets, codecs, your
own Rust functions — installs itself into the finished runtime. Nothing is
implicit, so a host that only wants pure data transformation never links a
socket API into its binary.

```rust
let runtime = Runtime::builder().build()?;

cljrs_compiler::jit::install(&runtime);   // Tier 2 (before any guest code)
cljrs_stdlib::install(&runtime);          // clojure.string, clojure.set, …
```

Every `install`/`init` in this chapter is idempotent, so calling one twice is
harmless.

## The standard library

`cljrs_stdlib::install(&runtime)` registers these namespaces:

| Namespace | Notes |
|---|---|
| `clojure.string` | native helpers plus Clojure source |
| `clojure.set` | native helpers plus Clojure source |
| `clojure.walk`, `clojure.data`, `clojure.zip`, `clojure.template` | pure Clojure |
| `clojure.test` | pure Clojure |
| `clojure.spec.alpha`, `clojure.spec.gen.alpha`, `clojure.spec.test.alpha` | generators are throwing stubs |
| `clojure.edn` | not available on `wasm32` (uses `std::fs`) |
| `clojure.rust.io` | synchronous file I/O; not available on `wasm32` |

They are registered as **embedded sources**, so each is parsed and evaluated on
its first `require` rather than at install time. Installing the whole stdlib
therefore costs almost nothing until guest code asks for a namespace.

`cljrs_stdlib::register(&globals)` is the same call for a host holding the
`Arc<GlobalEnv>`.

Note that this is all-or-nothing: there is no "stdlib without `clojure.rust.io`"
switch. If your concern is what guest code can reach, see [Limits &
sandboxing](sandboxing.md) — the answer is not to skip the stdlib, because
`clojure.core` itself has `slurp` and `spit`.

## Async, I/O, and networking

These extensions need a Tokio `LocalSet` — they spawn tasks with `spawn_local`,
and `cljrs_async::init` starts a background GC-service task immediately. Two
consequences for a host:

1. Call `init` **from inside** the `LocalSet`, not before it.
2. Drive each top-level form on that same `LocalSet`, or spawned tasks (channel
   producers, `^:async` bodies, I/O readers) never make progress.

```rust
let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
let local = tokio::task::LocalSet::new();

let globals = runtime.globals();
local.block_on(&rt, async {
    cljrs_async::init(globals);     // clojure.core.async, clojure.rust.error
    cljrs_io::init(globals);        // clojure.rust.io.async
    cljrs_net::init(globals);       // clojure.rust.net.* (calls cljrs_async::init itself)
    cljrs_charset::init(globals);   // clojure.rust.charset
});
cljrs_base64::init(globals);        // cljrs.base64 — no async requirement
```

| Crate | Namespaces | Requires |
|---|---|---|
| `cljrs-async` | `clojure.core.async`, `clojure.rust.error` | a `LocalSet` |
| `cljrs-io` | `clojure.rust.io.async` | `cljrs_async::init` + a `LocalSet` |
| `cljrs-net` | `clojure.rust.net`, `.tcp`, `.udp`, `.tls`, `.unix`, `.frame`, `.quic`, `.h3`, `.http2` | a `LocalSet` (initialises async itself) |
| `cljrs-charset` | `clojure.rust.charset`; `init_async` adds `clojure.rust.charset.async` | `init` is synchronous; `init_async` needs async + a `LocalSet` |
| `cljrs-base64` | `cljrs.base64` | nothing |

The CLI's own evaluation loop is the reference implementation: it builds the
runtime and the `LocalSet` once, then drives every top-level form with
`LocalSet::block_on`, so tasks that outlive one form stay queued and continue on
the next form's drive. Deliberately, it does **not** wrap the whole session in a
single outer `block_on` — that would panic when a per-form `block_on` nests
inside it.

Without a driver installed, evaluation still works; it is simply synchronous,
and anything that depends on a spawned task will not complete.

## Exposing your own Rust functions

The `Registry` is the same object described in [Rust
interop](../rust-interop/registry.md), but an embedding host constructs it
directly rather than exporting a `cljrs_init_<crate>` symbol for a dylib to be loaded
through:

```rust
use cljrs_interop::{Registry, wrap_fn1, wrap_fn2};

let registry = Registry::new(runtime.globals().clone());

registry.define(
    "acme.native/add",
    wrap_fn2("add", |a: i64, b: i64| Ok::<i64, String>(a + b)),
);
registry.define(
    "acme.native/config-value",
    wrap_fn1("config-value", move |key: String| {
        host_config.get(&key).cloned().ok_or_else(|| format!("no such key: {key}"))
    }),
);
```

Guest code then calls `(acme.native/add 3 4)`. Constructing a `Registry` also
registers every `#[export]`-annotated function in the binary via `inventory`, so
the [`#[export]` macro](../rust-interop/export-macro.md) works in a host program
exactly as it does in a dylib.

The wrappers marshal arguments and return values automatically for the types
implementing `FromValue`/`IntoValue` — `bool`, `i64`, `f64`, `String`, `BigInt`,
`Option<T>`, `Vec<Value>`, and `Value` itself. Anything else crosses as an
opaque `NativeObject`. A closure passed to `wrap_fn*` may capture host state, as
above; that is the usual way to give guest code a keyhole onto the host rather
than a general capability.

A native function that needs to call *back* into Clojure — a comparator, a
callback, a hook — uses `cljrs_runtime::tiered::invoke`, which is covered in
[Evaluating code](evaluating.md#calling-clojure-from-rust).

## Making namespaces resolvable

Guest code reaches a namespace through `require`, which resolves in this order:

1. **Already loaded** — anything an extension registered, or a previous
   `require` evaluated.
2. **A registered compiled-namespace loader** — how an AOT-produced binary
   supplies its own namespaces.
3. **Embedded sources** — namespaces registered with
   [`builtin_source`](runtime-builder.md#builtin_source), evaluated on first use.
4. **Source paths** — the first `<path>/<rel>.cljrs` or `<path>/<rel>.cljc` that
   exists, where `<rel>` is the namespace with dots turned into slashes and
   dashes into underscores, searched across the configured
   [source paths](runtime-builder.md#source_paths) in order.
5. **A native-require hook**, if the host installed one (the CLI does, for
   `:rust/load :dylib` dependencies).

A host that ships its Clojure inside the binary uses (3) and can leave the
source paths empty; a host that wants operators to drop `.cljc` files into a
plugins directory uses (4). A namespace found nowhere raises
`Could not find namespace <ns> on source path`.

## Serving an editor

An embedded runtime can expose the same tooling surface the CLI does. The nREPL
server takes the `Arc<GlobalEnv>` and runs its network side on its own thread,
handing jobs back to your interpreter thread as plain `Send` data:

```rust
let server = cljrs_nrepl::start(cljrs_nrepl::Config::default(), globals.clone())?;
println!("nREPL listening on port {}", server.port());
server.serve()?;   // blocks this thread, processing evals
```

That gives editors (CIDER, Calva, Conjure) a live connection into your running
application — the same runtime, the same namespaces, your native functions
included. `cljrs-lsp` is a separate, purely syntactic server (parse diagnostics
and a document-symbol outline); it never invokes the evaluator, so it does not
need your runtime at all.
