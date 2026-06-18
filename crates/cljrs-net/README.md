# cljrs-net

**Purpose**: TCP/Unix/TLS/QUIC networking for clojurust — channel-oriented sockets delivered as core.async channels.

**Status**: Phases A–G + A2 + Q1 + Q2 implemented (TCP client + server, framing, UDP datagrams, TLS client/server, Unix-domain stream sockets, lifecycle/timeouts/ergonomics, pool-based I/O, QUIC client transport, QUIC server transport).

**Design**: Follows the aleph/netty-in-core.async model. A connection is a duplex pair of channels — `:in` carries `byte-array` chunks read from the socket, `:out` accepts `byte-array`/string values to write. A server is a channel of connections. Higher-level protocols are higher-order functions over those channels: the `frame` function pipes a raw `:in` channel through a stateful framer spec, emitting complete application messages.

**Phase A2 — pool-based I/O**: TCP and TLS byte-level I/O (connect, accept loop, TLS handshake, reader, writer) now runs on the `WorkerPool` multi-thread runtime instead of on the `LocalSet` executor thread. The LocalSet bridge tasks (`read_bridge`, `write_bridge`, `local_accept_bridge`) run on the LocalSet and are the only code that touches `GcPtr`/`Value`. This separation ensures the heap thread remains responsive under sustained byte-level traffic. `pool_io.rs` contains the shared pool tasks and bridge helpers. UDP and Unix sockets are not pool-based yet (their I/O is lower-volume).

## File Layout

| File | Description |
|---|---|
| `src/lib.rs` | `init()` entry point; loads `clojure.rust.net.tcp`, `clojure.rust.net.frame`, `clojure.rust.net.udp`, `clojure.rust.net.tls`, `clojure.rust.net.unix`, `clojure.rust.net.quic`, and `clojure.rust.net` |
| `src/pool_io.rs` | Shared pool tasks and bridge helpers: `ReadMsg`, `PoolStreamSetup`, `pool_reader`, `pool_writer`, `read_bridge`, `write_bridge`, `bytes_value`, `net_error` |
| `src/tcp.rs` | `TcpStreamResource` (Vec<AbortHandle>), `TcpListenerResource` (Vec<AbortHandle>), pool-based connect/accept, `connect`/`listen`/`close` builtins |
| `src/frame.rs` | `FramerSpec` native object, stateful framers (`LinesFramer`, `DelimiterFramer`, `LengthPrefixedFramer`), `frame`/encode builtins |
| `src/udp.rs` | `UdpSocketResource`, reader/writer tasks, `socket`/`close` builtins |
| `src/tls.rs` | `TlsStreamResource` (Vec<AbortHandle>), `TlsListenerResource` (Vec<AbortHandle>), pool-based TLS connect/accept, `build_client_config`, `build_server_config`, `tls_connect_to`/`tls_listen_on`/`connect`/`listen`/`close` builtins |
| `src/unix.rs` | `UnixStreamResource`, `UnixListenerResource` (`close` unlinks path), reader/writer loops, accept loop, `connect`/`listen`/`close` builtins; `#[cfg(unix)]` with non-Unix stubs |
| `src/quic_config.rs` | Build `quinn::ClientConfig`/`ServerConfig` from opts maps; delegates TLS to `tls::build_client_config`/`build_server_config`, wraps via `QuicClientConfig::try_from`; applies QUIC transport params (`:max-idle-ms`, `:keep-alive-ms`, `:max-streams`) |
| `src/quic.rs` | `QuicConnectionResource` (holds `quinn::Connection` + `Endpoint` + abort handles), `QuicStreamResource` (abort handles for pool reader/writer + LocalSet bridges), `QuicListenerResource` (holds `Endpoint` + abort handles for pool accept loop + LocalSet bridge), pool accept/open loops, LocalSet bridges, `connect_to`/`open_stream_on`/`listen_on`, `connect`/`open-stream`/`close`/`stream-close`/`listen`/`listen-close` builtins |
| `src/clojure_rust_net_tcp.cljrs` | Clojure source for `clojure.rust.net.tcp`; `start-server` sugar |
| `src/clojure_rust_net_frame.cljrs` | Clojure source for `clojure.rust.net.frame`; `pipe-map` helper |
| `src/clojure_rust_net_udp.cljrs` | Clojure source for `clojure.rust.net.udp`; usage examples |
| `src/clojure_rust_net_tls.cljrs` | Clojure source for `clojure.rust.net.tls`; `start-server` sugar |
| `src/clojure_rust_net_unix.cljrs` | Clojure source for `clojure.rust.net.unix`; `start-server` sugar |
| `src/clojure_rust_net_quic.cljrs` | Clojure source for `clojure.rust.net.quic`; `with-stream`, `drain-stream`, `start-server` sugar |
| `src/clojure_rust_net.cljrs` | Clojure source for umbrella `clojure.rust.net`; dispatches on `:transport` (`:tcp`, `:tls`, `:unix`) |
| `tests/tcp_client.rs` | Phase A integration tests (connect, send, recv, error path) |
| `tests/tcp_server.rs` | Phase B integration tests (listen, echo round-trip, close) |
| `tests/framing.rs` | Phase C integration tests (lines + length-prefixed end-to-end, framer unit tests) |
| `tests/udp.rs` | Phase D integration tests (echo round-trip, multiple senders, close) |
| `tests/tls.rs` | Phase E integration tests (TLS echo round-trip with rcgen self-signed cert, connect failure) |
| `tests/unix.rs` | Phase F integration tests (Unix echo round-trip, listener unlinks path, stale socket removal) |
| `tests/lifecycle.rs` | Phase G integration tests (split-err, drain-to, with-open, :connect-timeout-ms) |
| `tests/quic.rs` | Phases Q1+Q2 integration tests (QUIC echo round-trip via quinn in-test server, QUIC server echo via `listen_on`, listener close, connect-failure path) |

