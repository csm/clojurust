# cljrs-net

**Purpose**: TCP/Unix/TLS networking for clojurust — channel-oriented sockets delivered as core.async channels.

**Status**: Phases A–B implemented (TCP client + server). Phases C–F (framing, UDP, TLS, Unix) are stubs.

**Design**: Follows the aleph/netty-in-core.async model. A connection is a duplex pair of channels — `:in` carries `byte-array` chunks read from the socket, `:out` accepts `byte-array`/string values to write. A server is a channel of connections. Higher-level protocols are higher-order functions over those channels.

## File Layout

| File | Description |
|---|---|
| `src/lib.rs` | `init()` entry point; loads `clojure.rust.net.tcp` and `clojure.rust.net` |
| `src/tcp.rs` | `TcpStreamResource`, `TcpListenerResource`, connection builder, accept loop, `connect`/`listen`/`close` builtins |
| `src/clojure_rust_net_tcp.cljrs` | Clojure source for `clojure.rust.net.tcp`; `start-server` sugar |
| `src/clojure_rust_net.cljrs` | Clojure source for umbrella `clojure.rust.net` |
| `tests/tcp_client.rs` | Phase A integration tests (connect, send, recv, error path) |
| `tests/tcp_server.rs` | Phase B integration tests (listen, echo round-trip, close) |

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

### `clojure.rust.net` (umbrella)

```clojure
(connect opts)            ;; delegates to tcp/connect; :transport key (default :tcp)
(listen opts)             ;; delegates to tcp/listen; :transport key (default :tcp)
(start-server handler opts) ;; delegates to tcp/start-server
(close x)                 ;; dispatches on :conns key: server → listen-close, else → close
```

### Rust

```rust
pub fn init(globals: &Arc<GlobalEnv>)
pub fn tcp::connect_to(host: &str, port: u16, in_buf: usize, out_buf: usize) -> Value
pub fn tcp::listen_on(host: &str, port: u16, conns_buf: usize, in_buf: usize, out_buf: usize) -> ValueResult<Value>
```

`init` registers both namespaces, calling `cljrs_async::init` internally. Idempotent.

### `TcpStreamResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks, dropping the socket halves and releasing the FD.

### `TcpListenerResource`

Implements `cljrs_value::Resource` (Arc-backed). Holds the `AbortHandle` for the `accept_loop` task. `close()` aborts the task, dropping the `TcpListener` and releasing the listener FD.

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
