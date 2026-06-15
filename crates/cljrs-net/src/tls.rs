//! TLS client/server support for `clojure.rust.net.tls`.
//!
//! Phase E delivers TLS on top of TCP, producing the **identical** connection
//! shape as `clojure.rust.net.tcp`:
//!
//! ```clojure
//! {:in          <chan>   ; byte-array chunks from the peer; closed at EOF
//!  :out         <chan>   ; put byte-array/string values here to send
//!  :remote-addr "ip:port"
//!  :local-addr  "ip:port"
//!  :resource    <handle>} ; TlsStreamResource — deterministic socket close
//! ```
//!
//! `connect` returns a capacity-1 promise channel that yields the connection
//! map once the TLS handshake completes, or a `Value::Error` on failure.
//!
//! The server map is:
//!
//! ```clojure
//! {:conns      <chan>   ; yields a connection map for each accepted socket
//!  :local-addr "ip:port"
//!  :resource   <handle>} ; TlsListenerResource — deterministic listener close
//! ```
//!
//! Phase A2: TCP connect, TLS handshake, and I/O tasks all run on the
//! `WorkerPool` multi-thread runtime.  `GcPtr`/`Value` construction happens in
//! LocalSet bridge tasks only.  `TlsStream<TcpStream>: Send` because
//! `ClientConnection`/`ServerConnection: Send`.

use std::any::Any;
use std::io::BufReader;
use std::sync::{Arc, Mutex};

use rustls::ClientConfig;
use rustls::ServerConfig;
use rustls::pki_types::ServerName;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;
use tokio_rustls::{TlsAcceptor, TlsConnector};

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
    PoolSetupResult, PoolStreamSetup, net_error, pool_reader, pool_writer, read_bridge,
    write_bridge,
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

// ── TlsStreamResource ─────────────────────────────────────────────────────────

