//! QUIC client transport for `clojure.rust.net.quic`.
//!
//! Phase Q1 delivers the client side: `connect`, `open-stream`, and `close`.
//! A connection is a map:
//!
//! ```clojure
//! {:streams     <chan>     ; yields stream maps for peer-initiated bidi streams
//!  :remote-addr "ip:port"
//!  :local-addr  "ip:port"
//!  :resource    <handle>} ; QuicConnectionResource — deterministic close
//! ```
//!
//! A stream map is:
//!
//! ```clojure
//! {:in        <chan>    ; byte-array chunks; closed at stream FIN
//!  :out       <chan>    ; put byte-arrays/strings; close! sends FIN
//!  :stream-id <long>   ; QUIC stream index
//!  :resource  <handle>} ; QuicStreamResource
//! ```
//!
//! `connect` returns a capacity-1 promise channel; `open-stream` also returns a
//! promise channel. The QUIC handshake and stream operations run on the
//! `WorkerPool`; `GcPtr`/`Value` construction and channel bridges run on the
//! `LocalSet`.
//!
//! QUIC streams (`quinn::SendStream` / `quinn::RecvStream`) implement
//! `tokio::io::AsyncWrite` / `AsyncRead`, so they feed directly into
//! `pool_writer` / `pool_reader` from `pool_io.rs` with no structural change.

use std::any::Any;
use std::sync::{Arc, Mutex};

use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, make_chan};
use cljrs_async::eval_async::spawn_future;
use cljrs_async::worker_pool::WorkerPool;
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, Keyword, MapValue, NativeFn, NativeObjectBox, Resource, ResourceHandle, Value,
    ValueError, ValueResult,
};

use crate::pool_io::{
    PoolStreamSetup, net_error, pool_reader, pool_writer, read_bridge, write_bridge,
};

// ── Public entry point ─────────────────────────────────────────────────────────

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        ("connect", Arity::Fixed(1), builtin_connect),
        ("open-stream", Arity::Variadic { min: 1 }, builtin_open_stream),
        ("close", Arity::Fixed(1), builtin_close),
        ("stream-close", Arity::Fixed(1), builtin_stream_close),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── QuicConnectionResource ─────────────────────────────────────────────────────

#[derive(Debug)]
struct QuicConnectionInner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` for a QUIC connection.
///
/// Holds the `quinn::Connection` (to call `open_bi` and `close`) and the
/// `quinn::Endpoint` (kept alive so the driver doesn't shut down while the
/// connection is live). Abort handles cover the peer-stream accept loop and
/// the LocalSet `:streams` bridge.
#[derive(Debug)]
pub struct QuicConnectionResource {
    pub connection: quinn::Connection,
    _endpoint: quinn::Endpoint,
    inner: Arc<Mutex<QuicConnectionInner>>,
}

impl QuicConnectionResource {
    fn new(connection: quinn::Connection, endpoint: quinn::Endpoint) -> Self {
        Self {
            connection,
            _endpoint: endpoint,
            inner: Arc::new(Mutex::new(QuicConnectionInner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for QuicConnectionResource {
    fn close(&self) -> ValueResult<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Ok(());
        }
        g.closed = true;
        for h in g.abort_handles.drain(..) {
            h.abort();
        }
        self.connection
            .close(quinn::VarInt::from_u32(0), b"closed");
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn resource_type(&self) -> &'static str {
        "QuicConnection"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── QuicStreamResource ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct QuicStreamInner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` for a QUIC stream.
///
/// Holds abort handles for the pool reader, pool writer, and LocalSet bridge
/// tasks (4 total). Aborting them drops the `SendStream`/`RecvStream`, which
/// quinn interprets as a `RESET_STREAM` / `STOP_SENDING` to the peer.
#[derive(Debug)]
pub struct QuicStreamResource {
    inner: Arc<Mutex<QuicStreamInner>>,
}