## Public API

### `clojure.rust.net.tcp`

```clojure
;; Phase A — client
(connect {:host "example.com" :port 80})
;; => promise-chan — yields {:in ch :out ch :remote-addr str :local-addr str :resource h}
;;    or Value::Error on failure

(close conn)         ;; => nil — closes :in/:out and aborts reader/writer tasks

;; Phase B — server
(listen {:port 8080})
;; => {:conns <chan> :local-addr "0.0.0.0:8080" :resource h}
;;    :conns yields a connection map per accepted socket; closed when listener closes

(listen-close server) ;; => nil — stops accept loop, closes :conns

(start-server handler {:port 8080})
;; => server-map — spawns go-loop that calls (handler conn) for each accepted conn
```

### `clojure.rust.net.frame`

```clojure
;; Decoder specs — pass to `frame`
(lines)                              ; => FramerSpec — split on \n, emit strings
(by-delimiter b)                     ; => FramerSpec — split on byte b, emit byte-arrays
(length-prefixed {:bytes 4 :endian :big :max-frame 16777216})  ; => FramerSpec — N-byte length prefix, emit byte-arrays

;; Main framing helper
(frame in-chan spec)                 ; => out-chan of decoded messages
(frame in-chan spec out-buf)         ; => out-chan with custom buffer depth

;; Encode helpers (write / outgoing direction)
(lines-encode str)                   ; => byte-array (UTF-8 + \n)
(length-prefixed-encode ba opts)     ; => byte-array (N-byte header prepended to ba)

;; Pipe-fn helper (Clojure source)
(pipe-map in-chan f)                 ; => out-chan — async map over channel values
(pipe-map in-chan f out-buf)
```

`frame` spawns a `LocalSet` background task that reads byte-arrays from `in-chan`, feeds them through the stateful framer, and puts complete frames on the returned output channel. Errors from `in-chan` are forwarded in-band; the output channel closes at EOF or on error.

### `clojure.rust.net.tls`

