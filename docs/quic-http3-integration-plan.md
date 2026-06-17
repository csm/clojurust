# Plan: Integrate QUIC + HTTP/3 into `cljrs-net` via quinn

## Context

`cljrs-net` (`crates/cljrs-net/`) is clojurust's channel-oriented networking
transport layer: TCP, TLS (rustls), UDP, Unix sockets, and pluggable framing,
all exposed to `.cljrs` code as connection/socket maps
(`{:in :out :remote-addr :local-addr :resource}`) over `cljrs-async` CSP
channels. There is **no QUIC or HTTP-level support today** — TLS accepts an
`:alpn` option but never negotiates on it.

This change adds **QUIC and HTTP/3** (client and server) by integrating the
**quinn** crate, exposing two new Clojure-facing namespaces:
`clojure.rust.net.quic` (raw multiplexed QUIC transport) and
`clojure.rust.net.h3` (HTTP/3 request/response).

### Library decision: quinn (not quiche, not quinn-proto)

An earlier draft targeted Cloudflare's **quiche**, but quiche is a sans-IO
state machine (you hand-write the UDP pump, timers, connection-ID demux, and
stateless Retry) and bundles **BoringSSL** (a cmake + C/C++ native build on
every `net` build). Both fight cljrs-net's existing architecture. We pivoted to
**quinn 0.11** because it fits almost perfectly:

- **No new native build.** quinn's default `rustls-ring` crypto provider is the
  **same rustls + `ring` backend cljrs-net already uses** (`tls.rs`). The
  BoringSSL/cmake burden disappears.
- **`SendStream`/`RecvStream` implement tokio `AsyncRead`/`AsyncWrite`.** Since
  `pool_reader`/`pool_writer` in `pool_io.rs` are already generic over
  `AsyncRead`/`AsyncWrite + Send + 'static`, each QUIC stream becomes an
  ordinary `PoolStreamSetup` feeding the existing `read_bridge`/`write_bridge`
  **with no changes**.
- **quinn owns its UDP socket + endpoint driver + timers internally.** There is
  **no hand-written event loop** — just `Endpoint::connect`/`accept`,
  `Connection::open_bi`/`accept_bi`.
- **Reuses the rustls config builders.** `tls.rs`'s
  `build_client_config`/`build_server_config` produce
  `rustls::ClientConfig`/`ServerConfig`, which wrap into quinn via
  `quinn::crypto::rustls::QuicClientConfig::try_from(...)` /
  `QuicServerConfig::try_from(...)`. ALPN/cert/key/insecure handling is shared.
- Everything (`Endpoint`, `Connection`, streams) is `Send`, so it lives on the
  **WorkerPool**, with GcPtr work in the LocalSet bridges — the existing
  two-layer split, unchanged.

(**quinn-proto** keeps the rustls benefit but reintroduces the manual sans-IO
driver, so it was rejected in favour of high-level quinn.)

### Other decisions
- **Scope:** Full QUIC + HTTP/3, client **and** server, phased Q1–Q4.
- **HTTP/3 now:** use `h3` (0.0.8) + `h3-quinn` (0.0.10), accepting that the
  `h3` crate is pre-1.0 / experimental. This is the one area quiche was ahead
  (mature built-in HTTP/3); flagged as a risk below.
- **Multiplexed stream API:** a QUIC connection exposes a `:streams`
  accept-channel and an `open-stream` builtin; each stream is its own
  `{:in :out}` pair.
- **Build gating:** on by default with `net` (quinn/h3 are plain, non-optional
  deps of `cljrs-net`).

## Architecture

quinn collapses what quiche would have needed into the existing model:

```
        WorkerPool (Send, multi-thread)              LocalSet (GcPtr, single-thread)
 ┌──────────────────────────────────────────┐   ┌────────────────────────────────────┐
 │ quinn::Endpoint (owns UDP + driver task)  │   │ per-stream read_bridge  (ReadMsg→ch)│
 │ quinn::Connection                         │   │ per-stream write_bridge (ch→bytes)  │
 │ accept loop: conn.accept_bi() ─┐          │◄─►│ stream-accept bridge → :streams chan│
 │ open:        conn.open_bi()  ──┤          │   │ conn-accept bridge → :conns chan    │
 │   each (SendStream,RecvStream) │          │   │ h3 event bridge → req/resp maps     │
 │     → pool_reader/pool_writer ─┘          │   └────────────────────────────────────┘
 └──────────────────────────────────────────┘      (all GcPtr/Value work lives here)
```