impl QuicStreamResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(QuicStreamInner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for QuicStreamResource {
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
        "QuicStream"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Internal message types ─────────────────────────────────────────────────────

/// A `PoolStreamSetup` augmented with the QUIC stream index.
struct QuicStreamSetup {
    stream_id: u64,
    setup: PoolStreamSetup,
}

type QuicStreamResult = Result<QuicStreamSetup, String>;

// ── Value helpers ──────────────────────────────────────────────────────────────

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

// ── Options-map parsing ────────────────────────────────────────────────────────

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

// ── Pool tasks (Send, no GcPtr) ────────────────────────────────────────────────

/// Runs on the pool: loops on `connection.accept_bi()` and, for each
/// peer-initiated bidirectional stream, spawns pool_reader + pool_writer and
/// sends a `QuicStreamResult` to the LocalSet bridge.
async fn pool_stream_accept_loop(
    connection: quinn::Connection,
    stream_tx: mpsc::Sender<QuicStreamResult>,
    in_buf: usize,
    out_buf: usize,
) {
    loop {
        match connection.accept_bi().await {
            Err(e) => {
                let _ = stream_tx
                    .send(Err(format!("accept_bi: {e}")))
                    .await;
                break;
            }
            Ok((send, recv)) => {
                let stream_id = send.id().index();
                let (read_tx, read_rx) =
                    mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
                let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(out_buf.max(1));

                let reader_jh = tokio::spawn(pool_reader(recv, read_tx));
                let writer_jh = tokio::spawn(pool_writer(send, write_rx));

                let msg = QuicStreamSetup {
                    stream_id,
                    setup: PoolStreamSetup {
                        remote_addr: String::new(),
                        local_addr: String::new(),
                        read_rx,
                        write_tx,
                        reader_abort: reader_jh.abort_handle(),
                        writer_abort: writer_jh.abort_handle(),
                    },
                };

                if stream_tx.send(Ok(msg)).await.is_err() {
                    // LocalSet bridge dropped; abort the just-spawned tasks.
                    break;
                }
            }
        }
    }
}

/// Runs on the pool: opens a single bidirectional stream on the connection,
/// spawns pool_reader + pool_writer, and sends the result via oneshot.
async fn pool_open_stream(
    connection: quinn::Connection,
    in_buf: usize,
    out_buf: usize,
    stream_tx: oneshot::Sender<QuicStreamResult>,
) {
    match connection.open_bi().await {
        Err(e) => {
            let _ = stream_tx.send(Err(format!("open_bi: {e}")));
        }
        Ok((send, recv)) => {
            let stream_id = send.id().index();
            let (read_tx, read_rx) =
                mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
            let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(out_buf.max(1));

            let reader_jh = tokio::spawn(pool_reader(recv, read_tx));
            let writer_jh = tokio::spawn(pool_writer(send, write_rx));

            let _ = stream_tx.send(Ok(QuicStreamSetup {
                stream_id,
                setup: PoolStreamSetup {
                    remote_addr: String::new(),
                    local_addr: String::new(),
                    read_rx,
                    write_tx,
                    reader_abort: reader_jh.abort_handle(),
                    writer_abort: writer_jh.abort_handle(),
                },
            }));
        }
    }
}

// ── LocalSet bridges (touch GcPtr / Value) ────────────────────────────────────

/// Runs on the LocalSet: receives `QuicStreamResult` from `pool_stream_accept_loop`,
/// builds stream maps, and puts them on the `:streams` channel.
async fn local_stream_accept_bridge(
    mut stream_rx: mpsc::Receiver<QuicStreamResult>,
    streams_chan: GcPtr<NativeObjectBox>,
    in_buf: usize,
    out_buf: usize,
) {
    while let Some(result) = stream_rx.recv().await {
        match result {
            Err(e) => {
                chan_put(&streams_chan, net_error(e)).await;
                break;
            }
            Ok(msg) => {
                let stream_val =
                    make_quic_stream_from_setup(msg.setup, msg.stream_id, in_buf, out_buf);
                if !chan_put(&streams_chan, stream_val).await {
                    break;
                }
            }
        }
    }
    chan_ref(streams_chan.get()).close();
}

/// Build a QUIC stream map from a `PoolStreamSetup`, spawning LocalSet bridge tasks.
/// Called on the LocalSet thread.
fn make_quic_stream_from_setup(
    setup: PoolStreamSetup,
    stream_id: u64,
    in_buf: usize,
    out_buf: usize,
) -> Value {
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);
    let resource = QuicStreamResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

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
        (kw("stream-id"), Value::Long(stream_id as i64)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]))
}