```clojure
;; Phase E — TLS client
(connect {:host "example.com" :port 443})
;; => promise-chan — yields {:in ch :out ch :remote-addr str :local-addr str :resource h}
;;    or Value::Error on failure
;; Optional keys: :roots (:webpki default, :system, or "path/to/ca.pem")
;;                :insecure-skip-verify true (skip cert verification — for testing only)
;;                :alpn ["h2" "http/1.1"]
;;                :in-buf, :out-buf (default 8)

(close conn)         ;; => nil — closes :in/:out and aborts reader/writer tasks

;; Phase E — TLS server
(listen {:port 8443 :cert "cert.pem" :key "key.pem"})
;; => {:conns <chan> :local-addr "0.0.0.0:8443" :resource h}
;;    :conns yields a connection map per accepted TLS socket; closed when listener closes
;; Optional keys: :host (default "0.0.0.0"), :alpn [...], :conns-buf, :in-buf, :out-buf

(listen-close server) ;; => nil — stops accept loop, closes :conns

(start-server handler {:port 8443 :cert "cert.pem" :key "key.pem"})
;; => server-map — spawns go-loop that calls (handler conn) for each accepted TLS conn
```

### `clojure.rust.net.unix`

Unix-domain stream sockets. `#[cfg(unix)]` — only on Unix targets; non-Unix
builds register stub functions that throw "not supported on this platform".

```clojure
;; Phase F — Unix client
(connect {:path "/tmp/app.sock"})
;; => promise-chan — yields {:in ch :out ch :remote-addr str :local-addr str :resource h}
;;    :remote-addr / :local-addr are filesystem paths (empty for unnamed sockets)
;; Optional keys: :in-buf, :out-buf (default 8)

(close conn)         ;; => nil — closes :in/:out and aborts reader/writer tasks

;; Phase F — Unix server
(listen {:path "/tmp/app.sock"})
;; => {:conns <chan> :local-addr "/tmp/app.sock" :resource h}
;;    (close server) also unlinks the socket file

(listen-close server) ;; => nil — stops accept loop, unlinks socket path, closes :conns

(start-server handler {:path "/tmp/app.sock"})
;; => server-map — spawns go-loop that calls (handler conn) for each accepted conn
```

### `clojure.rust.net` (umbrella)

```clojure
;; Phase A–F transport dispatch
(connect opts)              ;; :transport selects :tcp (default), :tls, :unix
                            ;; :connect-timeout-ms N — races connect against timeout(N)
(listen opts)               ;; :transport selects :tcp (default), :tls, :unix
(start-server handler opts) ;; :transport selects :tcp (default), :tls, :unix
(close x)                   ;; dispatches on map shape: :conns → server, :remote-addr → conn, else → udp

;; Phase G — lifecycle and ergonomics
(with-open [c (await (take! (connect ...)))] body...)
;; => try/finally that calls close on each binding; ensures FDs are released

(split-err in-chan)         ;; => {:out values-chan :err err-promise}
(split-err in-chan out-buf) ;; :out gets non-error values; :err gets error-or-nil at EOF

(drain-to in-chan)          ;; ^:async — blocks until close/error
                            ;; => {:values [...] :error err-or-nil}
```

**Half-close semantics**: `(close! (:out conn))` sends FIN (write half-close) so the
peer sees EOF on reads, while `:in` continues draining until the peer closes its write
side. `(close conn)` tears down both halves.

### `clojure.rust.net.udp`

```clojure
(socket {:port 9000})
;; => {:in <chan> :out <chan> :local-addr "0.0.0.0:9000" :resource h}
;;    :in yields {:data <byte-array> :addr "ip:port"} per received datagram
;;    put {:data <byte-array> :addr "ip:port"} on :out to send

(close sock)  ;; => nil — closes :in/:out and aborts reader/writer tasks
```

### `clojure.rust.net.quic` (Phases Q1+Q2)

