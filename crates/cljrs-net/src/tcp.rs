//! TCP client/server support for `clojure.rust.net.tcp`.
//!
//! Phase A delivers `connect` and `close`. A connection is a map:
//!
//! ```clojure
//! {:in          <chan>   ; byte-array chunks from the peer; closed at EOF
//!  :out         <chan>   ; put byte-array/string values here to send
//!  :remote-addr "ip:port"
//!  :local-addr  "ip:port"
//!  :resource    <handle>} ; TcpStreamResource — deterministic socket close
//! ```
//!
//! `connect` returns a capacity-1 promise channel that yields the connection
//! map once the TCP handshake completes, or a `Value::Error` on failure.
//!
//! Phase B adds `listen` and `listen-close`. A server map is:
//!
//! ```clojure
//! {:conns      <chan>   ; yields a connection map for each accepted socket
//!  :local-addr "ip:port"
//!  :resource   <handle>} ; TcpListenerResource — deterministic listener close
//! ```

use std::any::Any;
use std::sync::{Arc, Mutex};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::task::AbortHandle;

use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, chan_take, make_chan};
use cljrs_async::spawn_future;
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, ExceptionInfo, Keyword, MapValue, NativeFn, NativeObjectBox, Resource, ResourceHandle,
    Value, ValueError, ValueResult,
};

// ── Public entry point ────────────────────────────────────────────────────────

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        ("connect", Arity::Fixed(1), builtin_connect),
        ("close", Arity::Fixed(1), builtin_close),
        ("listen", Arity::Fixed(1), builtin_listen),
        ("listen-close", Arity::Fixed(1), builtin_listen_close),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── TcpStreamResource ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct TcpStreamInner {
    closed: bool,
    reader_abort: Option<AbortHandle>,
    writer_abort: Option<AbortHandle>,
}

/// `Resource` implementation for a TCP stream.
///
/// Holds `AbortHandle`s for the reader and writer tasks spawned in `connect`.
/// `close()` aborts both tasks, which drops the socket halves and closes the FD.
/// GC never finalises the socket — this `Arc`-backed resource is the sole
/// cleanup path, matching the design note in `resource.rs`.
#[derive(Debug)]
pub struct TcpStreamResource {
    inner: Arc<Mutex<TcpStreamInner>>,
}

impl TcpStreamResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TcpStreamInner {
                closed: false,
                reader_abort: None,
                writer_abort: None,
            })),
        }
    }
}

impl Resource for TcpStreamResource {
    fn close(&self) -> ValueResult<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Ok(());
        }
        g.closed = true;
        if let Some(h) = g.reader_abort.take() {
            h.abort();
        }
        if let Some(h) = g.writer_abort.take() {
            h.abort();
        }
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn resource_type(&self) -> &'static str {
        "TcpStream"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── TcpListenerResource ───────────────────────────────────────────────────────

#[derive(Debug)]
struct TcpListenerInner {
    closed: bool,
    accept_abort: Option<AbortHandle>,
}

/// `Resource` implementation for a TCP listener.
///
/// Holds the `AbortHandle` for the `accept_loop` task. `close()` aborts the
/// task, which drops the `TcpListener` and closes the listener FD.
#[derive(Debug)]
pub struct TcpListenerResource {
    inner: Arc<Mutex<TcpListenerInner>>,
}

impl TcpListenerResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TcpListenerInner {
                closed: false,
                accept_abort: None,
            })),
        }
    }
}

impl Resource for TcpListenerResource {
    fn close(&self) -> ValueResult<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Ok(());
        }
        g.closed = true;
        if let Some(h) = g.accept_abort.take() {
            h.abort();
        }
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn resource_type(&self) -> &'static str {
        "TcpListener"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Value helpers ─────────────────────────────────────────────────────────────

fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

