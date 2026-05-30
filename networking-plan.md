# Networking Plan for clojurust

## Design Philosophy

Networking is delivered as **`cljrs-net`**, a separate Rust crate that sits *on top of*
`cljrs-async` (channels, the Tokio `LocalSet` executor, `spawn_future`) and mirrors the
conventions established by `cljrs-io`. The model is **aleph/netty, expressed in core.async**:

- The **bottom layer is byte-stream oriented**. A connection is a duplex pair of
  `clojure.core.async` channels — `:in` carries `byte-array` chunks read off the socket,
  `:out` accepts `byte-array`/string values to write. This is the same shape as
  `cljrs-io`'s `chunk-chan`, so network bytes compose with `go`, `<!`, `alts!`, transducers,
  and the rest of the async toolkit.
- **Higher-level protocols are higher-order functions over those channels** — stateful
  transducers for pure byte→message reframing, and `(fn [in] -> out)` pipe functions for
  anything that needs async. Framing a TCP byte stream into application messages is the same
  operation whether the bytes came from a file or a socket.
- **Transports are interchangeable** behind that channel shape: Unix sockets, TCP, and TLS
  (via `rustls`/`tokio-rustls`) all yield the identical duplex `{:in :out}` connection. UDP
  is the deliberate exception (datagrams, not a stream — see Phase D).

Like `cljrs-async` and `cljrs-io`, the core crates know nothing about sockets. The CLI binary
links `cljrs-net` by default (same as `cljrs-io`); embedders opt in via `Cargo.toml`.

This document mirrors `async-plan.md`. Read that first — every mechanism here (the single-thread
`!Send` executor, `spawn_future`, the in-band `Value::Error` convention, channel backpressure)
is inherited from it.

---

## Crate Layout

```
cljrs-async        ← channels (CljChannel), LocalSet executor, spawn_future, await_value
cljrs-io           ← channel-oriented async file I/O (the pattern cljrs-net follows)
cljrs-net          ← NEW: TCP / Unix / TLS / UDP transports + framing, over core.async channels
cljrs (CLI)        ← feature "net" (default = on, like "async") links cljrs-net and calls init()
```

`cljrs-net` depends on: `cljrs-async` (channels + `spawn_future`), the value/env/interp core
crates (as `cljrs-io` does), `tokio` (`net`, `io-util`, `rt`), `tokio-rustls` + `rustls` +
`rustls-pemfile` + `webpki-roots` (TLS), and `tokio` Unix support behind `#[cfg(unix)]`.

### Why a separate crate

Identical reasoning to `async-plan.md`: no `#[cfg(feature)]` guards or `tokio`/`rustls`
dependencies leak into the core. `clojure.core.async` ships as a separate JAR; aleph ships
separately from Clojure. `cljrs-net` is the same — the runtime gains sockets only when the
crate is linked and `cljrs_net::init` is called.

---

## Namespaces

| Namespace | Contents |
|---|---|
| `clojure.rust.net` | umbrella: `connect`, `listen`, `start-server`, `close`, `with-open`-style helper |
| `clojure.rust.net.tcp` | TCP client/server (also the default transport of the umbrella fns) |
| `clojure.rust.net.unix` | Unix-domain stream sockets (`#[cfg(unix)]`) |
| `clojure.rust.net.tls` | TLS over TCP via `rustls`; client config (SNI, roots) and server config (cert/key) |
| `clojure.rust.net.udp` | UDP datagram sockets (datagram channel shape, not a byte stream) |
| `clojure.rust.net.frame` | framing transducers + pipe-fns: `lines`, `length-prefixed`, `by-delimiter` |

The umbrella `connect`/`listen` take a `:transport` key (`:tcp` default, `:unix`, `:tls`) so
callers can stay transport-agnostic; the per-transport namespaces are the explicit forms.

---

## The Connection Model

A connection is a **plain map of two simplex channels** (core.async has no duplex channel
primitive, so a pair is the idiomatic representation):

```clojure
{:in          <chan>        ; byte-array chunks read from the peer; closed at EOF/peer-close
 :out         <chan>        ; put byte-arrays/strings here to send; close to half-close write
 :remote-addr "1.2.3.4:443"
 :local-addr  "0.0.0.0:54321"
 :resource    <handle>}     ; the underlying socket Resource (Arc-backed, deterministic close)
```

