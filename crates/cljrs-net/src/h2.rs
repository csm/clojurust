//! HTTP/2 client for `clojure.rust.net.h2`.
//!
//! Wraps the `h2` crate on top of a TLS (rustls) connection. For each request
//! a fresh TCP + TLS connection is established; the TLS/HTTP/2 handshake and
//! all HTTP/2 I/O run on the `WorkerPool`; the response body is streamed to a
//! `:body` channel via a `LocalSet` bridge task.
//!
//! Response map:
//! ```clojure
//! {:status  200
//!  :headers {"content-type" "text/plain" ...}  ; string keys, string values
//!  :body    <chan>    ; byte-array chunks; closed at EOF or on error
//!  :resource <H2Resource>}
//! ```
//!
//! `H2Resource` holds abort handles for the pool streaming task and LocalSet
//! bridge task. Call `(h2/close resp)` to terminate the connection before the
//! body is fully drained.
//!
//! ALPN: HTTP/2 requires the ALPN token `"h2"`. `:alpn ["h2"]` is injected
//! automatically when not set in opts.

use std::any::Any;
use std::sync::{Arc, Mutex};

use rustls::pki_types::ServerName;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio_rustls::TlsConnector;

use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, make_chan};
use cljrs_async::eval_async::spawn_future;
use cljrs_async::worker_pool::WorkerPool;
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, Keyword, MapValue, NativeFn, NativeObjectBox, PersistentVector, Resource,
    ResourceHandle, Value, ValueError, ValueResult,
};

use crate::pool_io::{bytes_value, net_error};

// ── Public entry point ──────────────────────────────────────────────────────────

type Builtin = fn(&[Value]) -> ValueResult<Value>;