fn net_error(msg: impl Into<String>) -> Value {
    let msg = msg.into();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

// ── Options-map parsing ───────────────────────────────────────────────────────

fn opts_str(opts: &MapValue, key: &str) -> Option<String> {
    match opts.get(&kw(key))? {
        Value::Str(s) => Some(s.get().clone()),
        _ => None,
    }
}

fn opts_usize(opts: &MapValue, key: &str) -> Option<usize> {
    match opts.get(&kw(key))? {
        Value::Long(n) if n >= 0 => Some((n as usize).max(1)),
        _ => None,
    }
}

fn opts_port(opts: &MapValue) -> Option<u16> {
    match opts.get(&kw("port"))? {
        Value::Long(n) if n > 0 && n <= 65535 => Some(n as u16),
        _ => None,
    }
}

// ── Async tasks ───────────────────────────────────────────────────────────────

/// Read chunks from the socket and put them on `:in`.
///
/// Closes `:in` at EOF or on error (after putting the error value). Aborted
/// via `TcpStreamResource::close` if the user calls `(close conn)`.
async fn reader_loop(mut read_half: OwnedReadHalf, in_chan: GcPtr<NativeObjectBox>) {
    let mut buf = vec![0u8; 8192];
    loop {
        match read_half.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                if !chan_put(&in_chan, bytes_value(&buf[..n])).await {
                    break; // consumer closed :in
                }
            }
            Err(e) => {
                chan_put(&in_chan, net_error(format!("read error: {e}"))).await;
                break;
            }
        }
    }
    chan_ref(in_chan.get()).close();
}

/// Drain `:out` and write each value to the socket.
///
/// Accepts `byte-array` and `string` values. Calls `shutdown` on the write
/// half when `:out` is closed (TCP half-close: FIN without RST). Aborted via
/// `TcpStreamResource::close` if the user calls `(close conn)`.
async fn writer_loop(mut write_half: OwnedWriteHalf, out_chan: GcPtr<NativeObjectBox>) {
    // Nested async block so write errors propagate via `?` rather than
    // `if err { break }`, keeping each match arm free of collapsible ifs.
    let _: std::io::Result<()> = async {
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => {
                    // :out closed; gracefully half-close the write side.
                    let _ = write_half.shutdown().await;
                    break;
                }
                Value::ByteArray(arr) => {
                    let bytes: Vec<u8> =
                        arr.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                    write_half.write_all(&bytes).await?;
                }
                Value::Str(s) => {
                    write_half.write_all(s.get().as_bytes()).await?;
                }
                _ => {}
            }
        }
        Ok(())
    }
    .await;
}

// ── Connection builder ────────────────────────────────────────────────────────

fn make_connection(stream: TcpStream, in_buf: usize, out_buf: usize) -> Value {
    let remote_addr = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let local_addr = stream
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    let (read_half, write_half) = stream.into_split();
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);
    let resource = TcpStreamResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);
    let r_jh = tokio::task::spawn_local(reader_loop(read_half, in_chan.clone()));
    shared_inner.lock().unwrap().reader_abort = Some(r_jh.abort_handle());
    let w_jh = tokio::task::spawn_local(writer_loop(write_half, out_chan.clone()));
    shared_inner.lock().unwrap().writer_abort = Some(w_jh.abort_handle());
    Value::Map(MapValue::from_pairs(vec![
        (kw("in"), Value::NativeObject(in_chan)),
        (kw("out"), Value::NativeObject(out_chan)),
        (kw("remote-addr"), Value::string(remote_addr)),
        (kw("local-addr"), Value::string(local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]))
}

// ── Accept loop ───────────────────────────────────────────────────────────────

/// Accept connections from `listener` and put each connection map on `conns_chan`.
///
/// Exits when the listener returns an error (e.g. the FD is closed by aborting
/// this task) or when the consumer closes `conns_chan`. After exiting, closes
/// `conns_chan` so consumers see EOF.
async fn accept_loop(
    listener: tokio::net::TcpListener,
    conns_chan: GcPtr<NativeObjectBox>,
    in_buf: usize,
    out_buf: usize,
) {
    loop {
        match listener.accept().await {
            Err(e) => {
                chan_put(&conns_chan, net_error(format!("accept error: {e}"))).await;
                break;
            }
            Ok((stream, _)) => {
                let conn = make_connection(stream, in_buf, out_buf);
                if !chan_put(&conns_chan, conn).await {
                    break; // consumer closed :conns
                }
            }
        }
    }
    chan_ref(conns_chan.get()).close();
}

// ── Listen implementation ─────────────────────────────────────────────────────

/// Bind a TCP listener on `host:port` and return a server map.
///
/// Convenience wrapper used by tests and the Clojure `listen` builtin alike.
pub fn listen_on(
    host: &str,
    port: u16,
    conns_buf: usize,
    in_buf: usize,
    out_buf: usize,
) -> ValueResult<Value> {
    let addr = format!("{host}:{port}");

    // Bind synchronously (std), then convert to Tokio (requires runtime context).
    let std_listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| ValueError::Other(format!("listen on {addr}: {e}")))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| ValueError::Other(format!("set_nonblocking: {e}")))?;
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .map_err(|e| ValueError::Other(format!("from_std: {e}")))?;

    let local_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();

    let conns_chan = make_chan(conns_buf);

    let resource = TcpListenerResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    let jh = tokio::task::spawn_local(accept_loop(listener, conns_chan.clone(), in_buf, out_buf));
    shared_inner.lock().unwrap().accept_abort = Some(jh.abort_handle());

    Ok(Value::Map(MapValue::from_pairs(vec![
        (kw("conns"), Value::NativeObject(conns_chan)),
        (kw("local-addr"), Value::string(local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ])))
}

// ── Connect implementation ────────────────────────────────────────────────────

/// Initiate a TCP connection and return the promise channel as a `Value`.
///
/// Convenience wrapper used by tests and the Clojure `connect` builtin alike.
pub fn connect_to(host: &str, port: u16, in_buf: usize, out_buf: usize) -> Value {
    let host = host.to_string();
    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());
    spawn_future(async move { do_connect(host, port, in_buf, out_buf, promise).await });
    promise_val
}