- **`:in`** is a *raw* streaming channel (exactly `cljrs-io`'s `chunk-chan` contract): a producer
  task reads the socket and `put!`s `byte-array` chunks; the channel's bounded buffer (`:in-buf`,
  default 8) bounds read-ahead, so a slow consumer applies TCP-window backpressure to the peer.
  The channel **closes at EOF / peer half-close**.
- **`:out`** is a channel the user `put!`s onto; a writer task drains it to the socket. **Closing
  `:out` triggers a write-half shutdown** (TCP FIN) after the buffer drains. Its bounded buffer
  (`:out-buf`) means `put!` parks when the socket can't keep up — backpressure in the send
  direction.
- **`:resource`** holds the socket as an `Arc<dyn Resource>` (see `cljrs-value/src/resource.rs` —
  its doc comment already names sockets and uses `"TcpStream"` as the example `type_tag`).
  `(close conn)` closes both channels and the FD **deterministically**. GC never finalizes the
  socket; this is the whole reason sockets are `Resource` (Arc) and not `NativeObject` (GcPtr).

### Errors, in band

Following the `cljrs-io` convention exactly: a read/write failure is delivered as a
`Value::Error` `put!` onto `:in` (then the channel closes). Consumers use the existing
`error?`/`ok?` helpers. See "Key Design Constraints" for the error-propagation caveat this
inherits from choosing core.async over a Manifold-style stream.

---

## Execution Model

Identical to `cljrs-async`/`cljrs-io`: everything runs on the single-thread Tokio `current_thread`
+ `LocalSet` executor. Per connection, `cljrs-net` spawns (via `spawn_future`) two tasks:

1. a **reader task** — `socket.read()` into a plain `Vec<u8>` (which is `Send`), convert to a
   `byte-array` `Value` **on the executor thread**, `put!` to `:in`, repeat until EOF/error.
2. a **writer task** — `<!` from `:out`, write the bytes to the socket, repeat until `:out`
   closes, then shut down the write half.

The `Vec<u8> → byte-array` conversion happens on the GC thread because byte-arrays are `!Send`
GC values and cannot be constructed off-thread. Keeping socket I/O on the LocalSet is the
simplest correct design and matches `cljrs-io`. The fallback if TLS crypto throughput on the
interp thread ever bites (read on a worker pool, ship `Vec<u8>` across, build the byte-array on
the GC thread) is recorded under "Future Work" — not built now.

---

## Phase A — Crate Foundation & TCP Client

`cljrs-net` crate, `init(globals)` entry point (idempotent, requires `cljrs_async::init` and a
running `LocalSet`), CLI wiring under a default-on `net` feature.

- `TcpStreamResource` implementing `Resource` (close = shutdown + drop the socket halves).
- `connect` / `clojure.rust.net.tcp/connect`:
  ```clojure
  (connect {:host "example.com" :port 80})        ; => promise chan yielding a connection map
  ```
  Returns a **capacity-1 promise channel** (the `cljrs-io` discrete-op shape) that yields the
  connection map once connected, or a `Value::Error` on failure. The connection map's `:in`/`:out`
  are live streaming channels.
- Reader + writer tasks per the Execution Model.
- `close` closes `:out`, drains, shuts the socket, closes `:in`.

**Done when:** a client can connect, `(>! out req-bytes)`, `(close! out)`, and drain `:in` to EOF.

---

## Phase B — TCP Server (channel of connections)

The server primitive is a **channel of connections** (most core.async-native; the callback form
is sugar on top):

```clojure
(listen {:port 8080})            ; => {:conns <chan> :resource <listener-handle>}
;; (<! conns) yields a connection map per accepted socket; closes when the listener closes.

(start-server (fn [conn] ...) {:port 8080})   ; sugar: go-loop taking from :conns, calls handler
```

- `TcpListenerResource` (`Resource`); an accept-loop task `put!`s connection maps onto `:conns`.
- Bounded `:conns` buffer ⇒ accept backpressure (stop accepting when the app is behind).
- `(close server)` stops accepting and closes the listener FD; in-flight connections are
  independent and outlive it unless separately closed.

