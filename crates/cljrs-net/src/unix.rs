//! Unix-domain stream socket support for `clojure.rust.net.unix`.
//!
//! `#[cfg(unix)]` — full implementation on Unix targets (Linux, macOS, etc.).
//! On non-Unix targets every registered function returns a clear
//! "not supported on this platform" error.
//!
//! Connection shape (identical to TCP):
//! ```clojure
//! {:in          <chan>   ; byte-array chunks; closed at EOF
//!  :out         <chan>   ; put byte-array/string values here
//!  :remote-addr "..."   ; peer socket path, or empty for unnamed peers
//!  :local-addr  "..."   ; local socket path, or empty for unnamed sockets
//!  :resource    <handle>}  ; UnixStreamResource — deterministic close
//! ```
//!
//! Server shape (identical to TCP):
//! ```clojure
//! {:conns      <chan>     ; yields a connection map per accepted socket
//!  :local-addr "..."     ; socket file path
//!  :resource   <handle>} ; UnixListenerResource — close() unlinks the path
//! ```

use std::sync::Arc;

use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{Arity, NativeFn, Value, ValueError, ValueResult};

type Builtin = fn(&[Value]) -> ValueResult<Value>;

// ── Public entry point ────────────────────────────────────────────────────────

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

// ── Unix target ───────────────────────────────────────────────────────────────

#[cfg(unix)]
use std::{any::Any, sync::Mutex};

#[cfg(unix)]
use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, chan_take, make_chan};
#[cfg(unix)]
use cljrs_async::spawn_future;
#[cfg(unix)]
use cljrs_env::error::EvalResult;
#[cfg(unix)]
use cljrs_value::{ExceptionInfo, Keyword, MapValue, NativeObjectBox, Resource, ResourceHandle};
#[cfg(unix)]
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[cfg(unix)]
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
#[cfg(unix)]
use tokio::task::AbortHandle;

// ── Value helpers (unix only) ─────────────────────────────────────────────────

#[cfg(unix)]
fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

#[cfg(unix)]
fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

#[cfg(unix)]
fn net_error(msg: impl Into<String>) -> Value {
    let msg = msg.into();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

#[cfg(unix)]
fn opts_str(opts: &MapValue, key: &str) -> Option<String> {
    match opts.get(&kw(key))? {
        Value::Str(s) => Some(s.get().clone()),
        _ => None,
    }
}

#[cfg(unix)]
fn opts_usize(opts: &MapValue, key: &str) -> Option<usize> {
    match opts.get(&kw(key))? {
        Value::Long(n) if n >= 0 => Some((n as usize).max(1)),
        _ => None,
    }
}

// ── UnixStreamResource ────────────────────────────────────────────────────────

#[cfg(unix)]
#[derive(Debug)]
struct UnixStreamInner {
    closed: bool,
    reader_abort: Option<AbortHandle>,
    writer_abort: Option<AbortHandle>,
}

/// `Resource` for a Unix-domain stream connection.
///
/// Holds `AbortHandle`s for the reader and writer tasks. `close()` aborts both,
/// which drops the socket halves and releases the FD. The GC never finalises the
/// socket — this Arc-backed resource is the sole cleanup path.
#[cfg(unix)]
#[derive(Debug)]
pub struct UnixStreamResource {
    inner: Arc<Mutex<UnixStreamInner>>,
}

#[cfg(unix)]
impl UnixStreamResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(UnixStreamInner {
                closed: false,
                reader_abort: None,
                writer_abort: None,
            })),
        }
    }
}