async fn do_connect(
    host: String,
    port: u16,
    in_buf: usize,
    out_buf: usize,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    let addr = format!("{host}:{port}");
    let stream = match tokio::net::TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => {
            chan_deliver(&promise, net_error(format!("connect to {addr}: {e}"))).await;
            return Ok(Value::Nil);
        }
    };
    chan_deliver(&promise, make_connection(stream, in_buf, out_buf)).await;
    Ok(Value::Nil)
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(connect {:host h :port p})` — returns a capacity-1 promise channel that
/// yields the connection map once connected, or a `Value::Error` on failure.
/// Optional keys: `:in-buf` (default 8), `:out-buf` (default 8).
fn builtin_connect(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:host str :port long}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let host = opts_str(&opts, "host").ok_or_else(|| ValueError::Other(":host required".into()))?;
    let port =
        opts_port(&opts).ok_or_else(|| ValueError::Other(":port required (1-65535)".into()))?;
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);

    Ok(connect_to(&host, port, in_buf, out_buf))
}

/// `(close conn)` — close a connection map.
///
/// Closes both `:in` and `:out` channels and aborts the reader/writer tasks
/// via the connection's `:resource` handle, releasing the socket FD.
fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    let conn = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "connection map",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    // Signal the writer task; it will drain and shutdown the write half.
    if let Some(Value::NativeObject(obj)) = conn.get(&kw("out")) {
        chan_ref(obj.get()).close();
    }
    // Close :in so any pending takes complete.
    if let Some(Value::NativeObject(obj)) = conn.get(&kw("in")) {
        chan_ref(obj.get()).close();
    }
    // Abort tasks and release the FD deterministically.
    if let Some(Value::Resource(handle)) = conn.get(&kw("resource")) {
        let _ = handle.close();
    }

    Ok(Value::Nil)
}

/// `(listen {:port p})` — bind a TCP listener and return a server map.
///
/// The `:conns` channel yields a connection map for each accepted socket and
/// is closed when the listener closes. Optional keys: `:host` (default
/// `"0.0.0.0"`), `:conns-buf` (default 8), `:in-buf` (default 8), `:out-buf`
/// (default 8).
fn builtin_listen(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:port long}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let host = opts_str(&opts, "host").unwrap_or_else(|| "0.0.0.0".to_string());
    let port =
        opts_port(&opts).ok_or_else(|| ValueError::Other(":port required (1-65535)".into()))?;
    let conns_buf = opts_usize(&opts, "conns-buf").unwrap_or(8);
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);

    listen_on(&host, port, conns_buf, in_buf, out_buf)
}

/// `(listen-close server)` — stop the accept loop and close the `:conns` channel.
fn builtin_listen_close(args: &[Value]) -> ValueResult<Value> {
    let server = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "server map {:conns ch :resource handle}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    if let Some(Value::Resource(handle)) = server.get(&kw("resource")) {
        let _ = handle.close();
    }
    if let Some(Value::NativeObject(obj)) = server.get(&kw("conns")) {
        chan_ref(obj.get()).close();
    }

    Ok(Value::Nil)
}