/// Build a QUIC connection map from an established `quinn::Connection`.
/// Spawns the peer-stream accept loop (pool) and its LocalSet bridge.
fn make_quic_connection(
    connection: quinn::Connection,
    endpoint: quinn::Endpoint,
    remote_addr: String,
    local_addr: String,
    streams_buf: usize,
    in_buf: usize,
    out_buf: usize,
) -> Value {
    let streams_chan = make_chan(streams_buf);

    let resource = QuicConnectionResource::new(connection.clone(), endpoint);
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    let (stream_tx, stream_rx) = mpsc::channel::<QuicStreamResult>(streams_buf.max(1));

    let pool_jh = WorkerPool::global()
        .handle()
        .spawn(pool_stream_accept_loop(connection, stream_tx, in_buf, out_buf));

    let bridge_jh = tokio::task::spawn_local(local_stream_accept_bridge(
        stream_rx,
        streams_chan.clone(),
        in_buf,
        out_buf,
    ));

    {
        let mut g = shared_inner.lock().unwrap();
        g.abort_handles.push(pool_jh.abort_handle());
        g.abort_handles.push(bridge_jh.abort_handle());
    }

    Value::Map(MapValue::from_pairs(vec![
        (kw("streams"), Value::NativeObject(streams_chan)),
        (kw("remote-addr"), Value::string(remote_addr)),
        (kw("local-addr"), Value::string(local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]))
}

// ── Connect implementation ─────────────────────────────────────────────────────

/// Initiate a QUIC connection to `host:port` and return a promise channel.
///
/// The QUIC handshake runs on the `WorkerPool`. On success, delivers a
/// connection map to the promise channel on the LocalSet. On failure, delivers
/// a `Value::Error`.
pub fn connect_to(
    host: &str,
    port: u16,
    quinn_config: quinn::ClientConfig,
    streams_buf: usize,
    in_buf: usize,
    out_buf: usize,
) -> Value {
    let host = host.to_string();
    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());
    spawn_future(async move {
        do_quic_connect(host, port, quinn_config, streams_buf, in_buf, out_buf, promise).await
    });
    promise_val
}

async fn do_quic_connect(
    host: String,
    port: u16,
    quinn_config: quinn::ClientConfig,
    streams_buf: usize,
    in_buf: usize,
    out_buf: usize,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    type ConnResult = Result<(quinn::Connection, quinn::Endpoint, String, String), String>;
    let (conn_tx, conn_rx) = oneshot::channel::<ConnResult>();

    WorkerPool::global().handle().spawn(async move {
        let addr_str = format!("{host}:{port}");

        // Resolve host to a SocketAddr (handles both IPs and hostnames).
        let addr = match tokio::net::lookup_host(&addr_str).await {
            Ok(mut it) => match it.next() {
                Some(a) => a,
                None => {
                    let _ = conn_tx.send(Err(format!("no address for {addr_str}")));
                    return;
                }
            },
            Err(e) => {
                let _ = conn_tx.send(Err(format!("lookup {addr_str}: {e}")));
                return;
            }
        };

        let bind: std::net::SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };

        let mut endpoint = match quinn::Endpoint::client(bind) {
            Ok(e) => e,
            Err(e) => {
                let _ = conn_tx.send(Err(format!("endpoint: {e}")));
                return;
            }
        };
        endpoint.set_default_client_config(quinn_config);

        let connecting = match endpoint.connect(addr, &host) {
            Ok(c) => c,
            Err(e) => {
                let _ = conn_tx.send(Err(format!("connect: {e}")));
                return;
            }
        };

        let connection = match connecting.await {
            Ok(c) => c,
            Err(e) => {
                let _ = conn_tx.send(Err(format!("handshake: {e}")));
                return;
            }
        };

        let remote_addr = connection.remote_address().to_string();
        let local_addr = endpoint.local_addr().map(|a| a.to_string()).unwrap_or_default();
        let _ = conn_tx.send(Ok((connection, endpoint, remote_addr, local_addr)));
    });

    match conn_rx.await {
        Err(_) => chan_deliver(&promise, net_error("pool task dropped")).await,
        Ok(Err(e)) => chan_deliver(&promise, net_error(e)).await,
        Ok(Ok((connection, endpoint, remote_addr, local_addr))) => {
            let conn =
                make_quic_connection(connection, endpoint, remote_addr, local_addr, streams_buf, in_buf, out_buf);
            chan_deliver(&promise, conn).await;
        }
    }
    Ok(Value::Nil)
}