- A QUIC **stream** = a `(SendStream, RecvStream)` pair. `RecvStream` →
  `pool_reader` → `ReadMsg` mpsc → `read_bridge` → `:in` channel. `:out`
  channel → `write_bridge` → bytes mpsc → `pool_writer` → `SendStream`. This is
  **exactly** the existing `PoolStreamSetup` flow used by TCP/TLS — reused
  unchanged.
- A QUIC **connection** runs a small `Send` pool task that loops on
  `connection.accept_bi()` and, for each accepted stream, builds a
  `PoolStreamSetup` + per-stream resource, handing the stream map to a LocalSet
  bridge that puts it on the connection's `:streams` channel. `open-stream`
  triggers `connection.open_bi()` the same way.
- A QUIC **server** = a `quinn::Endpoint` in server mode; a pool task loops on
  `endpoint.accept().await` → `incoming.await` → `Connection`, building a
  connection map handed to the `:conns` LocalSet bridge.
- Connection setup is async (handshake), so `connect`/`open-stream` return a
  **promise channel** (`make_chan(1)` + `chan_deliver`), matching the existing
  TCP/TLS `connect` shape.

## Changes

### Cargo / build wiring
- `Cargo.toml` (workspace): add to `[workspace.dependencies]`:
  - `quinn = { version = "0.11", default-features = false, features = ["runtime-tokio", "rustls-ring"] }`
    — pin the crypto provider to **ring** to match the existing rustls/ring
    dependency and avoid pulling in aws-lc-rs (which would add a C build).
  - `h3 = "0.0.8"`, `h3-quinn = "0.0.10"`.
- `crates/cljrs-net/Cargo.toml`: add `quinn`, `h3`, `h3-quinn` as plain
  (non-optional) deps (`workspace = true` for quinn). `rustls`/`tokio-rustls`
  are already present; `rcgen` (dev) and tokio test features already present.
- No new cargo feature: `cljrs-net` only compiles when the `cljrs` `net`
  feature is on, so QUIC ships with `net` automatically.

### New source files in `crates/cljrs-net/src/`

| File | Responsibility |
|---|---|
| `quic_config.rs` | Build quinn client/server configs. **Reuse** `tls.rs::build_client_config`/`build_server_config` to get `rustls::ClientConfig`/`ServerConfig` from the opts map (`:cert :key :alpn :insecure-skip-verify`), then wrap: `QuicClientConfig::try_from(rustls_cfg)` → `quinn::ClientConfig`; `QuicServerConfig::try_from(rustls_cfg)` → `quinn::ServerConfig`. Apply QUIC transport params (`:max-idle-ms`, `:max-streams`, `:keep-alive-ms`) via `quinn::TransportConfig`. |
| `quic.rs` | Clojure-facing QUIC transport: `register` + `connect`/`listen`/`open-stream`/`close`/`listen-close` builtins; the pool accept/open loops; the LocalSet bridges turning quinn streams into `{:in :out}` maps via `PoolStreamSetup`; the resource types. Analogue of `tcp.rs`/`tls.rs`, reusing `pool_io.rs`. |
| `h3.rs` | HTTP/3 layer over `h3`/`h3-quinn`: client `get`/`request` (drive `h3::client::SendRequest`, stream the response body to a `:body` channel) and server `start-server` (drive `h3::server::Connection`, build request maps with a `respond` fn). Defines `H3Resource`. |
| `clojure_rust_net_quic.cljrs` | Clojure sugar/docstrings for `clojure.rust.net.quic`: `start-server` go-loop helper, stream ergonomics, `with-open` interop. Mirrors `clojure_rust_net_tcp.cljrs`. |
| `clojure_rust_net_h3.cljrs` | Clojure sugar for `clojure.rust.net.h3`: client `get`/`request` returning a promise channel; server `start-server` over a handler. |

### Modified files
- `crates/cljrs-net/src/pool_io.rs`: **no structural change expected** —
  `pool_reader`/`pool_writer`/`read_bridge`/`write_bridge`/`PoolStreamSetup`
  are reused as-is. (Add a small constructor helper only if needed to build a
  `PoolStreamSetup` directly from a quinn `(SendStream, RecvStream)` pair
  without a single `AsyncRead+AsyncWrite` object, since quinn splits them.)
- `crates/cljrs-net/src/tls.rs`: make `build_client_config`/`build_server_config`
  reachable from `quic_config.rs` (they are already `pub`; confirm visibility).
  No behavioural change.