#[cfg(unix)]
impl Resource for UnixStreamResource {
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
        "UnixStream"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── UnixListenerResource ──────────────────────────────────────────────────────

#[cfg(unix)]
#[derive(Debug)]
struct UnixListenerInner {
    closed: bool,
    accept_abort: Option<AbortHandle>,
    path: std::path::PathBuf,
}

/// `Resource` for a Unix-domain socket listener.
///
/// Holds the `AbortHandle` for the accept-loop task. `close()` aborts the task
/// and **unlinks the socket path** from the filesystem, so a subsequent `listen`
/// on the same path succeeds without EADDRINUSE.
#[cfg(unix)]
#[derive(Debug)]
pub struct UnixListenerResource {
    inner: Arc<Mutex<UnixListenerInner>>,
}

#[cfg(unix)]
impl UnixListenerResource {
    fn new(path: std::path::PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(UnixListenerInner {
                closed: false,
                accept_abort: None,
                path,
            })),
        }
    }
}

#[cfg(unix)]
impl Resource for UnixListenerResource {
    fn close(&self) -> ValueResult<()> {
        let mut g = self.inner.lock().unwrap();
        if g.closed {
            return Ok(());
        }
        g.closed = true;
        if let Some(h) = g.accept_abort.take() {
            h.abort();
        }
        // Unlink the socket file; ignore error if already gone.
        let _ = std::fs::remove_file(&g.path);
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn resource_type(&self) -> &'static str {
        "UnixListener"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Async tasks ───────────────────────────────────────────────────────────────

/// Read chunks from the socket and put them on `:in`.
///
/// Closes `:in` at EOF or on error (after putting the error value). Aborted via
/// `UnixStreamResource::close` when the user calls `(close conn)`.
#[cfg(unix)]
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
                chan_put(&in_chan, net_error(format!("unix read error: {e}"))).await;
                break;
            }
        }
    }
    chan_ref(in_chan.get()).close();
}