```clojure
;; Phase Q1 — QUIC client
(connect {:host "h" :port 4433 :alpn ["hq-interop"] :insecure-skip-verify true})
;; => promise-chan — yields {:streams ch :remote-addr str :local-addr str :resource h}
;;    or Value::Error on failure
;; Optional keys: :insecure-skip-verify, :alpn, :roots (same as tls/connect)
;;                :max-idle-ms, :keep-alive-ms, :max-streams (QUIC transport params)
;;                :streams-buf, :in-buf, :out-buf (default 8)

(open-stream conn)                    ;; => promise-chan yielding stream map
(open-stream conn {:in-buf N :out-buf N})
;; stream map: {:in ch :out ch :stream-id long :resource h}

(close conn)                          ;; => nil — sends CONNECTION_CLOSE, aborts tasks
(stream-close stream)                 ;; => nil — aborts stream tasks (sends RESET/FIN)

;; Phase Q2 — QUIC server
(listen {:port 4433 :cert "cert.pem" :key "key.pem"})
;; => {:conns <chan> :local-addr "ip:port" :resource h}
;;    :conns yields a connection map per accepted QUIC connection; closed when listener closes
;;    connection map: {:streams <chan> :remote-addr str :local-addr str :resource h}
;;    :streams yields stream maps per accepted bidi stream: {:in ch :out ch :stream-id long :resource h}
;; Optional keys: :host (default "0.0.0.0"), :alpn [...], :max-idle-ms, :keep-alive-ms
;;                :max-streams, :conns-buf, :streams-buf, :in-buf, :out-buf (default 8)

(listen-close server)                 ;; => nil — sends CONNECTION_CLOSE, closes :conns

;; Clojure sugar:
(with-stream conn (fn [s] ...))       ;; open, use, close
(drain-stream stream)                 ;; => byte-array of all :in data (^:async context)
(start-server handler {:port 4433 :cert "cert.pem" :key "key.pem"})
;; => server-map — spawns go-loop that calls (handler conn) for each accepted QUIC conn
```

### Rust

```rust
pub fn init(globals: &Arc<GlobalEnv>)
pub fn tcp::connect_to(host: &str, port: u16, in_buf: usize, out_buf: usize) -> Value
pub fn tcp::listen_on(host: &str, port: u16, conns_buf: usize, in_buf: usize, out_buf: usize) -> ValueResult<Value>
pub fn udp::socket_on(host: &str, port: u16, in_buf: usize, out_buf: usize) -> ValueResult<Value>
pub fn frame::frame_channel(in_chan: GcPtr<NativeObjectBox>, spec: FramerSpec, out_buf: usize) -> GcPtr<NativeObjectBox>
pub fn frame::encode_line(s: &str) -> Value
pub fn frame::encode_length_prefixed(data: &[u8], prefix_len: usize, big_endian: bool) -> Value
pub fn tls::tls_connect_to(host: &str, port: u16, config: Arc<rustls::ClientConfig>, in_buf: usize, out_buf: usize) -> Value
pub fn tls::tls_listen_on(host: &str, port: u16, config: Arc<rustls::ServerConfig>, conns_buf: usize, in_buf: usize, out_buf: usize) -> ValueResult<Value>
pub fn tls::build_client_config(opts: &MapValue) -> ValueResult<Arc<rustls::ClientConfig>>
pub fn tls::build_server_config(opts: &MapValue) -> ValueResult<Arc<rustls::ServerConfig>>
#[cfg(unix)] pub fn unix::connect_to(path: &str, in_buf: usize, out_buf: usize) -> Value
#[cfg(unix)] pub fn unix::listen_on(path: &str, conns_buf: usize, in_buf: usize, out_buf: usize) -> ValueResult<Value>
pub fn quic::connect_to(host: &str, port: u16, config: quinn::ClientConfig, streams_buf: usize, in_buf: usize, out_buf: usize) -> Value
pub fn quic::open_stream_on(connection: quinn::Connection, in_buf: usize, out_buf: usize) -> Value
pub fn quic::listen_on(host: &str, port: u16, server_config: quinn::ServerConfig, conns_buf: usize, streams_buf: usize, in_buf: usize, out_buf: usize) -> ValueResult<Value>
pub fn quic_config::client_config(opts: &MapValue) -> ValueResult<quinn::ClientConfig>
pub fn quic_config::server_config(opts: &MapValue) -> ValueResult<quinn::ServerConfig>
pub const NS_QUIC: &str  // "clojure.rust.net.quic"
```

