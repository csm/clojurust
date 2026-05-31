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
//!
//! Phase A2: TCP connect and accept now run on the `WorkerPool` multi-thread
//! runtime, so byte-level I/O never blocks the heap (`LocalSet`) thread.
//! `GcPtr`/`Value` construction happens in LocalSet bridge tasks only.

use std::any::Any;
use std::sync::{Arc, Mutex};

use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, make_chan};
use cljrs_async::spawn_future;
use cljrs_async::worker_pool::WorkerPool;
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, Keyword, MapValue, NativeFn, NativeObjectBox, Resource, ResourceHandle, Value,
    ValueError, ValueResult,
};

use crate::pool_io::{
    net_error, pool_reader, pool_writer, read_bridge, write_bridge, PoolSetupResult,
    PoolStreamSetup,
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
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` implementation for a TCP stream.
///
/// Holds `AbortHandle`s for the pool reader, pool writer, and LocalSet bridge
/// tasks. `close()` aborts all handles, which drops the pool socket halves and
/// closes the FD. GC never finalises the socket — this `Arc`-backed resource is
/// the sole cleanup path.
#[derive(Debug)]
pub struct TcpStreamResource {
    inner: Arc<Mutex<TcpStreamInner>>,
}

impl TcpStreamResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TcpStreamInner {
                closed: false,
                abort_handles: Vec::new(),
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
        for h in g.abort_handles.drain(..) {
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
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` implementation for a TCP listener.
///
/// Holds `AbortHandle`s for the pool accept loop and the LocalSet accept bridge.
/// `close()` aborts all handles, which drops the listener and closes the FD.
#[derive(Debug)]
pub struct TcpListenerResource {
    inner: Arc<Mutex<TcpListenerInner>>,
}

impl TcpListenerResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TcpListenerInner {
                closed: false,
                abort_handles: Vec::new(),
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
        for h in g.abort_handles.drain(..) {
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

// ── Pool tasks (Send, no GcPtr) ───────────────────────────────────────────────

/// Runs on the pool: accepts connections from `listener`, spawns pool_reader +
/// pool_writer for each, and sends the `PoolStreamSetup` via `conn_info_tx`.
/// Exits when the listener FD is closed or an accept error occurs.
async fn pool_accept_loop(
    std_listener: std::net::TcpListener,
    conn_info_tx: mpsc::Sender<PoolSetupResult>,
    in_buf: usize,
    out_buf: usize,
) {
    // Convert the std listener inside the pool runtime context.
    let listener = match TcpListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            let _ = conn_info_tx
                .send(Err(format!("from_std: {e}")))
                .await;
            return;
        }
    };

    loop {
        match listener.accept().await {
            Err(e) => {
                let _ = conn_info_tx
                    .send(Err(format!("accept error: {e}")))
                    .await;
                break;
            }
            Ok((stream, _peer)) => {
                let remote_addr = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                let local_addr = stream
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let (read_half, write_half) = stream.into_split();

                let (read_tx, read_rx) =
                    mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
                let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(out_buf.max(1));

                let reader_jh = tokio::spawn(pool_reader(read_half, read_tx));
                let writer_jh = tokio::spawn(pool_writer(write_half, write_rx));

                let setup = PoolStreamSetup {
                    remote_addr,
                    local_addr,
                    read_rx,
                    write_tx,
                    reader_abort: reader_jh.abort_handle(),
                    writer_abort: writer_jh.abort_handle(),
                };

                if conn_info_tx.send(Ok(setup)).await.is_err() {
                    // LocalSet bridge dropped; abort the just-spawned tasks.
                    reader_jh.abort();
                    writer_jh.abort();
                    break;
                }
            }
        }
    }
}

// ── LocalSet bridge (touches GcPtr / Value) ───────────────────────────────────

/// Runs on the LocalSet: receives `PoolSetupResult` from the pool accept loop,
/// creates Clojure channels + bridges for each new connection, and puts the
/// connection map on `conns_chan`.
async fn local_accept_bridge(
    mut conn_info_rx: mpsc::Receiver<PoolSetupResult>,
    conns_chan: GcPtr<NativeObjectBox>,
    in_buf: usize,
    out_buf: usize,
) {
    while let Some(result) = conn_info_rx.recv().await {
        match result {
            Err(e) => {
                chan_put(&conns_chan, net_error(e)).await;
                break;
            }
            Ok(setup) => {
                let conn = make_tcp_connection_from_setup(setup, in_buf, out_buf);
                if !chan_put(&conns_chan, conn).await {
                    break; // consumer closed :conns
                }
            }
        }
    }
    chan_ref(conns_chan.get()).close();
}

/// Build a connection map + resource from a `PoolStreamSetup`, spawning LocalSet
/// bridge tasks. Called on the LocalSet thread.
fn make_tcp_connection_from_setup(setup: PoolStreamSetup, in_buf: usize, out_buf: usize) -> Value {
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);
    let resource = TcpStreamResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    // Bridge tasks: run on LocalSet, touch GcPtr.
    let rb_jh = tokio::task::spawn_local(read_bridge(setup.read_rx, in_chan.clone()));
    let wb_jh = tokio::task::spawn_local(write_bridge(out_chan.clone(), setup.write_tx));

    {
        let mut g = shared_inner.lock().unwrap();
        g.abort_handles.push(setup.reader_abort);
        g.abort_handles.push(setup.writer_abort);
        g.abort_handles.push(rb_jh.abort_handle());
        g.abort_handles.push(wb_jh.abort_handle());
    }

    Value::Map(MapValue::from_pairs(vec![
        (kw("in"), Value::NativeObject(in_chan)),
        (kw("out"), Value::NativeObject(out_chan)),
        (kw("remote-addr"), Value::string(setup.remote_addr)),
        (kw("local-addr"), Value::string(setup.local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]))
}

// ── Listen implementation ─────────────────────────────────────────────────────

/// Bind a TCP listener on `host:port` and return a server map.
///
/// The accept loop runs on the `WorkerPool`; a LocalSet bridge delivers
/// connection maps onto `conns_chan`. Convenience wrapper used by tests and the
/// Clojure `listen` builtin alike.
pub fn listen_on(
    host: &str,
    port: u16,
    conns_buf: usize,
    in_buf: usize,
    out_buf: usize,
) -> ValueResult<Value> {
    let addr = format!("{host}:{port}");

    // Bind synchronously (std) so we get the local_addr immediately.
    let std_listener = std::net::TcpListener::bind(&addr)
        .map_err(|e| ValueError::Other(format!("listen on {addr}: {e}")))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| ValueError::Other(format!("set_nonblocking: {e}")))?;

    let local_addr = std_listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();

    let (conn_info_tx, conn_info_rx) = mpsc::channel::<PoolSetupResult>(conns_buf.max(1));
    let conns_chan = make_chan(conns_buf);

    let resource = TcpListenerResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    // Spawn accept loop on pool (Send).
    let pool_jh = WorkerPool::global()
        .handle()
        .spawn(pool_accept_loop(std_listener, conn_info_tx, in_buf, out_buf));

    // Spawn bridge on LocalSet (!Send, touches GcPtr).
    let bridge_jh = tokio::task::spawn_local(local_accept_bridge(
        conn_info_rx,
        conns_chan.clone(),
        in_buf,
        out_buf,
    ));

    {
        let mut g = shared_inner.lock().unwrap();
        g.abort_handles.push(pool_jh.abort_handle());
        g.abort_handles.push(bridge_jh.abort_handle());
    }

    Ok(Value::Map(MapValue::from_pairs(vec![
        (kw("conns"), Value::NativeObject(conns_chan)),
        (kw("local-addr"), Value::string(local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ])))
}

// ── Connect implementation ────────────────────────────────────────────────────

/// Initiate a TCP connection and return the promise channel as a `Value`.
///
/// The TCP connect and I/O tasks run on the `WorkerPool`; bridge tasks run on
/// the LocalSet. Convenience wrapper used by tests and the Clojure `connect`
/// builtin alike.
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

    // Channel for the pool → LocalSet handshake.
    let (setup_tx, setup_rx) = oneshot::channel::<PoolSetupResult>();

    // Spawn the entire TCP connect + I/O pump on the pool (Send).
    WorkerPool::global().handle().spawn(async move {
        match TcpStream::connect(&addr).await {
            Err(e) => {
                let _ = setup_tx.send(Err(format!("connect to {addr}: {e}")));
            }
            Ok(stream) => {
                let remote_addr = stream
                    .peer_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                let local_addr = stream
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_default();

                let (read_half, write_half) = stream.into_split();
                let (read_tx, read_rx) =
                    mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
                let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(out_buf.max(1));

                let reader_jh = tokio::spawn(pool_reader(read_half, read_tx));
                let writer_jh = tokio::spawn(pool_writer(write_half, write_rx));

                let _ = setup_tx.send(Ok(PoolStreamSetup {
                    remote_addr,
                    local_addr,
                    read_rx,
                    write_tx,
                    reader_abort: reader_jh.abort_handle(),
                    writer_abort: writer_jh.abort_handle(),
                }));
            }
        }
    });

    // Await the setup result back on the LocalSet (not blocking the thread).
    match setup_rx.await {
        Err(_) => {
            chan_deliver(&promise, net_error("pool task dropped")).await;
        }
        Ok(Err(e)) => {
            chan_deliver(&promise, net_error(e)).await;
        }
        Ok(Ok(setup)) => {
            let conn = make_tcp_connection_from_setup(setup, in_buf, out_buf);
            chan_deliver(&promise, conn).await;
        }
    }
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
/// Closes both `:in` and `:out` channels and aborts all I/O tasks via the
/// connection's `:resource` handle, releasing the socket FD.
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

    // Signal the writer task; it will drain and shutdown the write side.
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
