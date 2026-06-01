# Networking

The `clojure.rust.net` family of namespaces provides channel-oriented TCP, TLS,
Unix-domain, and UDP sockets. Every socket is modelled as a pair of
`clojure.core.async` channels — bytes arrive on `:in`, bytes leave through
`:out`. This is the same duplex-channel model used by libraries such as aleph on
the JVM, so protocols can be written as ordinary channel operations: `go` loops,
`take!`, `put!`, `alts`, and the [framing](net-framing.md) helpers.

The stack is provided by the `cljrs-net` crate and loaded automatically by the
`cljrs` CLI.

```clojure
(require '[clojure.rust.net :as net])
```

| Namespace | Contents |
|---|---|
| `clojure.rust.net.tcp` | Plain TCP client and server |
| `clojure.rust.net.tls` | TLS client and server (rustls) |
| `clojure.rust.net.unix` | Unix-domain stream sockets (`#[cfg(unix)]`) |
| `clojure.rust.net.udp` | UDP datagrams |
| `clojure.rust.net.frame` | Stateful framers and encode helpers |
| `clojure.rust.net` | Umbrella namespace — dispatches on `:transport` |

## Connection model

`connect` returns a capacity-1 promise channel (the discrete-op shape used
throughout clojurust I/O). Taking from it yields either a connection map or an
error:

```clojure
{:in          <chan>     ; byte-array chunks, closed at EOF/error
 :out         <chan>     ; put byte-arrays or strings here; close to half-close
 :remote-addr "ip:port"
 :local-addr  "ip:port"
 :resource    <handle>} ; call (net/close conn) to release the FD
```

**Half-close**: `(close! (:out conn))` sends a TCP FIN while leaving `:in`
open, so you can finish reading any in-flight data before the peer closes its
side. `(net/close conn)` tears down both halves immediately.

## Server model

`listen` binds synchronously and returns a server map immediately:

```clojure
{:conns      <chan>     ; yields a connection map per accepted socket; closed at shutdown
 :local-addr "ip:port"
 :resource   <handle>} ; call (net/close server) to stop accepting
```

Taking from `:conns` blocks until the next accepted connection or the listener
closes. Backpressure is propagated: when `:conns` is full the accept loop parks
until the application drains the channel.

`start-server` is sugar that spawns a `go-loop` accepting from `:conns` and
calling a handler for each connection:

```clojure
(net/start-server
  (fn [conn]
    (go (loop []
          (when-let [chunk (<! (:in conn))]
            (>! (:out conn) chunk)
            (recur)))
        (close! (:out conn))))
  {:port 8080})
```

## Umbrella dispatch

`clojure.rust.net` delegates every call to the right namespace based on the
`:transport` key (defaulting to `:tcp`):

```clojure
(net/connect {:host "example.com" :port 443 :transport :tls})
(net/connect {:path "/tmp/app.sock" :transport :unix})
(net/listen  {:port 8080})                   ; :tcp by default
```

The umbrella `close` inspects the map shape to decide whether the argument is a
server, a stream connection, or a UDP socket:

```clojure
(net/close conn)    ; :remote-addr present → stream connection
(net/close server)  ; :conns present → listener
(net/close udp)     ; everything else → UDP socket
```

## Lifecycle helpers

These are provided by `clojure.rust.net` and work with any connection or server:

```clojure
;; Ensure a connection is closed even if the body throws
(with-open [c (await (take! (net/connect opts)))]
  body...)

;; Separate a stream of values from the first error
(let [{:keys [out err]} (net/split-err (:in conn))]
  ;; out  — channel of non-error values
  ;; err  — promise of the first error, or nil at clean EOF
  )

;; Consume an entire channel, collecting results
(let [{:keys [values error]} (await (net/drain-to (:in conn)))]
  ...)
```

## Pool-based I/O (Phase A2)

TCP and TLS byte-level I/O runs on a `WorkerPool` multi-thread runtime. The
LocalSet executor — which owns all `GcPtr<Value>` — interacts with it through
lightweight bridge tasks that convert between Rust bytes and Clojure values.
This keeps the heap thread responsive under sustained byte traffic while
preserving the single-thread invariant that the garbage collector requires.

UDP and Unix sockets use a simpler single-task model; their I/O volume does not
justify the bridge overhead.

## TCP

```clojure
(require '[clojure.rust.net.tcp :as tcp])

;; Client
(let [conn (await (take! (tcp/connect {:host "example.com" :port 80})))]
  ...)

;; Server — manual accept loop
(let [server (tcp/listen {:port 8080})]
  (go (loop []
        (when-let [conn (<! (:conns server))]
          (handle conn)
          (recur)))))

;; Server — with sugar
(tcp/start-server handle-fn {:port 8080})
```

`connect` accepts `:in-buf` and `:out-buf` keyword options to set channel buffer
depths (default 8 each). `listen` additionally accepts `:host` (default
`"0.0.0.0"`) and `:conns-buf`.

## TLS