**Done when:** an echo server built from `listen` + `go` round-trips bytes from a `connect` client.

---

## Phase C — Framing: protocols as higher-order fns

This is the "higher level protocols using higher order functions to transform byte streams" layer
— `clojure.rust.net.frame`. Two composable idioms:

### Stateful transducers (pure byte → message)

core.async `(chan buf xform)` accepts a transducer. A framing transducer buffers partial bytes
across chunk boundaries (the `partition-by` pattern — leftover state in the reducing fn) and emits
complete frames:

```clojure
(lines)                         ; byte-array chunks -> strings (split on \n, decode utf-8)
(by-delimiter (byte 0))         ; chunks -> byte-array frames split on a delimiter byte
(length-prefixed {:bytes 4 :endian :big})  ; chunks -> length-delimited byte-array frames
```

Apply by attaching to a derived channel, or via a `frame` helper that pipes `:in` through the
xform into a new channel:

```clojure
(let [msgs (frame (:in conn) (length-prefixed {:bytes 4}))]
  (go-loop [] (when-let [m (<! msgs)] (handle m) (recur))))
```

The encode direction is an ordinary `map` xform on the write side (frame → length-prefixed
byte-array), attached to a channel that pipes into `:out`.

### Pipe-fns (async transforms)

For protocols needing async (request/response correlation, lookups, coupled backpressure):
`(fn [in-chan] -> out-chan)` that spawns a `go-loop`. More general than transducers, less
composable — used when a pure transducer can't express the transform.

**Done when:** a line-protocol and a length-prefixed-frame protocol both work end-to-end over a
TCP connection, built purely from `frame` + transducers.

---

## Phase D — UDP (datagrams)

UDP does **not** fit the byte-stream duplex; it is message-oriented, so it gets its own shape:

```clojure
(udp/socket {:port 9000})       ; => {:in <chan> :out <chan> :resource h}
;; :in  yields  {:data <byte-array> :addr "ip:port"}
;; :out accepts {:data <byte-array> :addr "ip:port"}
```

- `UdpSocketResource` (`Resource`); reader task `recv_from` → `{:data :addr}` maps; writer task
  `send_to` per outgoing map.
- No connection/accept concept; one socket multiplexes peers by address.

**Done when:** a UDP echo responder round-trips datagrams with correct `:addr`.

---

## Phase E — TLS (rustls)

TLS wraps a TCP stream and produces the **identical** `{:in :out}` connection shape, so all of
Phases A–C compose over it unchanged. Built on `tokio-rustls`.

### Client

```clojure
(tls/connect {:host "example.com" :port 443})   ; SNI = :host; roots from webpki-roots by default
(tls/connect {:host ... :port ...
              :roots :system|:webpki|<pem-path>  ; trust anchors
              :alpn ["h2" "http/1.1"]
              :insecure-skip-verify false})      ; explicit opt-out, off by default
```

- SNI derived from `:host`; a custom `ServerName` allowed via `:sni`.
- `rustls::ClientConfig` built once and cached; per-connection `TlsConnector`.

### Server

```clojure
(tls/listen {:port 8443 :cert "server.pem" :key "server-key.pem" :alpn [...]})
```

- `rustls::ServerConfig` from a cert chain + private key (`rustls-pemfile`), optional client-auth.
- Yields the same `{:conns <chan> ...}` shape as the plaintext server; each accepted connection is
  TLS-wrapped before its map is put on `:conns`.

`TlsStreamResource` wraps `tokio_rustls::{client,server}::TlsStream<TcpStream>`. The handshake runs
on the LocalSet as part of the connect/accept task; failures surface as a `Value::Error` on the
promise/conns channel.

**Done when:** a `tls/connect` to a public HTTPS host completes a handshake and exchanges bytes,
and a local `tls/listen` server round-trips with a `tls/connect` client using a test cert.

---

## Phase F — Unix-domain sockets

`#[cfg(unix)]`. `tokio::net::{UnixStream, UnixListener}`. Same duplex `{:in :out}` connection and
same channel-of-connections server as TCP — only the address is a filesystem path:

