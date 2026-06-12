//! Network side of the server: TCP accept loop and per-connection tasks.
//!
//! Everything here runs on the dedicated network thread's current-thread
//! tokio runtime and only ever touches `Send` data. `describe` and
//! `interrupt` are answered here (an interrupt must not queue behind the
//! eval it is trying to cancel); all other ops become [`Job`]s for the
//! interpreter thread.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::sync::watch;

use crate::Job;
use crate::bencode::{self, Bencode};
use crate::protocol::{Request, Response};

/// Identifies an in-flight (queued or evaluating) request: `(session, id)`.
pub(crate) type PendingKey = (String, String);

/// Registry of in-flight requests and their cancellation flags, shared
/// between connection tasks (which register and interrupt) and the
/// interpreter thread (which clears entries as jobs finish).
#[derive(Default)]
pub(crate) struct Pending {
    map: Mutex<HashMap<PendingKey, Arc<AtomicBool>>>,
}

impl Pending {
    fn insert(&self, key: PendingKey, flag: Arc<AtomicBool>) {
        self.map.lock().unwrap().insert(key, flag);
    }

    pub(crate) fn remove(&self, key: &PendingKey) {
        self.map.lock().unwrap().remove(key);
    }

    /// Set the cancellation flag for `id` in `session` (any id in the session
    /// when `id` is `None`). Returns true if something was flagged.
    fn interrupt(&self, session: &str, id: Option<&str>) -> bool {
        let map = self.map.lock().unwrap();
        let mut hit = false;
        for ((s, i), flag) in map.iter() {
            if s == session && id.is_none_or(|want| want == i) {
                flag.store(true, Ordering::SeqCst);
                hit = true;
            }
        }
        hit
    }
}

/// Accept loop. Returns when the shutdown signal fires; dropping the runtime
/// afterwards aborts the connection tasks spawned here.
pub(crate) async fn run(
    listener: std::net::TcpListener,
    job_tx: mpsc::Sender<Job>,
    mut shutdown: watch::Receiver<bool>,
) {
    let listener = match TcpListener::from_std(listener) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nrepl: failed to register listener with tokio: {e}");
            return;
        }
    };
    let pending = Arc::new(Pending::default());
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                match accepted {
                    Ok((stream, _peer)) => {
                        let _ = stream.set_nodelay(true);
                        let (read_half, write_half) = stream.into_split();
                        let (reply_tx, reply_rx) =
                            tokio::sync::mpsc::unbounded_channel::<Bencode>();
                        tokio::spawn(writer_task(write_half, reply_rx));
                        tokio::spawn(reader_task(
                            read_half,
                            reply_tx,
                            job_tx.clone(),
                            pending.clone(),
                        ));
                    }
                    Err(e) => {
                        eprintln!("nrepl: accept error: {e}");
                    }
                }
            }
            _ = shutdown.changed() => return,
        }
    }
}

/// Drains response messages and writes their bencode encoding to the socket.
async fn writer_task(mut write_half: OwnedWriteHalf, mut replies: UnboundedReceiver<Bencode>) {
    while let Some(msg) = replies.recv().await {
        let bytes = bencode::encode_to_vec(&msg);
        if write_half.write_all(&bytes).await.is_err() {
            return;
        }
    }
}

/// Reads bencode frames off the socket and dispatches each request.
async fn reader_task(
    mut read_half: OwnedReadHalf,
    reply_tx: UnboundedSender<Bencode>,
    job_tx: mpsc::Sender<Job>,
    pending: Arc<Pending>,
) {
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    loop {
        // Decode every complete message currently buffered.
        loop {
            match bencode::decode(&buf) {
                Ok(Some((msg, consumed))) => {
                    buf.drain(..consumed);
                    dispatch(&msg, &reply_tx, &job_tx, &pending);
                }
                Ok(None) => break, // incomplete — read more bytes
                Err(e) => {
                    eprintln!("nrepl: dropping connection ({e})");
                    return;
                }
            }
        }
        match read_half.read(&mut chunk).await {
            Ok(0) | Err(_) => return, // EOF or socket error
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn dispatch(
    msg: &Bencode,
    reply_tx: &UnboundedSender<Bencode>,
    job_tx: &mpsc::Sender<Job>,
    pending: &Arc<Pending>,
) {
    let Some(req) = Request::from_bencode(msg) else {
        return; // not a dict, or no "op" — nothing to dispatch
    };
    match req.op.as_str() {
        "describe" => {
            let _ = reply_tx.send(describe_response(&req));
        }
        "interrupt" => {
            let session = req.session.clone().unwrap_or_default();
            let hit = pending.interrupt(&session, req.interrupt_id.as_deref());
            let resp = Response::for_request(&req, &session);
            let resp = if hit {
                resp.status(&["done"])
            } else {
                resp.status(&["done", "session-idle"])
            };
            let _ = reply_tx.send(resp.build());
        }
        _ => {
            let cancelled = Arc::new(AtomicBool::new(false));
            // Only evals are interruptible, and only when addressable.
            let pending_key = match (req.op.as_str(), &req.session, &req.id) {
                ("eval" | "load-file", Some(s), Some(i)) => {
                    let key = (s.clone(), i.clone());
                    pending.insert(key.clone(), cancelled.clone());
                    Some(key)
                }
                _ => None,
            };
            let job = Job {
                req,
                replies: reply_tx.clone(),
                cancelled,
                pending_key,
                pending: pending.clone(),
            };
            if job_tx.send(job).is_err() {
                // Interpreter thread is gone; the server is shutting down.
            }
        }
    }
}

/// The ops this server understands, advertised to clients.
const OPS: &[&str] = &[
    "clone",
    "close",
    "completions",
    "describe",
    "eval",
    "interrupt",
    "load-file",
    "lookup",
    "ls-sessions",
];

fn describe_response(req: &Request) -> Bencode {
    let ops: BTreeMap<Vec<u8>, Bencode> = OPS
        .iter()
        .map(|op| (op.as_bytes().to_vec(), Bencode::Dict(BTreeMap::new())))
        .collect();

    let version = |s: &str| -> Bencode {
        let mut parts = s.split('.');
        let mut dict = BTreeMap::new();
        for key in ["major", "minor", "incremental"] {
            let n: i64 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            dict.insert(key.as_bytes().to_vec(), Bencode::Int(n));
        }
        dict.insert(b"version-string".to_vec(), Bencode::str(s));
        Bencode::Dict(dict)
    };
    let mut versions = BTreeMap::new();
    versions.insert(b"cljrs".to_vec(), version(env!("CARGO_PKG_VERSION")));
    versions.insert(b"nrepl".to_vec(), version("1.0.0"));

    Response::for_request(&req.clone(), req.session.as_deref().unwrap_or("none"))
        .field("ops", Bencode::Dict(ops))
        .field("versions", Bencode::Dict(versions))
        .status(&["done"])
        .build()
}
