//! HTTP/3 client for `clojure.rust.net.h3` (Phase Q3).
//!
//! Wraps `h3`/`h3-quinn` on top of a quinn QUIC connection. For each request a
//! fresh QUIC connection is established; the TLS/QUIC handshake and all HTTP/3
//! I/O run on the `WorkerPool`; the response body is streamed to a `:body`
//! channel via a `LocalSet` bridge task.
//!
//! Response map:
//! ```clojure
//! {:status  200
//!  :headers {"content-type" "text/plain" ...}  ; string keys, string values
//!  :body    <chan>    ; byte-array chunks; closed at EOF or on error
//!  :resource <H3Resource>}
//! ```
//!
//! `H3Resource` keeps the QUIC connection alive and holds abort handles for the
//! pool streaming task and LocalSet bridge. Call `(h3/close resp)` to terminate
//! the connection before the body is fully drained.
//!
//! ALPN: HTTP/3 requires the ALPN token `"h3"`. Pass `:alpn ["h3"]` in opts
//! (or rely on the default injection done by `h3_client_config`).

use std::any::Any;
use std::sync::{Arc, Mutex};

use bytes::Buf;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use cljrs_async::channel::{chan_deliver, chan_put, chan_ref, make_chan};
use cljrs_async::eval_async::spawn_future;
use cljrs_async::worker_pool::WorkerPool;
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_value::{
    Arity, Keyword, MapValue, NativeFn, NativeObjectBox, PersistentVector, Resource, ResourceHandle,
    Value, ValueError, ValueResult,
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

// ── H3Resource ──────────────────────────────────────────────────────────────────

#[derive(Debug)]
struct H3Inner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` for an HTTP/3 connection.
///
/// Holds the underlying `quinn::Connection` and `quinn::Endpoint` (keeping the
/// QUIC/UDP driver alive) and abort handles for the WorkerPool body-streaming
/// task and the LocalSet body-bridge task. `close()` aborts all tasks and sends
/// a QUIC CONNECTION_CLOSE to the server. `resource_type` → `"H3Connection"`.
#[derive(Debug)]
pub struct H3Resource {
    connection: quinn::Connection,
    _endpoint: quinn::Endpoint,
    inner: Arc<Mutex<H3Inner>>,
}

impl H3Resource {
    fn new(connection: quinn::Connection, endpoint: quinn::Endpoint) -> Self {
        Self {
            connection,
            _endpoint: endpoint,
            inner: Arc::new(Mutex::new(H3Inner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for H3Resource {
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
            .close(quinn::VarInt::from_u32(0), b"h3 closed");
        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.lock().unwrap().closed
    }

    fn resource_type(&self) -> &'static str {
        "H3Connection"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Internal types ──────────────────────────────────────────────────────────────

/// Partial response from the pool task — delivered as soon as response headers
/// arrive so the LocalSet can build the `:body` channel and response map before
/// the body is fully streamed.
struct H3ResponsePartial {
    status: u16,
    headers: Vec<(String, String)>,
    connection: quinn::Connection,
    endpoint: quinn::Endpoint,
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

/// Return a `quinn::ClientConfig` suitable for HTTP/3: identical to the TLS/QUIC
/// client config from `quic_config`, but with `"h3"` injected into `:alpn` when
/// the caller did not set it.
fn h3_client_config(opts: &MapValue) -> ValueResult<quinn::ClientConfig> {
    if opts.get(&kw("alpn")).is_some() {
        return crate::quic_config::client_config(opts);
    }
    let alpn_val = Value::Vector(GcPtr::new(PersistentVector::from_iter([Value::string(
        "h3",
    )])));
    let new_opts = opts.assoc(kw("alpn"), alpn_val);
    crate::quic_config::client_config(&new_opts)
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

/// Extract `{:headers {"k" "v" ...}}` from a request map. Returns an empty vec
/// if the key is absent or not a string→string map.
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
/// 1. QUIC connect (mirrors `quic.rs::do_quic_connect`).
/// 2. HTTP/3 handshake via `h3_quinn::Connection` + `h3::client::new`.
/// 3. Spawn the h3 connection driver task (needed for control-stream frames).
/// 4. Send the HTTP request and receive response headers.
/// 5. Send `H3ResponsePartial` (status + headers + QUIC objects) via `response_tx`
///    so the `LocalSet` can build the response map while body streaming continues.
/// 6. Stream response body chunks to `body_tx` (dropping it closes the `:body` chan).
#[allow(clippy::too_many_arguments)]
async fn pool_h3_request(
    host: String,
    port: u16,
    quinn_config: quinn::ClientConfig,
    uri: http::Uri,
    method: http::Method,
    extra_headers: Vec<(String, String)>,
    body_tx: mpsc::Sender<Result<Vec<u8>, String>>,
    response_tx: oneshot::Sender<Result<H3ResponsePartial, String>>,
) {
    macro_rules! fail {
        ($msg:expr) => {{
            let _ = response_tx.send(Err($msg));
            return;
        }};
    }

    // ── 1. QUIC connect ────────────────────────────────────────────────────────
    let addr_str = format!("{host}:{port}");
    let addrs: Vec<std::net::SocketAddr> = match tokio::net::lookup_host(&addr_str).await {
        Ok(it) => {
            let (v4, v6): (Vec<_>, Vec<_>) = it.partition(|a| a.is_ipv4());
            v4.into_iter().chain(v6).collect()
        }
        Err(e) => fail!(format!("lookup {addr_str}: {e}")),
    };
    if addrs.is_empty() {
        fail!(format!("no address for {addr_str}"));
    }

    let mut last_err = String::new();
    let mut maybe_conn: Option<(quinn::Connection, quinn::Endpoint)> = None;
    for addr in &addrs {
        let bind: std::net::SocketAddr = if addr.is_ipv6() {
            "[::]:0".parse().unwrap()
        } else {
            "0.0.0.0:0".parse().unwrap()
        };
        let mut endpoint = match quinn::Endpoint::client(bind) {
            Ok(e) => e,
            Err(e) => {
                last_err = format!("endpoint: {e}");
                continue;
            }
        };
        endpoint.set_default_client_config(quinn_config.clone());
        let connecting = match endpoint.connect(*addr, &host) {
            Ok(c) => c,
            Err(e) => {
                last_err = format!("connect: {e}");
                continue;
            }
        };
        match connecting.await {
            Ok(connection) => {
                maybe_conn = Some((connection, endpoint));
                break;
            }
            Err(e) => {
                last_err = format!("handshake: {e}");
                continue;
            }
        }
    }
    let (quinn_connection, endpoint) = match maybe_conn {
        Some(pair) => pair,
        None => fail!(last_err),
    };

    // ── 2. H3 setup ───────────────────────────────────────────────────────────
    let h3_conn = h3_quinn::Connection::new(quinn_connection.clone());
    let (mut driver, mut send_request) = match h3::client::new(h3_conn).await {
        Ok(pair) => pair,
        Err(e) => fail!(format!("h3 handshake: {e}")),
    };

    // Spawn the h3 connection driver (processes control-stream frames while
    // request/response I/O is in flight).
    let driver_jh = tokio::spawn(async move {
        let _ = std::future::poll_fn(|cx| driver.poll_close(cx)).await;
    });
    let driver_abort = driver_jh.abort_handle();

    // ── 3. Build and send request ─────────────────────────────────────────────
    let mut req_builder = http::Request::builder().method(method).uri(uri);
    for (k, v) in &extra_headers {
        req_builder = req_builder.header(k.as_str(), v.as_str());
    }
    let req = match req_builder.body(()) {
        Ok(r) => r,
        Err(e) => {
            driver_abort.abort();
            fail!(format!("build request: {e}"));
        }
    };

    let mut stream = match send_request.send_request(req).await {
        Ok(s) => s,
        Err(e) => {
            driver_abort.abort();
            fail!(format!("send request: {e}"));
        }
    };

    // Finish the request (no body for GET / HEAD; POST body is future work).
    if let Err(e) = stream.finish().await {
        driver_abort.abort();
        fail!(format!("finish request: {e}"));
    }

    // ── 4. Receive response headers ───────────────────────────────────────────
    let response = match stream.recv_response().await {
        Ok(r) => r,
        Err(e) => {
            driver_abort.abort();
            fail!(format!("recv response: {e}"));
        }
    };

    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(k, v)| {
            (
                k.as_str().to_string(),
                v.to_str().unwrap_or("").to_string(),
            )
        })
        .collect();

    let partial = H3ResponsePartial {
        status,
        headers,
        connection: quinn_connection,
        endpoint,
    };

    // ── 5. Deliver headers to LocalSet ────────────────────────────────────────
    if response_tx.send(Ok(partial)).is_err() {
        // LocalSet gave up before headers arrived; clean up.
        driver_abort.abort();
        return;
    }

    // ── 6. Stream response body ───────────────────────────────────────────────
    loop {
        match stream.recv_data().await {
            Ok(None) => break,
            Ok(Some(mut chunk)) => {
                let n = chunk.remaining();
                let bytes = chunk.copy_to_bytes(n).to_vec();
                if body_tx.send(Ok(bytes)).await.is_err() {
                    break; // body bridge was dropped (LocalSet closed :body)
                }
            }
            Err(e) => {
                let _ = body_tx.send(Err(format!("body read: {e}"))).await;
                break;
            }
        }
    }
    // Dropping body_tx closes the mpsc channel → body bridge closes `:body`.
    driver_abort.abort();
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
            Ok(bytes) if bytes.is_empty() => {} // skip empty DATA frames
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

/// Issue an HTTP/3 `GET` request and return a promise channel.
///
/// The promise yields:
/// ```clojure
/// {:status 200 :headers {"k" "v" ...} :body <chan> :resource <H3Resource>}
/// ```
/// or a `Value::Error` on connection/request failure.
///
/// `opts` are the same as `quic/connect` (`:insecure-skip-verify`, `:alpn`,
/// `:roots`, `:max-idle-ms`, etc.). `:alpn ["h3"]` is injected automatically
/// when absent.
pub fn get(url: &str, opts: &MapValue, body_buf: usize) -> ValueResult<Value> {
    h3_request(
        url,
        http::Method::GET,
        vec![],
        opts,
        body_buf,
    )
}

/// Issue an arbitrary HTTP/3 request and return a promise channel.
///
/// `method` is a string (e.g. `"GET"`, `"POST"`). `extra_headers` is a list of
/// `(name, value)` pairs to add to the request. Currently no request body is
/// sent (POST body support is deferred to a later phase).
pub fn h3_request(
    url: &str,
    method: http::Method,
    extra_headers: Vec<(String, String)>,
    opts: &MapValue,
    body_buf: usize,
) -> ValueResult<Value> {
    let (host, port, uri) =
        parse_url(url).map_err(ValueError::Other)?;
    let quinn_config = h3_client_config(opts)?;

    let (body_tx, body_rx) =
        mpsc::channel::<Result<Vec<u8>, String>>(body_buf.max(1));
    let (response_tx, response_rx) =
        oneshot::channel::<Result<H3ResponsePartial, String>>();

    let pool_jh = WorkerPool::global().handle().spawn(pool_h3_request(
        host,
        port,
        quinn_config,
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
        do_h3_response(response_rx, body_rx, body_buf, pool_abort, promise).await
    });

    Ok(promise_val)
}

async fn do_h3_response(
    response_rx: oneshot::Receiver<Result<H3ResponsePartial, String>>,
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
    let bridge_jh =
        tokio::task::spawn_local(local_body_bridge(body_rx, body_chan.clone()));
    let bridge_abort = bridge_jh.abort_handle();

    // Build H3Resource.
    let resource = H3Resource::new(partial.connection, partial.endpoint);
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
/// `url` must be a full HTTPS URL, e.g. `"https://host:4433/path"`.
/// Opts: `:insecure-skip-verify`, `:alpn`, `:roots`, `:max-idle-ms`,
/// `:keep-alive-ms`, `:max-streams`, `:body-buf` (channel depth, default 8).
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

/// `(request {:method m :url u :headers {...}} opts)` — general HTTP/3 request.
///
/// `:method` defaults to `"GET"`. `:url` is required. `:headers` is an optional
/// map of extra request header name→value strings. No request body is sent;
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

    let url =
        opts_str(&req_map, "url").ok_or_else(|| ValueError::Other(":url required".into()))?;
    let method_str = opts_str(&req_map, "method").unwrap_or_else(|| "GET".to_string());
    let method = method_str
        .parse::<http::Method>()
        .map_err(|e| ValueError::Other(format!("invalid :method '{method_str}': {e}")))?;
    let extra_headers = extract_headers(&req_map);
    let body_buf = opts_usize(&opts, "body-buf").unwrap_or(8);

    h3_request(&url, method, extra_headers, &opts, body_buf)
}

/// `(close resp)` — close an H3 response / connection early.
///
/// Aborts all background tasks and sends a QUIC CONNECTION_CLOSE to the server.
/// The `:body` channel will be closed if it hasn't already been drained.
fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    let resp = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "h3 response map {:resource <H3Resource>}",
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