`init` registers all namespaces including `clojure.rust.net.quic`, calling `cljrs_async::init` internally. Idempotent.

### `TcpStreamResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `Vec<AbortHandle>` for the pool reader, pool writer, and LocalSet bridge tasks (4 total). `close()` aborts all handles, dropping the pool socket halves and releasing the FD.

### `TcpListenerResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `Vec<AbortHandle>` for the pool accept loop and the LocalSet accept bridge (2 total). `close()` aborts all handles, dropping the `TcpListener` and releasing the listener FD.

### `UdpSocketResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks; once they finish the `Arc<UdpSocket>` drops and the FD is released.

### `TlsStreamResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `Vec<AbortHandle>` for the pool reader, pool writer, and LocalSet bridge tasks (4 total). `close()` aborts all handles, dropping the TLS stream halves and releasing the FD.

### `TlsListenerResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `Vec<AbortHandle>` for the pool TLS accept loop and the LocalSet accept bridge (2 total). `close()` aborts all handles, dropping the `TcpListener` and releasing the listener FD.

### `UnixStreamResource` (`#[cfg(unix)]`)

Implements `cljrs_value::Resource` (Arc-backed). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks, dropping the Unix stream halves and releasing the FD.

### `QuicConnectionResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds the `quinn::Connection` (for `open_bi`/`close` calls) and the `quinn::Endpoint` (kept alive so the internal UDP driver doesn't shut down while the connection is live). Also holds `Vec<AbortHandle>` for the peer-stream accept loop and LocalSet `:streams` bridge (2 total). `close()` aborts all handles and calls `connection.close(0, b"closed")`, sending a QUIC CONNECTION_CLOSE frame to the peer. `resource_type` → `"QuicConnection"`.

### `QuicStreamResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `Vec<AbortHandle>` for the pool reader, pool writer, and LocalSet bridge tasks (4 total). `close()` aborts all handles; dropping the `SendStream`/`RecvStream` on the pool causes quinn to send RESET_STREAM / STOP_SENDING to the peer. `resource_type` → `"QuicStream"`.

### `QuicListenerResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds the `quinn::Endpoint` (keeps the UDP driver alive and provides `close()`) and `Vec<AbortHandle>` for the pool accept loop and the LocalSet connection bridge (2 total). `close()` aborts both handles and calls `endpoint.close(0, b"listener closed")`, sending QUIC CONNECTION_CLOSE to any in-flight handshakes. `resource_type` → `"QuicListener"`.

### `UnixListenerResource` (`#[cfg(unix)]`)

Implements `cljrs_value::Resource` (Arc-backed). Holds the `AbortHandle` for the `accept_loop` task and the socket's filesystem `path`. `close()` aborts the task and **unlinks the socket path** via `std::fs::remove_file`, so the next `listen_on` on the same path does not get EADDRINUSE. `listen_on` also pre-unlinks any stale socket file before binding.

## Connection Model

`connect` returns a capacity-1 promise channel (the `cljrs-io` discrete-op shape). When connected, it yields:

```clojure
{:in          <chan>          ; byte-array chunks, closed at EOF/error
 :out         <chan>          ; put byte-arrays/strings here; close to half-close
 :remote-addr "ip:port"
 :local-addr  "ip:port"
 :resource    <TcpStream>}   ; Arc<TcpStreamResource> — call (close conn) to release
```

Four tasks are spawned per connection (A2 pool model):
- **pool reader** (pool thread) — reads the socket into `Vec<u8>` chunks and sends `ReadMsg` to the LocalSet bridge
- **pool writer** (pool thread) — receives `Vec<u8>` from the LocalSet bridge and writes to the socket; shuts down gracefully when the sender is dropped
- **read bridge** (LocalSet) — converts `ReadMsg` → `Value::ByteArray` and puts on `:in`
- **write bridge** (LocalSet) — takes from `:out`, converts to `Vec<u8>`, sends to the pool writer