// ── Open-stream implementation ─────────────────────────────────────────────────

/// Open a new bidirectional stream on an existing connection.
///
/// Runs `open_bi()` on the `WorkerPool` and returns a promise channel that
/// yields a stream map on the LocalSet.
pub fn open_stream_on(
    connection: quinn::Connection,
    in_buf: usize,
    out_buf: usize,
) -> Value {
    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());
    spawn_future(async move {
        do_open_stream(connection, in_buf, out_buf, promise).await
    });
    promise_val
}

async fn do_open_stream(
    connection: quinn::Connection,
    in_buf: usize,
    out_buf: usize,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    let (stream_tx, stream_rx) = oneshot::channel::<QuicStreamResult>();

    WorkerPool::global()
        .handle()
        .spawn(pool_open_stream(connection, in_buf, out_buf, stream_tx));

    match stream_rx.await {
        Err(_) => chan_deliver(&promise, net_error("pool task dropped")).await,
        Ok(Err(e)) => chan_deliver(&promise, net_error(e)).await,
        Ok(Ok(msg)) => {
            let stream_val =
                make_quic_stream_from_setup(msg.setup, msg.stream_id, in_buf, out_buf);
            chan_deliver(&promise, stream_val).await;
        }
    }
    Ok(Value::Nil)
}

// ── Builtins ───────────────────────────────────────────────────────────────────

/// `(connect {:host h :port p :alpn [...] :insecure-skip-verify bool ...})`
/// Returns a promise channel yielding the connection map or a `Value::Error`.
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
    let streams_buf = opts_usize(&opts, "streams-buf").unwrap_or(8);
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);

    let config = crate::quic_config::client_config(&opts)?;
    Ok(connect_to(&host, port, config, streams_buf, in_buf, out_buf))
}

/// `(open-stream conn)` or `(open-stream conn {:in-buf N :out-buf N})`
/// Returns a promise channel yielding a stream map or a `Value::Error`.
fn builtin_open_stream(args: &[Value]) -> ValueResult<Value> {
    let conn_map = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "connection map {:resource handle}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    let (in_buf, out_buf) = match args.get(1) {
        Some(Value::Map(m)) => (
            opts_usize(m, "in-buf").unwrap_or(8),
            opts_usize(m, "out-buf").unwrap_or(8),
        ),
        _ => (8, 8),
    };

    let resource_handle = match conn_map.get(&kw("resource")) {
        Some(Value::Resource(h)) => h.clone(),
        _ => {
            return Err(ValueError::Other(
                "connection map missing :resource".into(),
            ));
        }
    };

    let conn_res = resource_handle
        .downcast::<QuicConnectionResource>()
        .ok_or_else(|| ValueError::Other("not a QuicConnectionResource".into()))?;

    let connection = conn_res.connection.clone();
    Ok(open_stream_on(connection, in_buf, out_buf))
}

/// `(close conn)` — close a QUIC connection, aborting all tasks and sending
/// a CONNECTION_CLOSE frame to the peer.
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

    if let Some(Value::NativeObject(obj)) = conn.get(&kw("streams")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::Resource(handle)) = conn.get(&kw("resource")) {
        let _ = handle.close();
    }

    Ok(Value::Nil)
}

/// `(stream-close stream)` — close a QUIC stream, sending FIN/RESET to the peer.
fn builtin_stream_close(args: &[Value]) -> ValueResult<Value> {
    let stream = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "stream map",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    if let Some(Value::NativeObject(obj)) = stream.get(&kw("out")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::NativeObject(obj)) = stream.get(&kw("in")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::Resource(handle)) = stream.get(&kw("resource")) {
        let _ = handle.close();
    }

    Ok(Value::Nil)
}
