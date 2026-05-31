# cljrs-net

**Purpose**: TCP/Unix/TLS networking for clojurust — channel-oriented sockets delivered as core.async channels.

**Status**: Phases A–D implemented (TCP client + server + framing + UDP datagrams). Phases E–F (TLS, Unix) are stubs.

**Design**: Follows the aleph/netty-in-core.async model. A connection is a duplex pair of channels — `:in` carries `byte-array` chunks read from the socket, `:out` accepts `byte-array`/string values to write. A server is a channel of connections. Higher-level protocols are higher-order functions over those channels: the `frame` function pipes a raw `:in` channel through a stateful framer spec, emitting complete application messages.

## File Layout

| File | Description |
|---|---|
| `src/lib.rs` | `init()` entry point; loads `clojure.rust.net.tcp`, `clojure.rust.net.frame`, `clojure.rust.net.udp`, and `clojure.rust.net` |
| `src/tcp.rs` | `TcpStreamResource`, `TcpListenerResource`, connection builder, accept loop, `connect`/`listen`/`close` builtins |
| `src/frame.rs` | `FramerSpec` native object, stateful framers (`LinesFramer`, `DelimiterFramer`, `LengthPrefixedFramer`), `frame`/encode builtins |
| `src/udp.rs` | `UdpSocketResource`, reader/writer tasks, `socket`/`close` builtins |
| `src/clojure_rust_net_tcp.cljrs` | Clojure source for `clojure.rust.net.tcp`; `start-server` sugar |
| `src/clojure_rust_net_frame.cljrs` | Clojure source for `clojure.rust.net.frame`; `pipe-map` helper |
| `src/clojure_rust_net_udp.cljrs` | Clojure source for `clojure.rust.net.udp`; usage examples |
| `src/clojure_rust_net.cljrs` | Clojure source for umbrella `clojure.rust.net` |
| `tests/tcp_client.rs` | Phase A integration tests (connect, send, recv, error path) |
| `tests/tcp_server.rs` | Phase B integration tests (listen, echo round-trip, close) |
| `tests/framing.rs` | Phase C integration tests (lines + length-prefixed end-to-end, framer unit tests) |
| `tests/udp.rs` | Phase D integration tests (echo round-trip, multiple senders, close) |

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
(length-prefixed {:bytes 4 :endian :big})  ; => FramerSpec — N-byte length prefix, emit byte-arrays

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

### `clojure.rust.net` (umbrella)

```clojure
(connect opts)            ;; delegates to tcp/connect; :transport key (default :tcp)
(listen opts)             ;; delegates to tcp/listen; :transport key (default :tcp)
(start-server handler opts) ;; delegates to tcp/start-server
(close x)                 ;; dispatches on :conns key: server → listen-close, else → close
```

### `clojure.rust.net.udp`

```clojure
(socket {:port 9000})
;; => {:in <chan> :out <chan> :local-addr "0.0.0.0:9000" :resource h}
;;    :in yields {:data <byte-array> :addr "ip:port"} per received datagram
;;    put {:data <byte-array> :addr "ip:port"} on :out to send

(close sock)  ;; => nil — closes :in/:out and aborts reader/writer tasks
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
```

`init` registers all three namespaces, calling `cljrs_async::init` internally. Idempotent.

### `TcpStreamResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks, dropping the socket halves and releasing the FD.

### `TcpListenerResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds the `AbortHandle` for the `accept_loop` task. `close()` aborts the task, dropping the `TcpListener` and releasing the listener FD.

### `UdpSocketResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks; once they finish the `Arc<UdpSocket>` drops and the FD is released.

## Connection Model

`connect` returns a capacity-1 promise channel (the `cljrs-io` discrete-op shape). When connected, it yields:

```clojure
{:in          <chan>          ; byte-array chunks, closed at EOF/error
 :out         <chan>          ; put byte-arrays/strings here; close to half-close
 :remote-addr "ip:port"
 :local-addr  "ip:port"
 :resource    <TcpStream>}   ; Arc<TcpStreamResource> — call (close conn) to release
```

Two tasks are spawned per connection on the `LocalSet`:
- **reader task** — reads the socket into `byte-array` chunks and puts them on `:in`
- **writer task** — takes from `:out` and writes to the socket; shuts down the write half when `:out` closes

## Server Model

`listen` binds synchronously (std socket → Tokio listener) and returns a server map:

```clojure
{:conns      <chan>          ; yields a connection map per accepted socket; closed at listener close
 :local-addr "ip:port"
 :resource   <TcpListener>} ; Arc<TcpListenerResource> — call (close server) to stop accepting
```

The accept loop runs as a `spawn_local` task. When `:conns` is full, `chan_put` parks the accept loop, applying backpressure all the way to the TCP accept window.

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
| `(length-prefixed {:bytes n})` | `byte-array` chunks | `byte-array` per frame | N-byte big-endian (default) or little-endian length header; partial frame at EOF discarded |

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