- `crates/cljrs-net/src/lib.rs`: add `NS_QUIC` (`clojure.rust.net.quic`) and
  `NS_H3` (`clojure.rust.net.h3`) constants + two `init` blocks following the
  idempotent `is_loaded` pattern; embed the two new `.cljrs` sources.
- `crates/cljrs-net/src/clojure_rust_net.cljrs`: add `:quic` cases to the
  umbrella `connect`/`listen`/`close` dispatch (h3 referenced as a sibling
  namespace; its map shape differs from byte-stream conns).

### Resource types (all `Arc<Mutex<Inner>>` + `Vec<AbortHandle>`, like `UdpSocketResource`)

The `Resource` trait (`crates/cljrs-value/src/resource.rs`) needs only
`close`/`is_closed`/`resource_type`/`as_any` — no `Trace`/`NativeObject`.

- **`QuicConnectionResource`** — holds the `quinn::Connection` (to call
  `close()` with an app error code on shutdown) + abort handles for the pool
  accept loop and LocalSet `:streams` bridge. `resource_type` → `"QuicConnection"`.
- **`QuicStreamResource`** — abort handles for the stream's `pool_reader`/
  `pool_writer`/bridges; `close` aborts them (quinn sends RESET/FIN on stream
  drop). `resource_type` → `"QuicStream"`.
- **`QuicListenerResource`** — holds the `quinn::Endpoint` + abort handles for
  the pool accept loop and LocalSet `:conns` bridge; `close` calls
  `endpoint.close(...)`. `resource_type` → `"QuicListener"`.
- **`H3Resource`** — wraps a `QuicConnectionResource` + the h3 driver abort
  handles. `resource_type` → `"H3Connection"`.

Holding the `quinn::Connection`/`Endpoint` in the resource keeps quinn's driver
alive; dropping/closing them is the deterministic cleanup path (same philosophy
as `udp.rs`).

## Clojure-facing API

### `clojure.rust.net.quic`
```clojure
;; (connect opts) -> promise chan yielding a connection map:
{:streams     <chan>   ; yields a stream map per peer-initiated stream
 :open-stream <fn>     ; (open-stream conn {:bidi? true}) -> promise chan -> stream map
 :remote-addr "ip:port" :local-addr "ip:port"
 :resource    <QuicConnectionResource>}

;; a stream map:
{:in <chan>   ; byte-array chunks; closed at stream FIN
 :out <chan>  ; put byte-arrays/strings; close! sends FIN
 :stream-id 0 :resource <QuicStreamResource>}

;; (listen opts) -> server map:
{:conns <chan> :local-addr "ip:port" :resource <QuicListenerResource>}
```
Builtins: `connect`, `listen`, `open-stream`, `close`, `listen-close`. Option
keys reuse `tls.rs` names (`:host :port :alpn :cert :key :insecure-skip-verify
:in-buf :out-buf :conns-buf`) plus `:max-idle-ms :max-streams :keep-alive-ms`.

Client example:
```clojure
(go
  (let [conn   (<! (quic/connect {:host "h" :port 4433 :alpn ["hq-interop"]
                                  :insecure-skip-verify true}))
        stream (<! (quic/open-stream conn {}))]
    (>! (:out stream) (.getBytes "GET /\r\n"))
    (close! (:out stream))                         ; FIN
    (loop [] (when-let [c (<! (:in stream))] (recur)))
    (quic/close conn)))
```
Server example:
```clojure
(quic/start-server
  (fn [conn]
    (go-loop []
      (when-let [s (<! (:streams conn))]
        (go (>! (:out s) (echo (<! (:in s)))) (close! (:out s)))
        (recur))))
  {:port 4433 :cert "cert.pem" :key "key.pem" :alpn ["hq-interop"]})
```

### `clojure.rust.net.h3`
```clojure
;; client (get url opts) / (request req opts) -> promise chan yielding:
{:status 200 :headers {...} :body <chan> :resource <H3Resource>}
;; server handler receives:
{:method "GET" :path "/" :authority "h" :scheme "https"
 :headers {...} :body <chan> :respond <fn>}   ; (respond {:status :headers :body})
```
Client: `(h3/get "https://h:4433/" {:insecure-skip-verify true})`.
Server: `(h3/start-server (fn [{:keys [method path respond]}] (respond {...})) {:port :cert :key})`.