```clojure
(require '[clojure.rust.net.tls :as tls])

;; Client — uses WebPKI roots by default
(tls/connect {:host "example.com" :port 443})

;; Client — system roots or custom CA bundle
(tls/connect {:host "internal.example.com" :port 8443 :roots :system})
(tls/connect {:host "dev.local" :port 8443 :roots "ca.pem"})

;; Client — disable cert verification (testing only)
(tls/connect {:host "localhost" :port 8443 :insecure-skip-verify true})

;; Client — ALPN negotiation
(tls/connect {:host "example.com" :port 443 :alpn ["h2" "http/1.1"]})

;; Server — PEM cert and key required
(tls/listen {:port 8443 :cert "cert.pem" :key "key.pem"})
(tls/start-server handle-fn {:port 8443 :cert "cert.pem" :key "key.pem"})
```

The returned connection and server maps have the same shape as TCP.

## Unix-domain sockets

Unix-domain sockets are only available on Unix targets. On other platforms the
functions are registered but throw `"not supported on this platform"`.

```clojure
(require '[clojure.rust.net.unix :as unix])

;; Client
(unix/connect {:path "/tmp/app.sock"})

;; Server — automatically unlinks the path on close
(unix/listen {:path "/tmp/app.sock"})
(unix/start-server handle-fn {:path "/tmp/app.sock"})
```

`listen` pre-unlinks any stale socket file at the path before binding, so
restarting a server after a crash does not require manual cleanup. `close` also
unlinks the path.

## UDP

UDP sockets use a datagram map on both `:in` and `:out`:

```clojure
(require '[clojure.rust.net.udp :as udp])

(let [sock (udp/socket {:port 9000})]
  ;; Receive: {:data <byte-array> :addr "ip:port"}
  (go (loop []
        (when-let [pkt (<! (:in sock))]
          (println (:addr pkt) "->" (count (:data pkt)) "bytes")
          (recur))))

  ;; Send
  (>! (:out sock) {:data my-bytes :addr "10.0.0.1:9000"})

  (udp/close sock))
```

`socket` accepts `:host` (default `"0.0.0.0"`) and `:in-buf` / `:out-buf` channel buffer options.

## Framing

Raw TCP connections deliver bytes in arbitrary-sized chunks; protocols typically
need message boundaries. `clojure.rust.net.frame/frame` pipes a raw `:in`
channel through a stateful framer and returns a new channel of complete
messages:

```clojure
(require '[clojure.rust.net.frame :as frame])

;; Line-delimited protocol
(let [lines (frame/frame (:in conn) (frame/lines))]
  (go (loop []
        (when-let [line (<! lines)]
          (println line)
          (>! (:out conn) (frame/lines-encode line))
          (recur)))))

;; Length-prefixed protocol (4-byte big-endian header)
(let [msgs (frame/frame (:in conn) (frame/length-prefixed {:bytes 4}))]
  (go (loop []
        (when-let [msg (<! msgs)]
          (>! (:out conn) (frame/length-prefixed-encode msg {:bytes 4}))
          (recur)))))
```

### Framer specs

| Constructor | Output type | Notes |
|---|---|---|
| `(frame/lines)` | `string` per line | strips `\r`; emits partial final line at EOF |
| `(frame/by-delimiter b)` | `byte-array` per frame | delimiter byte excluded from output |
| `(frame/length-prefixed {:bytes n})` | `byte-array` per frame | N-byte header (big-endian by default); partial frames at EOF are discarded |

Pass `:endian :little` to `length-prefixed` for little-endian headers.

### Encode helpers

```clojure
(frame/lines-encode str)                     ; => byte-array (UTF-8 + \n)
(frame/length-prefixed-encode ba {:bytes 4}) ; => byte-array (4-byte header prepended)
```

### Async map over a framed channel

`pipe-map` covers the common case of applying a function to every message on a
channel:

```clojure
(let [msgs   (frame/frame (:in conn) (frame/lines))
      parsed (frame/pipe-map msgs parse-json)]
  ;; parsed is a channel of parse-json results
  )
```

## Embedding from Rust

`cljrs_net::init` registers all namespaces. It is idempotent and calls
`cljrs_async::init` internally, so you do not need to call it separately:

```rust
rt.block_on(local.run_until(async {
    let globals = cljrs_stdlib::standard_env();
    cljrs_net::init(&globals);
    // ... evaluate code ...
}));
```

Lower-level functions are also callable directly from Rust if you need to
open sockets outside Clojure code:

```rust
use cljrs_net::{tcp, tls, udp, frame};

tcp::connect_to("example.com", 80, 8, 8);
tcp::listen_on("0.0.0.0", 8080, 16, 8, 8)?;
udp::socket_on("0.0.0.0", 9000, 8, 8)?;
tls::tls_connect_to("example.com", 443, client_cfg, 8, 8);
tls::tls_listen_on("0.0.0.0", 8443, server_cfg, 16, 8, 8)?;
frame::frame_channel(in_chan, FramerSpec::Lines, 8);
```