#[derive(Debug)]
struct TlsStreamInner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` implementation for a TLS stream.
///
/// Holds `AbortHandle`s for the pool reader, pool writer, and LocalSet bridge
/// tasks. `close()` aborts all handles, which drops the socket halves and
/// closes the FD.
#[derive(Debug)]
pub struct TlsStreamResource {
    inner: Arc<Mutex<TlsStreamInner>>,
}

impl TlsStreamResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TlsStreamInner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for TlsStreamResource {
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
        "TlsStream"
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── TlsListenerResource ───────────────────────────────────────────────────────

#[derive(Debug)]
struct TlsListenerInner {
    closed: bool,
    abort_handles: Vec<AbortHandle>,
}

/// `Resource` implementation for a TLS listener.
///
/// Holds `AbortHandle`s for the pool accept loop and the LocalSet accept bridge.
/// `close()` aborts all handles, which drops the listener and closes the FD.
#[derive(Debug)]
pub struct TlsListenerResource {
    inner: Arc<Mutex<TlsListenerInner>>,
}

impl TlsListenerResource {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TlsListenerInner {
                closed: false,
                abort_handles: Vec::new(),
            })),
        }
    }
}

impl Resource for TlsListenerResource {
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
        "TlsListener"
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

fn opts_bool(opts: &MapValue, key: &str) -> Option<bool> {
    match opts.get(&kw(key))? {
        Value::Bool(b) => Some(b),
        _ => None,
    }
}

fn opts_string_vec(opts: &MapValue, key: &str) -> Option<Vec<String>> {
    match opts.get(&kw(key))? {
        Value::Vector(v) => {
            let items: Vec<String> = v
                .get()
                .iter()
                .filter_map(|val| match val {
                    Value::Str(s) => Some(s.get().clone()),
                    _ => None,
                })
                .collect();
            Some(items)
        }
        _ => None,
    }
}

// ── Pool tasks (Send, no GcPtr) ───────────────────────────────────────────────

/// Runs on the pool: accepts TCP connections, performs TLS handshake on the
/// pool, splits the TLS stream, spawns pool_reader + pool_writer, and sends
/// the `PoolStreamSetup` via `conn_info_tx`.
async fn pool_tls_accept_loop(
    std_listener: std::net::TcpListener,
    acceptor: TlsAcceptor,
    conn_info_tx: mpsc::Sender<PoolSetupResult>,
    in_buf: usize,
    out_buf: usize,
) {
    // Convert std listener inside pool runtime context.
    let listener = match TcpListener::from_std(std_listener) {
        Ok(l) => l,
        Err(e) => {
            let _ = conn_info_tx.send(Err(format!("from_std: {e}"))).await;
            return;
        }
    };

    let local_addr = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();

    loop {
        match listener.accept().await {
            Err(e) => {
                let _ = conn_info_tx.send(Err(format!("accept error: {e}"))).await;
                break;
            }
            Ok((tcp_stream, peer_addr)) => {
                let remote_addr = peer_addr.to_string();
                let local_addr_conn = local_addr.clone();
                let acceptor = acceptor.clone();
                let conn_info_tx = conn_info_tx.clone();

                // Spawn per-connection TLS handshake as a separate pool task.
                tokio::spawn(async move {
                    let tls_stream = match acceptor.accept(tcp_stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = conn_info_tx.send(Err(format!("TLS handshake: {e}"))).await;
                            return;
                        }
                    };

                    let (read_half, write_half) = tokio::io::split(tls_stream);
                    let (read_tx, read_rx) =
                        mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
                    let (write_tx, write_rx) = mpsc::channel::<Vec<u8>>(out_buf.max(1));

                    let reader_jh = tokio::spawn(pool_reader(read_half, read_tx));
                    let writer_jh = tokio::spawn(pool_writer(write_half, write_rx));

                    let setup = PoolStreamSetup {
                        remote_addr,
                        local_addr: local_addr_conn,
                        read_rx,
                        write_tx,
                        reader_abort: reader_jh.abort_handle(),
                        writer_abort: writer_jh.abort_handle(),
                    };
                    let _ = conn_info_tx.send(Ok(setup)).await;
                });
            }
        }
    }
}

// ── LocalSet bridge (touches GcPtr / Value) ───────────────────────────────────

/// Runs on the LocalSet: receives `PoolSetupResult` from the pool accept loop,
/// creates Clojure channels + bridges for each new connection, and puts the
/// connection map on `conns_chan`.
async fn local_tls_accept_bridge(
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
                let conn = make_tls_connection_from_setup(setup, in_buf, out_buf);
                if !chan_put(&conns_chan, conn).await {
                    break; // consumer closed :conns
                }
            }
        }
    }
    chan_ref(conns_chan.get()).close();
}

/// Build a TLS connection map + resource from a `PoolStreamSetup`, spawning
/// LocalSet bridge tasks. Called on the LocalSet thread.
fn make_tls_connection_from_setup(setup: PoolStreamSetup, in_buf: usize, out_buf: usize) -> Value {
    let in_chan = make_chan(in_buf);
    let out_chan = make_chan(out_buf);
    let resource = TlsStreamResource::new();
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
        (kw("remote-addr"), Value::string(setup.remote_addr)),
        (kw("local-addr"), Value::string(setup.local_addr)),
        (kw("resource"), Value::Resource(resource_handle)),
    ]))
}

// ── SkipCertVerification ──────────────────────────────────────────────────────

#[derive(Debug)]
struct SkipCertVerification(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for SkipCertVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

// ── Client config builder ─────────────────────────────────────────────────────

pub fn build_client_config(opts: &MapValue) -> ValueResult<Arc<ClientConfig>> {
    let insecure = opts_bool(opts, "insecure-skip-verify").unwrap_or(false);

    let mut config = if insecure {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        ClientConfig::builder_with_provider(provider.clone())
            .with_safe_default_protocol_versions()
            .map_err(|e| ValueError::Other(format!("TLS protocol versions: {e}")))?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipCertVerification(provider)))
            .with_no_client_auth()
    } else {
        let mut root_store = rustls::RootCertStore::empty();

        // Determine root source
        let roots_val = opts.get(&kw("roots"));
        match &roots_val {
            None => {
                // Default: webpki roots
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
            Some(Value::Keyword(kw_val)) if kw_val.get().name.as_ref() == "webpki" => {
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
            Some(Value::Keyword(kw_val)) if kw_val.get().name.as_ref() == "system" => {
                let native_certs = rustls_native_certs::load_native_certs();
                for cert in native_certs.certs {
                    root_store.add(cert).ok();
                }
            }
            Some(Value::Str(s)) => {
                // Load PEM certs from file path
                let certs = load_certs(s.get())?;
                for cert in certs {
                    root_store
                        .add(cert)
                        .map_err(|e| ValueError::Other(format!("add cert: {e}")))?;
                }
            }
            _ => {
                // Fall back to webpki roots for unknown values
                root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            }
        }

        ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| ValueError::Other(format!("tls provider error: {e}")))?
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };

    // ALPN
    if let Some(alpn) = opts_string_vec(opts, "alpn") {
        config.alpn_protocols = alpn.into_iter().map(|s| s.into_bytes()).collect();
    }

    Ok(Arc::new(config))
}

// ── Server config builder ─────────────────────────────────────────────────────

pub fn build_server_config(opts: &MapValue) -> ValueResult<Arc<ServerConfig>> {
    let cert_path =
        opts_str(opts, "cert").ok_or_else(|| ValueError::Other(":cert required".into()))?;
    let key_path =
        opts_str(opts, "key").ok_or_else(|| ValueError::Other(":key required".into()))?;

    let certs = load_certs(&cert_path)?;
    let key = load_private_key(&key_path)?;

    let mut config =
        ServerConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .map_err(|e| ValueError::Other(format!("tls provider error: {e}")))?
            .with_no_client_auth()
            .with_single_cert(certs, key)
            .map_err(|e| ValueError::Other(format!("server cert/key error: {e}")))?;

    // ALPN
    if let Some(alpn) = opts_string_vec(opts, "alpn") {
        config.alpn_protocols = alpn.into_iter().map(|s| s.into_bytes()).collect();
    }

    Ok(Arc::new(config))
}

// ── Cert/key loaders ──────────────────────────────────────────────────────────

fn load_certs(path: &str) -> ValueResult<Vec<rustls::pki_types::CertificateDer<'static>>> {
    let file =
        std::fs::File::open(path).map_err(|e| ValueError::Other(format!("open {path}: {e}")))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ValueError::Other(format!("read certs from {path}: {e}")))
}

fn load_private_key(path: &str) -> ValueResult<rustls::pki_types::PrivateKeyDer<'static>> {
    let file =
        std::fs::File::open(path).map_err(|e| ValueError::Other(format!("open {path}: {e}")))?;
    let mut reader = BufReader::new(file);
    rustls_pemfile::private_key(&mut reader)
        .map_err(|e| ValueError::Other(format!("read key from {path}: {e}")))?
        .ok_or_else(|| ValueError::Other(format!("no private key found in {path}")))
}

// ── Connect implementation ────────────────────────────────────────────────────

/// Initiate a TLS connection and return the promise channel as a `Value`.
///
/// TCP connect + TLS handshake + I/O tasks run on the `WorkerPool`; bridge
/// tasks run on the LocalSet.
pub fn tls_connect_to(
    host: &str,
    port: u16,
    config: Arc<ClientConfig>,
    in_buf: usize,
    out_buf: usize,
) -> Value {
    let host = host.to_string();
    let promise = make_chan(1);
    let promise_val = Value::NativeObject(promise.clone());
    spawn_future(async move { do_tls_connect(host, port, config, in_buf, out_buf, promise).await });
    promise_val
}

async fn do_tls_connect(
    host: String,
    port: u16,
    config: Arc<ClientConfig>,
    in_buf: usize,
    out_buf: usize,
    promise: GcPtr<NativeObjectBox>,
) -> EvalResult {
    let addr = format!("{host}:{port}");

    // Channel for pool → LocalSet handshake.
    let (setup_tx, setup_rx) = oneshot::channel::<PoolSetupResult>();

    // Spawn entire TLS connect + I/O pump on pool (Send).
    WorkerPool::global().handle().spawn(async move {
        // TCP connect
        let tcp_stream = match TcpStream::connect(&addr).await {
            Ok(s) => s,
            Err(e) => {
                let _ = setup_tx.send(Err(format!("connect to {addr}: {e}")));
                return;
            }
        };

        let remote_addr = tcp_stream
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();
        let local_addr = tcp_stream
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();

        // TLS handshake
        let server_name = match ServerName::try_from(host.as_str()) {
            Ok(n) => n.to_owned(),
            Err(e) => {
                let _ = setup_tx.send(Err(format!("invalid SNI hostname {host}: {e}")));
                return;
            }
        };

        let connector = TlsConnector::from(config);
        let tls_stream = match connector.connect(server_name, tcp_stream).await {
            Ok(s) => s,
            Err(e) => {
                let _ = setup_tx.send(Err(format!("TLS handshake: {e}")));
                return;
            }
        };

        let (read_half, write_half) = tokio::io::split(tls_stream);
        let (read_tx, read_rx) = mpsc::channel::<crate::pool_io::ReadMsg>(in_buf.max(1));
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
    });

    // Await the setup result back on the LocalSet.
    match setup_rx.await {
        Err(_) => {
            chan_deliver(&promise, net_error("pool task dropped")).await;
        }
        Ok(Err(e)) => {
            chan_deliver(&promise, net_error(e)).await;
        }
        Ok(Ok(setup)) => {
            let conn = make_tls_connection_from_setup(setup, in_buf, out_buf);
            chan_deliver(&promise, conn).await;
        }
    }
    Ok(Value::Nil)
}

// ── Listen implementation ─────────────────────────────────────────────────────

/// Bind a TLS listener on `host:port` and return a server map.
///
/// The accept loop and TLS handshakes run on the `WorkerPool`; a LocalSet bridge
/// delivers connection maps onto `conns_chan`.
pub fn tls_listen_on(
    host: &str,
    port: u16,
    config: Arc<ServerConfig>,
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
    let acceptor = TlsAcceptor::from(config);

    let resource = TlsListenerResource::new();
    let shared_inner = resource.inner.clone();
    let resource_handle = ResourceHandle::new(resource);

    // Spawn accept loop on pool (Send).
    let pool_jh = WorkerPool::global().handle().spawn(pool_tls_accept_loop(
        std_listener,
        acceptor,
        conn_info_tx,
        in_buf,
        out_buf,
    ));

    // Spawn bridge on LocalSet (!Send, touches GcPtr).
    let bridge_jh = tokio::task::spawn_local(local_tls_accept_bridge(
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

// ── Builtins ──────────────────────────────────────────────────────────────────

/// `(connect {:host h :port p :cert "ca.pem"})` — returns a promise channel.
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

    let config = build_client_config(&opts)?;
    Ok(tls_connect_to(&host, port, config, in_buf, out_buf))
}

/// `(close conn)` — close a TLS connection map.
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

/// `(listen {:port p :cert "cert.pem" :key "key.pem"})` — bind a TLS listener.
fn builtin_listen(args: &[Value]) -> ValueResult<Value> {
    let opts = match args.first() {
        Some(Value::Map(m)) => m.clone(),
        other => {
            return Err(ValueError::WrongType {
                expected: "map {:port long :cert str :key str}",
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

    let config = build_server_config(&opts)?;
    tls_listen_on(&host, port, config, conns_buf, in_buf, out_buf)
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