/// Drain `:out` and write each value to the socket.
///
/// Accepts `byte-array` and `string` values. Calls `shutdown` on the write half
/// when `:out` is closed (half-close: FIN without RST). Aborted via
/// `UnixStreamResource::close` when the user calls `(close conn)`.
#[cfg(unix)]
async fn writer_loop(mut write_half: OwnedWriteHalf, out_chan: GcPtr<NativeObjectBox>) {
    let _: std::io::Result<()> = async {
        loop {
            match chan_take(&out_chan).await {
                Value::Nil => {
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

#[cfg(unix)]
fn make_connection(stream: tokio::net::UnixStream, in_buf: usize, out_buf: usize) -> Value {
    let remote_addr = stream
        .peer_addr()
        .ok()
        .and_then(|a| a.as_pathname().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let local_addr = stream
        .local_addr()
        .ok()
        .and_then(|a| a.as_pathname().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_default();
    let (read_half, write_half) = stream.into_split();
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);
    let resource = UnixStreamResource::new();
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
/// Exits on accept error or when the consumer closes `conns_chan`. Closes
/// `conns_chan` after exiting so consumers see EOF.
#[cfg(unix)]
async fn accept_loop(
    listener: tokio::net::UnixListener,
    conns_chan: GcPtr<NativeObjectBox>,
    in_buf: usize,
    out_buf: usize,
) {
    loop {
        match listener.accept().await {
            Err(e) => {
                chan_put(&conns_chan, net_error(format!("unix accept error: {e}"))).await;
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

// ── Public Rust API ───────────────────────────────────────────────────────────

/// Connect to the Unix-domain socket at `path`. Returns a capacity-1 promise
/// channel that yields the connection map once connected, or a `Value::Error`
/// on failure.
#[cfg(unix)]
pub fn connect_to(path: &str, in_buf: usize, out_buf: usize) -> Value {
    let path = path.to_string();
    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());
    spawn_future(async move { do_connect(path, in_buf, out_buf, promise).await });
    promise_val
}

#[cfg(unix)]
async fn do_connect(
    path: String,
    in_buf: usize,
    out_buf: usize,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    let stream = match tokio::net::UnixStream::connect(&path).await {
        Ok(s) => s,
        Err(e) => {
            chan_deliver(&promise, net_error(format!("unix connect to {path}: {e}"))).await;
            return Ok(Value::Nil);
        }
    };
    chan_deliver(&promise, make_connection(stream, in_buf, out_buf)).await;
    Ok(Value::Nil)
}

/// Bind a Unix listener at `path` and return a server map.
///
/// Removes any existing socket file at `path` before binding so that a
/// server restart after a crash does not get EADDRINUSE.
///
/// Convenience function used by tests and the Clojure `listen` builtin alike.
#[cfg(unix)]
pub fn listen_on(
    path: &str,
    conns_buf: usize,
    in_buf: usize,
    out_buf: usize,
) -> ValueResult<Value> {
    let path_buf = std::path::PathBuf::from(path);

    // Remove any stale socket file; ignore errors (file may not exist).
    let _ = std::fs::remove_file(&path_buf);

    // Bind synchronously (std), then wrap as tokio (requires runtime context).
    let std_listener = std::os::unix::net::UnixListener::bind(&path_buf)
        .map_err(|e| ValueError::Other(format!("unix listen on {path}: {e}")))?;
    std_listener
        .set_nonblocking(true)
        .map_err(|e| ValueError::Other(format!("set_nonblocking: {e}")))?;
    let listener = tokio::net::UnixListener::from_std(std_listener)
        .map_err(|e| ValueError::Other(format!("from_std: {e}")))?;

    let local_addr = listener
        .local_addr()
        .ok()
        .and_then(|a| a.as_pathname().map(|p| p.to_string_lossy().into_owned()))
        .unwrap_or_else(|| path.to_string());

    let conns_chan = make_chan(conns_buf);
    let resource = UnixListenerResource::new(path_buf);
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

// ── Builtins — unix ───────────────────────────────────────────────────────────

/// `(connect {:path "/tmp/app.sock"})` — returns a capacity-1 promise channel
/// that yields the connection map once connected, or a `Value::Error` on failure.
/// Optional keys: `:in-buf` (default 8), `:out-buf` (default 8).
#[cfg(unix)]
fn builtin_connect(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:path str}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let path = opts_str(&opts, "path").ok_or_else(|| ValueError::Other(":path required".into()))?;
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);
    Ok(connect_to(&path, in_buf, out_buf))
}

/// `(close conn)` — close a Unix-socket connection map.
#[cfg(unix)]
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
    if let Some(Value::NativeObject(obj)) = conn.get(&kw("out")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::NativeObject(obj)) = conn.get(&kw("in")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::Resource(handle)) = conn.get(&kw("resource")) {
        let _ = handle.close();
    }
    Ok(Value::Nil)
}

/// `(listen {:path "/tmp/app.sock"})` — bind a Unix listener and return a server map.
///
/// The `:conns` channel yields a connection map for each accepted socket and is
/// closed when the listener closes. Optional keys: `:conns-buf` (default 8),
/// `:in-buf` (default 8), `:out-buf` (default 8).
#[cfg(unix)]
fn builtin_listen(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:path str}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let path = opts_str(&opts, "path").ok_or_else(|| ValueError::Other(":path required".into()))?;
    let conns_buf = opts_usize(&opts, "conns-buf").unwrap_or(8);
    let in_buf = opts_usize(&opts, "in-buf").unwrap_or(8);
    let out_buf = opts_usize(&opts, "out-buf").unwrap_or(8);
    listen_on(&path, conns_buf, in_buf, out_buf)
}

/// `(listen-close server)` — stop the accept loop and close the `:conns` channel.
#[cfg(unix)]
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

// ── Builtins — non-unix stubs ─────────────────────────────────────────────────

#[cfg(not(unix))]
fn builtin_connect(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "clojure.rust.net.unix: Unix domain sockets are not supported on this platform".into(),
    ))
}

#[cfg(not(unix))]
fn builtin_close(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "clojure.rust.net.unix: Unix domain sockets are not supported on this platform".into(),
    ))
}

#[cfg(not(unix))]
fn builtin_listen(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "clojure.rust.net.unix: Unix domain sockets are not supported on this platform".into(),
    ))
}

#[cfg(not(unix))]
fn builtin_listen_close(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "clojure.rust.net.unix: Unix domain sockets are not supported on this platform".into(),
    ))
}