pub fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, Builtin)> = vec![
        ("get", Arity::Variadic { min: 1 }, builtin_get),
        ("request", Arity::Variadic { min: 1 }, builtin_request),
        ("close", Arity::Fixed(1), builtin_close),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

// ── H2Resource ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct H2Inner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` for an HTTP/2 connection.
///
/// Holds abort handles for the WorkerPool streaming task (which drives the h2
/// connection and streams the response body) and the LocalSet body-bridge task.
/// `close()` aborts both tasks, which drops the TLS/TCP socket and releases
/// the FD. `resource_type` → `"H2Connection"`.
#[derive(Debug)]
pub struct H2Resource {
    inner: Arc<Mutex<H2Inner>>,
}

impl H2Resource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(H2Inner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for H2Resource {
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
        "H2Connection"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Internal types ──────────────────────────────────────────────────────────────

/// Partial response from the pool task — delivered as soon as response headers
/// arrive so the LocalSet can build the `:body` channel and response map before
/// the body is fully streamed.
struct H2ResponsePartial {
    status: u16,
    headers: Vec<(String, String)>,
}

// ── Value / opts helpers ────────────────────────────────────────────────────────

fn kw(name: &str) -> Value {
    Value::keyword(Keyword::simple(name))
}

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

/// Return a TLS `ClientConfig` with `"h2"` injected into ALPN when the caller
/// did not set `:alpn` explicitly.
fn h2_client_config(opts: &MapValue) -> ValueResult<Arc<rustls::ClientConfig>> {
    if opts.get(&kw("alpn")).is_some() {
        return crate::tls::build_client_config(opts);
    }
    let alpn_val = Value::Vector(GcPtr::new(PersistentVector::from_iter([Value::string(
        "h2",
    )])));
    let new_opts = opts.assoc(kw("alpn"), alpn_val);
    crate::tls::build_client_config(&new_opts)
}

// ── URL / header parsing ────────────────────────────────────────────────────────

fn parse_url(url: &str) -> Result<(String, u16, http::Uri), String> {
    let uri: http::Uri = url
        .parse()
        .map_err(|e| format!("invalid URL '{url}': {e}"))?;
    let host = uri
        .host()
        .ok_or_else(|| format!("URL '{url}' missing host"))?
        .to_string();
    let port = uri.port_u16().unwrap_or(443);
    Ok((host, port, uri))
}

fn extract_headers(req_map: &MapValue) -> Vec<(String, String)> {
    let h_map = match req_map.get(&kw("headers")) {
        Some(Value::Map(m)) => m.clone(),
        _ => return vec![],
    };
    h_map
        .iter()
        .filter_map(|(k, v)| {
            let ks = match k {
                Value::Str(s) => s.get().clone(),
                Value::Keyword(kw) => kw.get().full_name(),
                _ => return None,
            };
            let vs = match v {
                Value::Str(s) => s.get().clone(),
                _ => return None,
            };
            Some((ks, vs))
        })
        .collect()
}

// ── Pool task ───────────────────────────────────────────────────────────────────

/// Runs entirely on the `WorkerPool`:
///
/// 1. TCP connect.
/// 2. TLS handshake with ALPN `"h2"`.
/// 3. HTTP/2 handshake via `h2::client::handshake`.
/// 4. Spawn the h2 connection driver task.
/// 5. Build and send the HTTP request.
/// 6. Receive response headers.
/// 7. Send `H2ResponsePartial` via `response_tx` so the `LocalSet` can build
///    the response map while body streaming continues.
/// 8. Stream response body chunks to `body_tx`.
#[allow(clippy::too_many_arguments)]
async fn pool_h2_request(
    host: String,
    port: u16,
    tls_config: Arc<rustls::ClientConfig>,
    uri: http::Uri,
    method: http::Method,
    extra_headers: Vec<(String, String)>,
    body_tx: mpsc::Sender<Result<Vec<u8>, String>>,
    response_tx: oneshot::Sender<Result<H2ResponsePartial, String>>,
) {
    macro_rules! fail {
        ($msg:expr) => {{
            let _ = response_tx.send(Err($msg));
            return;
        }};
    }

    // ── 1. TCP connect ────────────────────────────────────────────────────────
    let addr = format!("{host}:{port}");
    let tcp_stream = match TcpStream::connect(&addr).await {
        Ok(s) => s,
        Err(e) => fail!(format!("connect to {addr}: {e}")),
    };

    // ── 2. TLS handshake ──────────────────────────────────────────────────────
    let server_name = match ServerName::try_from(host.as_str()) {
        Ok(n) => n.to_owned(),
        Err(e) => fail!(format!("invalid SNI hostname '{host}': {e}")),
    };
    let connector = TlsConnector::from(tls_config);
    let tls_stream = match connector.connect(server_name, tcp_stream).await {
        Ok(s) => s,
        Err(e) => fail!(format!("TLS handshake: {e}")),
    };

    // ── 3. HTTP/2 handshake ───────────────────────────────────────────────────
    let (mut send_request, conn) = match h2::client::handshake(tls_stream).await {
        Ok(pair) => pair,
        Err(e) => fail!(format!("h2 handshake: {e}")),
    };

    // Spawn the h2 connection driver so control frames are processed while
    // request/response I/O is in flight.
    let conn_jh = tokio::spawn(async move {
        let _ = conn.await;
    });
    let conn_abort = conn_jh.abort_handle();

    // ── 4. Build and send request ─────────────────────────────────────────────
    let mut req_builder = http::Request::builder().method(method).uri(uri);
    for (k, v) in &extra_headers {
        req_builder = req_builder.header(k.as_str(), v.as_str());
    }
    let req = match req_builder.body(()) {
        Ok(r) => r,
        Err(e) => {
            conn_abort.abort();
            fail!(format!("build request: {e}"));
        }
    };

    // `end_of_stream = true` → no request body (GET / HEAD).
    let (response_future, _send_stream) = match send_request.send_request(req, true) {
        Ok(pair) => pair,
        Err(e) => {
            conn_abort.abort();
            fail!(format!("send request: {e}"));
        }
    };

    // ── 5. Receive response headers ───────────────────────────────────────────
    let response = match response_future.await {
        Ok(r) => r,
        Err(e) => {
            conn_abort.abort();
            fail!(format!("recv response: {e}"));
        }
    };

    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
        .collect();

    // ── 6. Deliver headers to LocalSet ────────────────────────────────────────
    if response_tx
        .send(Ok(H2ResponsePartial { status, headers }))
        .is_err()
    {
        // LocalSet gave up before headers arrived; clean up.
        conn_abort.abort();
        return;
    }

    // ── 7. Stream response body ───────────────────────────────────────────────
    let mut body = response.into_body();
    loop {
        match body.data().await {
            None => break,
            Some(Ok(chunk)) => {
                let n = chunk.len();
                let bytes: Vec<u8> = chunk.to_vec();
                // Release the flow-control window so the server keeps sending.
                let _ = body.flow_control().release_capacity(n);
                if !bytes.is_empty() && body_tx.send(Ok(bytes)).await.is_err() {
                    break; // body bridge was dropped (LocalSet closed :body)
                }
            }
            Some(Err(e)) => {
                let _ = body_tx.send(Err(format!("body read: {e}"))).await;
                break;
            }
        }
    }
    // Dropping body_tx closes the mpsc channel → body bridge closes `:body`.
    conn_abort.abort();
}

// ── LocalSet bridge ─────────────────────────────────────────────────────────────

/// Receives body chunks from the pool task and puts `Value::ByteArray` values
/// on the `:body` channel. Closes the channel at EOF or on error.
async fn local_body_bridge(
    mut body_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    body_chan: GcPtr<NativeObjectBox>,
) {
    while let Some(result) = body_rx.recv().await {
        match result {
            Ok(bytes) if bytes.is_empty() => {}
            Ok(bytes) => {
                if !chan_put(&body_chan, bytes_value(&bytes)).await {
                    break; // consumer closed :body early
                }
            }
            Err(e) => {
                chan_put(&body_chan, net_error(e)).await;
                break;
            }
        }
    }
    chan_ref(body_chan.get()).close();
}

// ── Public Rust API ─────────────────────────────────────────────────────────────

/// Issue an HTTP/2 `GET` request and return a promise channel.
///
/// The promise yields:
/// ```clojure
/// {:status 200 :headers {"k" "v" ...} :body <chan> :resource <H2Resource>}
/// ```
/// or a `Value::Error` on connection/request failure.
///
/// `opts` support `:insecure-skip-verify`, `:alpn` (default `["h2"]`),
/// `:roots` (`:webpki` default, `:system`, or CA PEM path), `:body-buf`
/// (channel depth, default 8).
pub fn get(url: &str, opts: &MapValue, body_buf: usize) -> ValueResult<Value> {
    h2_request(url, http::Method::GET, vec![], opts, body_buf)
}

/// Issue an arbitrary HTTP/2 request and return a promise channel.
pub fn h2_request(
    url: &str,
    method: http::Method,
    extra_headers: Vec<(String, String)>,
    opts: &MapValue,
    body_buf: usize,
) -> ValueResult<Value> {
    let (host, port, uri) = parse_url(url).map_err(ValueError::Other)?;
    let tls_config = h2_client_config(opts)?;

    let (body_tx, body_rx) = mpsc::channel::<Result<Vec<u8>, String>>(body_buf.max(1));
    let (response_tx, response_rx) = oneshot::channel::<Result<H2ResponsePartial, String>>();

    let pool_jh = WorkerPool::global().handle().spawn(pool_h2_request(
        host,
        port,
        tls_config,
        uri,
        method,
        extra_headers,
        body_tx,
        response_tx,
    ));
    let pool_abort = pool_jh.abort_handle();

    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());

    spawn_future(async move {
        do_h2_response(response_rx, body_rx, body_buf, pool_abort, promise).await
    });

    Ok(promise_val)
}

async fn do_h2_response(
    response_rx: oneshot::Receiver<Result<H2ResponsePartial, String>>,
    body_rx: mpsc::Receiver<Result<Vec<u8>, String>>,
    body_buf: usize,
    pool_abort: AbortHandle,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    let partial = match response_rx.await {
        Err(_) => {
            chan_deliver(&promise, net_error("pool task dropped")).await;
            return Ok(Value::Nil);
        }
        Ok(Err(e)) => {
            chan_deliver(&promise, net_error(e)).await;
            return Ok(Value::Nil);
        }
        Ok(Ok(p)) => p,
    };

    // Build :body channel and LocalSet bridge.
    let body_chan = make_chan(body_buf);
    let bridge_jh = tokio::task::spawn_local(local_body_bridge(body_rx, body_chan.clone()));
    let bridge_abort = bridge_jh.abort_handle();

    // Build H2Resource.
    let resource = H2Resource::new();
    {
        let mut g = resource.inner.lock().unwrap();
        g.abort_handles.push(pool_abort);
        g.abort_handles.push(bridge_abort);
    }
    let resource_handle = ResourceHandle::new(resource);

    // Build headers map (string keys → string values).
    let headers_pairs: Vec<(Value, Value)> = partial
        .headers
        .iter()
        .map(|(k, v)| (Value::string(k.clone()), Value::string(v.clone())))
        .collect();
    let headers_val = Value::Map(MapValue::from_pairs(headers_pairs));

    // Deliver response map.
    let resp = Value::Map(MapValue::from_pairs(vec![
        (kw("status"), Value::Long(partial.status as i64)),
        (kw("headers"), headers_val),
        (kw("body"), Value::NativeObject(body_chan)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]));
    chan_deliver(&promise, resp).await;

    Ok(Value::Nil)
}

// ── Builtins ────────────────────────────────────────────────────────────────────

/// `(get url)` or `(get url opts)` — issue a GET request, return promise chan.
///
/// `url` must be a full HTTPS URL, e.g. `"https://example.com/path"`.
/// Opts: `:insecure-skip-verify`, `:alpn`, `:roots`, `:body-buf` (default 8).
fn builtin_get(args: &[Value]) -> ValueResult<Value> {
    let url = match args.first() {
        Some(Value::Str(s)) => s.get().clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "string URL",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let opts = match args.get(1) {
        Some(Value::Map(m)) => m.clone(),
        _ => MapValue::empty(),
    };
    let body_buf = opts_usize(&opts, "body-buf").unwrap_or(8);
    get(&url, &opts, body_buf)
}

/// `(request {:method m :url u :headers {...}} opts)` — general HTTP/2 request.
///
/// `:method` defaults to `"GET"`. `:url` is required. `:headers` is an optional
/// string→string map of extra request headers. No request body is sent;
/// POST/PUT body support is deferred to a later phase.
fn builtin_request(args: &[Value]) -> ValueResult<Value> {
    let req_map = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:url str :method str :headers map}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    let opts = match args.get(1) {
        Some(Value::Map(m)) => m.clone(),
        _ => MapValue::empty(),
    };

    let url = opts_str(&req_map, "url").ok_or_else(|| ValueError::Other(":url required".into()))?;
    let method_str = opts_str(&req_map, "method").unwrap_or_else(|| "GET".to_string());
    let method = method_str
        .parse::<http::Method>()
        .map_err(|e| ValueError::Other(format!("invalid :method '{method_str}': {e}")))?;
    let extra_headers = extract_headers(&req_map);
    let body_buf = opts_usize(&opts, "body-buf").unwrap_or(8);

    h2_request(&url, method, extra_headers, &opts, body_buf)
}

/// `(close resp)` — close an H2 response / connection early.
///
/// Aborts all background tasks, releasing the TLS/TCP connection.
/// The `:body` channel will be closed if it has not already been drained.
fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    let resp = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "h2 response map {:resource <H2Resource>}",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };

    if let Some(Value::NativeObject(obj)) = resp.get(&kw("body")) {
        chan_ref(obj.get()).close();
    }
    if let Some(Value::Resource(handle)) = resp.get(&kw("resource")) {
        let _ = handle.close();
    }

    Ok(Value::Nil)
}
