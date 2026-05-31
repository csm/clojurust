//! UDP datagram socket support for `clojure.rust.net.udp`.
//!
//! A UDP socket is a map:
//!
//! ```clojure
//! {:in          <chan>   ; yields {:data <byte-array> :addr "ip:port"}
//!  :out         <chan>   ; put {:data <byte-array> :addr "ip:port"}
//!  :local-addr  "ip:port"
//!  :resource    <handle>} ; UdpSocketResource — deterministic socket close
//! ```
//!
//! `socket` returns the socket map immediately (bind is synchronous). Reader
//! and writer tasks are spawned on the `LocalSet`.

use std::any::Any;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use tokio::net::UdpSocket;
use tokio::task::AbortHandle;

use cljrs_async::channel::{chan_put, chan_ref, chan_take, make_chan};
use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, ExceptionInfo, Keyword, MapValue, NativeFn, NativeObjectBox, Resource, ResourceHandle,
    Value, ValueError, ValueResult,
};

// ── Public entry point ────────────────────────────────────────────────────────

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        ("socket", Arity::Fixed(1), builtin_socket),
        ("close", Arity::Fixed(1), builtin_close),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── UdpSocketResource ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct UdpSocketInner {
    closed: bool,
    reader_abort: Option<AbortHandle>,
    writer_abort: Option<AbortHandle>,
}

/// `Resource` implementation for a UDP socket.
///
/// Holds `AbortHandle`s for the reader and writer tasks spawned in `socket_on`.
/// `close()` aborts both tasks, which drops the `Arc<UdpSocket>` they hold and
/// releases the FD once both tasks are gone. GC never finalises the socket —
/// this `Arc`-backed resource is the sole cleanup path.
#[derive(Debug)]
pub struct UdpSocketResource {
    inner: Arc<Mutex<UdpSocketInner>>,
}

impl UdpSocketResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(UdpSocketInner {
                closed: false,
                reader_abort: None,
                writer_abort: None,
            })),
        }
    }
}

impl Resource for UdpSocketResource {
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
        "UdpSocket"
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

fn make_datagram(bytes: &[u8], addr: &str) -> Value {
    Value::Map(MapValue::from_pairs(vec![
        (kw("data"), bytes_value(bytes)),
        (kw("addr"), Value::string(addr.to_string())),
    ]))
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

/// Read datagrams from the socket and put `{:data :addr}` maps onto `:in`.
///
/// Closes `:in` on error (after putting the error value) or when aborted
/// via `UdpSocketResource::close`.
async fn reader_loop(socket: Arc<UdpSocket>, in_chan: GcPtr<NativeObjectBox>) {
    let mut buf = vec![0u8; 65536];
    loop {
        match socket.recv_from(&mut buf).await {
            Ok((n, peer_addr)) => {
                let datagram = make_datagram(&buf[..n], &peer_addr.to_string());
                if !chan_put(&in_chan, datagram).await {
                    break; // consumer closed :in
                }
            }
            Err(e) => {
                chan_put(&in_chan, net_error(format!("recv_from error: {e}"))).await;
                break;
            }
        }
    }
    chan_ref(in_chan.get()).close();
}

/// Drain `:out` and send each `{:data :addr}` map as a datagram.
///
/// Exits when `:out` closes (channel yields `nil`). Aborted via
/// `UdpSocketResource::close` if the user calls `(close sock)`.
async fn writer_loop(socket: Arc<UdpSocket>, out_chan: GcPtr<NativeObjectBox>) {
    loop {
        match chan_take(&out_chan).await {
            Value::Nil => break,
            Value::Map(m) => {
                let data = m.get(&kw("data"));
                let addr = m.get(&kw("addr"));
                if let (Some(Value::ByteArray(arr)), Some(Value::Str(addr_str))) = (data, addr) {
                    let bytes: Vec<u8> =
                        arr.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                    let addr_str = addr_str.get().clone();
                    if let Ok(addr) = addr_str.parse::<SocketAddr>() {
                        let _ = socket.send_to(&bytes, addr).await;
                    }
                }
            }
            _ => {}
        }
    }
}

// ── Socket builder ────────────────────────────────────────────────────────────

/// Bind a UDP socket on `host:port` and return a socket map.
///
/// Convenience wrapper used by tests and the Clojure `socket` builtin alike.
pub fn socket_on(host: &str, port: u16, in_buf: usize, out_buf: usize) -> ValueResult<Value> {
    let addr = format!("{host}:{port}");

    // Bind synchronously (std), then convert to Tokio (requires runtime context).
    let std_socket = std::net::UdpSocket::bind(&addr)
        .map_err(|e| ValueError::Other(format!("bind {addr}: {e}")))?;
    std_socket
        .set_nonblocking(true)
        .map_err(|e| ValueError::Other(format!("set_nonblocking: {e}")))?;
    let socket = tokio::net::UdpSocket::from_std(std_socket)
        .map_err(|e| ValueError::Other(format!("from_std: {e}")))?;

    let local_addr = socket
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();

    let socket = Arc::new(socket);
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);

    let resource = UdpSocketResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    let r_jh = tokio::task::spawn_local(reader_loop(socket.clone(), in_chan.clone()));
    shared_inner.lock().unwrap().reader_abort = Some(r_jh.abort_handle());
    let w_jh = tokio::task::spawn_local(writer_loop(socket, out_chan.clone()));
    shared_inner.lock().unwrap().writer_abort = Some(w_jh.abort_handle());

    Ok(Value::Map(MapValue::from_pairs(vec![
        (kw("in"), Value::NativeObject(in_chan)),
        (kw("out"), Value::NativeObject(out_chan)),
        (kw("local-addr"), Value::string(local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ])))
}

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(socket {:port p})` — bind a UDP socket and return a socket map.
///
/// `:in` yields `{:data <byte-array> :addr "ip:port"}` maps for each received
/// datagram. Put `{:data <byte-array> :addr "ip:port"}` maps on `:out` to send.
/// Optional keys: `:host` (default `"0.0.0.0"`), `:in-buf` (default 8),
/// `:out-buf` (default 8).
fn builtin_socket(args: &[Value]) -> ValueResult<Value> {
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
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);

    socket_on(&host, port, in_buf, out_buf)
}

/// `(close sock)` — close a UDP socket map, releasing the FD and aborting tasks.
fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    let sock = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "UDP socket map {:in ch :out ch :resource handle}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    if let Some(Value::NativeObject(obj)) = sock.get(&kw("in")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::NativeObject(obj)) = sock.get(&kw("out")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::Resource(handle)) = sock.get(&kw("resource")) {
        let _ = handle.close();
    }

    Ok(Value::Nil)
}