```clojure
(unix/connect {:path "/tmp/app.sock"})
(unix/listen  {:path "/tmp/app.sock"})
```

`UnixListenerResource::close` unlinks the socket path. On non-unix targets the namespace is absent
(or its fns throw a clear "unsupported on this platform" error).

**Done when:** a Unix-socket echo server round-trips with a Unix-socket client.

---

## Phase G — Lifecycle, timeouts, ergonomics

- **Deterministic teardown.** A `with-open`-style binding macro (or reuse an existing one) so a
  dropped connection map cannot leak an FD — GC will not finalize the `Resource`. Document that
  connections must be `close`d.
- **Connect / read / write timeouts** via the existing `clojure.core.async/timeout` + `alts!`
  (`connect` already returns a channel — race it against `timeout`). Provide `:connect-timeout-ms`
  sugar.
- **Half-close** semantics documented: closing `:out` sends FIN but `:in` keeps draining until the
  peer closes; full `close` tears down both.
- **Error/EOF helper.** Because we chose core.async + in-band `Value::Error` (not a Manifold-style
  stream), add `(drain-to in)` / a split helper that separates the value stream from a single
  terminal error/EOF promise, so consumers don't have to `error?`-check every take.

---

## Key Design Constraints

- **All socket I/O on the single-thread `!Send` LocalSet.** Byte-arrays are `!Send` GC values;
  they are constructed on the executor thread from `Send` `Vec<u8>` buffers. Inherited from
  `cljrs-async`/`cljrs-io`.
- **Sockets are `Resource` (Arc), never `GcPtr`.** Deterministic close; no FD leaks. The GC has no
  finalizers. `resource.rs` already anticipates this.
- **Backpressure is channel-native.** Bounded `:in`/`:out`/`:conns` buffers translate directly to
  TCP window / accept backpressure. No separate flow-control mechanism.
- **Errors are in band** (`Value::Error` + close), matching `cljrs-io`. **Caveat — the aleph
  lesson:** aleph deliberately built Manifold instead of using core.async because core.async
  channels don't propagate errors and have no "closed-because-of-exception" state, and
  `mult`/`pipeline` are awkward across a network backpressure boundary. clojurust takes the
  opposite bet for consistency with `cljrs-io`; the cost is that "drained OK" vs "drained due to
  error" is only visible by inspecting the last value, which Phase G's split helper mitigates.
- **UDP is not a byte stream.** It keeps a distinct `{:data :addr}` datagram shape rather than
  being forced into the duplex byte-stream abstraction.
- **Transports share one connection shape.** TCP, Unix, and TLS all yield `{:in :out …}`, so
  framing (Phase C) and any protocol built on it work over every transport unchanged.

---

## Implementation Phases Summary

| Phase | Deliverable | Depends on |
|---|---|---|
| A | `cljrs-net` crate, `TcpStreamResource`, `connect`, reader/writer tasks, CLI wiring | `cljrs-async`, `cljrs-io` patterns |
| B | TCP server: `listen` (channel of conns) + `start-server` sugar | A |
| C | Framing: `clojure.rust.net.frame` transducers (`lines`, `length-prefixed`, `by-delimiter`) + pipe-fns | A, B |
| D | UDP datagram sockets (`{:data :addr}` shape) | A |
| E | TLS client + server via `rustls`/`tokio-rustls` (same `{:in :out}` shape) | A, B |
| F | Unix-domain stream sockets (`#[cfg(unix)]`) | A, B |
| G | Lifecycle: `with-open`, timeouts, half-close, error/EOF split helper | A–F |

---

## Future Work (not in scope now)

- Off-thread socket I/O with a worker pool feeding `Vec<u8>` to the GC thread, if TLS crypto
  throughput on the interp thread becomes a bottleneck.
- Higher-level protocol crates built on `cljrs-net`: HTTP/1.1, WebSocket, HTTP/2 (the aleph stack),
  each as a separate crate consuming the `{:in :out}` connection + framing layer.
- Connection pooling and a client-side reconnect/backoff helper.
- `SO_*` socket options (keepalive, nodelay, reuseaddr) on the `connect`/`listen` opts maps.