## Phasing (each phase: code + Clojure source + README/TODO in one commit)
- **Q1 — QUIC client transport.** `quic_config.rs` (client config from rustls),
  `quic.rs` `connect`/`open-stream`/`close`, the pool open loop + per-stream
  `PoolStreamSetup` wiring, `QuicConnectionResource`/`QuicStreamResource`.
  Test against a quinn-driven in-test echo server. Smaller than the quiche
  equivalent — no event loop to build.
- **Q2 — QUIC server transport.** `quic.rs` `listen`/`listen-close`, the
  `endpoint.accept()` pool loop, `:conns`/`:streams` LocalSet bridges,
  `QuicListenerResource`. Depends on Q1.
- **Q3 — HTTP/3 client.** `h3.rs` client over `h3-quinn`: `h3/get`/`request`,
  response-body streaming to a `:body` channel. Depends on Q1.
- **Q4 — HTTP/3 server.** `h3.rs` server: request map + `respond` fn,
  `send_response`/`send_data`. Depends on Q2+Q3.
- **Q5 (optional) — QUIC datagrams.** quinn `send_datagram`/`read_datagram`;
  connection-level `:dgram-in`/`:dgram-out` channels.

## Verification
- New tests `crates/cljrs-net/tests/quic.rs` and `tests/h3.rs`, mirroring
  `tests/tls.rs`:
  - Setup: `let globals = cljrs_stdlib::standard_env(); cljrs_net::init(&globals);`
    on a `current_thread` runtime + `LocalSet::block_on` (as `tls.rs` does), so
    both the LocalSet bridges and the WorkerPool pool tasks run.
  - Drive Rust entry points directly (`quic::connect_to`, `quic::listen_on`,
    `h3::get`) — same style as `tls.rs`.
  - Certs via `rcgen::generate_simple_self_signed(vec!["localhost"])`. quinn can
    take cert/key in-memory (`CertificateDer`/`PrivateKeyDer`) via the rustls
    config, so no temp PEM file is strictly required (simpler than quiche).
    Client uses `:insecure-skip-verify true` →
    `rustls` dangerous `NoServerVerification` (reuse `tls.rs`'s existing
    insecure verifier).
  - Bind port 0; read the assigned port from `:local-addr`.
  - **Q1/Q2 echo:** client `open-stream`, write bytes, `close!` (FIN); server
    echoes; client drains `:in` to nil; assert equality.
  - **Q3/Q4 round-trip:** `h3/get`, assert `:status` 200 and drained `:body`.
  - **Failure path:** `connect` with short `:max-idle-ms` to a dead UDP port,
    assert the promise channel yields `Value::Error` (QUIC handshake-timeout
    path; no instant connection-refused).
- Run: `cargo test -p cljrs-net`; `cargo build -p cljrs --features net` to
  confirm the binary links quinn/h3.
- Update `crates/cljrs-net/README.md` (every new source file + public API:
  `quic::{register, connect_to, listen_on, open_stream_on}`,
  `Quic{Connection,Stream,Listener}Resource`,
  `quic_config::{client_config, server_config}`,
  `h3::{register, get, request, start_server}`, `H3Resource`, new
  `NS_QUIC`/`NS_H3` consts). Note the quinn/`rustls-ring` reuse and that QUIC
  streams reuse `pool_io.rs`. Add Q1–Q5 to `TODO.md`.

## Risks
1. **`h3`/`h3-quinn` are pre-1.0 / experimental** ("there may still be bugs").
   This is the one place quiche was ahead. Mitigation: the raw QUIC layer
   (Q1/Q2, on stable quinn) is fully usable on its own; HTTP/3 (Q3/Q4) is
   isolated in `h3.rs` and can be pinned/updated independently.
2. **quinn crypto provider must be pinned to `ring`.** quinn's default features
   include `rustls-aws-lc-rs`, which adds a C build. The plan pins
   `default-features = false` + `rustls-ring` to match the existing backend and
   avoid that — must verify only one rustls `CryptoProvider` is installed
   process-wide (set it explicitly if needed).
3. **Runtime context.** quinn's `Endpoint` must be created inside a tokio
   runtime; it spawns its driver via the `runtime-tokio` feature. Endpoints/
   streams must be created on a tokio context (the WorkerPool handle) — confirm
   the WorkerPool runtime is multi-thread tokio (it is, per `cljrs-async`).
4. **Stream split.** quinn yields `SendStream` + `RecvStream` separately (not a
   single `AsyncRead+AsyncWrite`), so `PoolStreamSetup` construction must accept
   the two halves directly — a minor helper, not a redesign.
