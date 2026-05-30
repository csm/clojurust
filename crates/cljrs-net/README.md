# cljrs-net

**Purpose**: TCP/Unix/TLS networking for clojurust — channel-oriented sockets delivered as core.async channels.

**Status**: Phase A implemented (TCP client: `connect`, `close`). Phases B–F (server, framing, UDP, TLS, Unix) are stubs.

**Design**: Follows the aleph/netty-in-core.async model. A connection is a duplex pair of channels — `:in` carries `byte-array` chunks read from the socket, `:out` accepts `byte-array`/string values to write. Higher-level protocols are higher-order functions over those channels.

## File Layout

| File | Description |
|---|---|
| `src/lib.rs` | `init()` entry point; loads `clojure.rust.net.tcp` and `clojure.rust.net` |
| `src/tcp.rs` | `TcpStreamResource`, `connect`, `close`, reader/writer tasks |
| `src/clojure_rust_net_tcp.cljrs` | Clojure source for `clojure.rust.net.tcp` |
| `src/clojure_rust_net.cljrs` | Clojure source for umbrella `clojure.rust.net` |

## Public API

### `clojure.rust.net.tcp`

```clojure
(connect {:host "example.com" :port 80})
;; => promise-chan — yields {:in ch :out ch :remote-addr str :local-addr str :resource h}
;;    or Value::Error on failure

(close conn)  ;; => nil — closes :in/:out channels and aborts tasks
```

### `clojure.rust.net` (umbrella)

```clojure
(connect opts)  ;; delegates to tcp/connect; :transport key (default :tcp)
(close conn)    ;; delegates to tcp/close
```

### Rust

```rust
pub fn init(globals: &Arc<GlobalEnv>)
```

Registers `clojure.rust.net.tcp` and `clojure.rust.net`. Calls `cljrs_async::init` internally. Idempotent.

### `TcpStreamResource`

Implements `cljrs_value::Resource` (Arc-backed, `Send + Sync`). Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both tasks, dropping the `OwnedReadHalf`/`OwnedWriteHalf` and releasing the file descriptor.

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

## Usage Example

```clojure
(require '[clojure.core.async :refer [<!! >!! close!]])
(require '[clojure.rust.net.tcp :as tcp])

(let [conn (<!! (tcp/connect {:host "example.com" :port 80}))]
  (>!! (:out conn) (.getBytes "GET / HTTP/1.0\r\nHost: example.com\r\n\r\n"))
  (close! (:out conn))
  (loop []
    (when-let [chunk (<!! (:in conn))]
      (print (String. (byte-array (map #(bit-and % 0xff) chunk))))
      (recur)))
  (tcp/close conn))
```