## Server Model

`listen` binds synchronously (std socket → Tokio listener) and returns a server map:

```clojure
{:conns      <chan>          ; yields a connection map per accepted socket; closed at listener close
 :local-addr "ip:port"
 :resource   <TcpListener>} ; Arc<TcpListenerResource> — call (close server) to stop accepting
```

The accept loop runs on the `WorkerPool` (`pool_accept_loop`). A LocalSet bridge (`local_accept_bridge`) receives the `PoolStreamSetup` for each accepted connection and spawns the read/write bridges. When `:conns` is full, `chan_put` parks the bridge, applying backpressure all the way to the pool accept loop.

## Usage Example

```clojure
(require '[clojure.rust.net :as net])
(require '[clojure.core.async :refer [go <!! close!]])

;; Echo server
(let [server (net/listen {:port 8080})]
  (go (loop []
        (when-let [conn (<! (:conns server))]
          (go (loop []
                (when-let [chunk (<! (:in conn))]
                  (>! (:out conn) chunk)
                  (recur)))
              (close! (:out conn)))
          (recur))))
  ;; ... later ...
  (net/close server))

;; Or with start-server sugar:
(net/start-server
  (fn [conn]
    (go (loop []
          (when-let [chunk (<! (:in conn))]
            (>! (:out conn) chunk)
            (recur)))
        (close! (:out conn))))
  {:port 8080})
```

## Framing Model (Phase C)

`frame` turns a raw `:in` byte-stream channel into a channel of application messages by
piping chunks through a stateful framer. The framer handles TCP segmentation: chunks
can be split across frame boundaries or contain multiple frames.

```clojure
(require '[clojure.rust.net.frame :as frame])

;; Line-protocol server: read \n-delimited strings
(net/start-server
  (fn [conn]
    (let [lines (frame/frame (:in conn) (frame/lines))]
      (go (loop []
            (when-let [line (<! lines)]
              (println "got line:" line)
              ;; echo back
              (>! (:out conn) (frame/lines-encode line))
              (recur)))
          (close! (:out conn)))))
  {:port 8080})

;; Length-prefixed protocol: 4-byte big-endian length header
(net/start-server
  (fn [conn]
    (let [msgs (frame/frame (:in conn) (frame/length-prefixed {:bytes 4}))]
      (go (loop []
            (when-let [msg (<! msgs)]
              ;; msg is a byte-array of exactly the declared length
              (>! (:out conn) (frame/length-prefixed-encode msg {:bytes 4}))
              (recur)))
          (close! (:out conn)))))
  {:port 9090})
```

### Stateful framers

| Spec | Input | Output | Notes |
|---|---|---|---|
| `(lines)` | `byte-array` chunks | `string` per line | strips `\r`, emits partial final line at EOF |
| `(by-delimiter b)` | `byte-array` chunks | `byte-array` per frame | `b` is excluded from frames |
| `(length-prefixed {:bytes n})` | `byte-array` chunks | `byte-array` per frame | N-byte big-endian (default) or little-endian length header; partial frame at EOF discarded. `:max-frame` (default 16 MiB) caps the declared body size — an oversized header emits an error frame instead of buffering, preventing memory exhaustion from a malicious peer |

### Encode helpers

| Function | Returns |
|---|---|
| `(lines-encode str)` | `byte-array` — UTF-8 bytes of `str` followed by `\n` |
| `(length-prefixed-encode ba opts)` | `byte-array` — N-byte length header prepended to `ba` |

### Pipe-fn pattern

For protocols that need async logic (request/response correlation, lookups), use
a `(fn [in-chan] -> out-chan)` that spawns a `go-loop`. The `pipe-map` helper
in this namespace covers the simple map-over-channel case:

```clojure
;; Transform each decoded message asynchronously
(let [msgs   (frame/frame (:in conn) (frame/lines))
      parsed (frame/pipe-map msgs parse-json)]
  ...)
```
