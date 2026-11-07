//! Network side of the server: TCP accept loop and per-connection tasks.
//!
//! Everything here runs on the dedicated network thread's current-thread
//! tokio runtime and only ever touches `Send` data. `describe` and
//! `interrupt` are answered here (an interrupt must not queue behind the
//! eval it is trying to cancel); all other ops become [`Job`]s for the
//! interpreter thread, subject to the job admission limits in
//! [`crate::limits`]. Interrupts are answered before admission, so they keep
//! working when the server is saturated.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, mpsc};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::watch;

use cljrs_runtime::env::gas::InterruptFlag;

use crate::Job;
use crate::bencode::{self, Bencode};
use crate::limits::{Admission, ReplyReceiver, ReplySender, reply_channel};
use crate::protocol::{Request, Response};

/// Identifies an in-flight (queued or evaluating) request: `(session, id)`.
pub(crate) type PendingKey = (String, String);

/// Registry of in-flight requests and their cancellation flags, shared
/// between connection tasks (which register and interrupt) and the
/// interpreter thread (which clears entries as jobs finish).
#[derive(Default)]
pub(crate) struct Pending {
    map: Mutex<HashMap<PendingKey, Arc<InterruptFlag>>>,
}

impl Pending {
    fn insert(&self, key: PendingKey, flag: Arc<InterruptFlag>) {
        self.map.lock().unwrap().insert(key, flag);
    }

    pub(crate) fn remove(&self, key: &PendingKey) {
        self.map.lock().unwrap().remove(key);
    }

    /// Set the cancellation flag for `id` in `session` (any id in the session
    /// when `id` is `None`), reporting the nREPL status of the attempt.
    fn interrupt(&self, session: &str, id: Option<&str>) -> &'static [&'static str] {
        let map = self.map.lock().unwrap();
        let mut hit = false;
        let mut busy = false;
        for ((s, i), flag) in map.iter() {
            if s != session {
                continue;
            }
            busy = true;
            if id.is_none_or(|want| want == i) {
                // Also wakes the eval if it is parked on a wait.
                cljrs_runtime::env::gas::request_interrupt(flag);
                hit = true;
            }
        }
        match (hit, busy) {
            (true, _) => &["done"],
            (false, true) => &["interrupt-id-mismatch", "done"],
            (false, false) => &["session-idle", "done"],
        }
    }
}

/// Accept loop. Returns when the shutdown signal fires; dropping the runtime
/// afterwards aborts the connection tasks spawned here.
pub(crate) async fn run(
    listener: std::net::TcpListener,
    job_tx: mpsc::Sender<Job>,
    admission: Arc<Admission>,
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
                        let (reply_tx, reply_rx, closed) =
                            reply_channel(admission.limits().max_output_bytes);
                        tokio::spawn(writer_task(write_half, reply_rx, closed.clone()));
                        tokio::spawn(reader_task(
                            read_half,
                            reply_tx,
                            closed,
                            job_tx.clone(),
                            pending.clone(),
                            admission.clone(),
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

/// Drains encoded response messages and writes them to the socket. Stops
/// when the connection is closed for exceeding its output limit.
async fn writer_task(
    mut write_half: OwnedWriteHalf,
    mut replies: ReplyReceiver,
    mut closed: watch::Receiver<bool>,
) {
    loop {
        let bytes = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return,
            msg = replies.recv() => match msg {
                Some(bytes) => bytes,
                None => return,
            },
        };
        let written = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return,
            result = write_half.write_all(&bytes) => result,
        };
        if written.is_err() {
            return;
        }
        replies.written(bytes.len());
    }
}

/// Reads bencode frames off the socket and dispatches each request.
///
/// The read buffer never holds more than one request frame of at most
/// [`crate::Limits::max_request_bytes`] (plus one read chunk): a connection
/// that sends a larger frame is closed.
async fn reader_task(
    mut read_half: OwnedReadHalf,
    reply_tx: ReplySender,
    mut closed: watch::Receiver<bool>,
    job_tx: mpsc::Sender<Job>,
    pending: Arc<Pending>,
    admission: Arc<Admission>,
) {
    let max_request = admission.limits().max_request_bytes;
    let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
    let mut chunk = [0u8; 8 * 1024];
    loop {
        // Decode every complete message currently buffered.
        loop {
            match bencode::decode(&buf) {
                Ok(Some((_, consumed))) if consumed > max_request => {
                    eprintln!(
                        "nrepl: dropping connection (request of {consumed} bytes exceeds the {max_request}-byte limit)"
                    );
                    return;
                }
                Ok(Some((msg, consumed))) => {
                    buf.drain(..consumed);
                    dispatch(&msg, &reply_tx, &job_tx, &pending, &admission);
                }
                Ok(None) if buf.len() > max_request => {
                    eprintln!(
                        "nrepl: dropping connection (incomplete request exceeds the {max_request}-byte limit)"
                    );
                    return;
                }
                Ok(None) => break, // incomplete — read more bytes
                Err(e) => {
                    eprintln!("nrepl: dropping connection ({e})");
                    return;
                }
            }
        }
        if reply_tx.closed() {
            return;
        }
        let read = tokio::select! {
            biased;
            _ = closed.wait_for(|closed| *closed) => return,
            read = read_half.read(&mut chunk) => read,
        };
        match read {
            Ok(0) | Err(_) => return, // EOF or socket error
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
        }
    }
}

fn dispatch(
    msg: &Bencode,
    reply_tx: &ReplySender,
    job_tx: &mpsc::Sender<Job>,
    pending: &Arc<Pending>,
    admission: &Arc<Admission>,
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
            let status = pending.interrupt(&session, req.interrupt_id.as_deref());
            let resp = Response::for_request(&req, &session).status(status);
            let _ = reply_tx.send(resp.build());
        }
        _ => {
            // Admission happens after `describe`/`interrupt`, which never
            // queue, so a saturated server still answers interrupts.
            let ticket = match admission.try_admit(&crate::lane_key(&req)) {
                Ok(ticket) => ticket,
                Err(refusal) => {
                    let session = req.session.clone().unwrap_or_default();
                    let _ = reply_tx.send(
                        Response::for_request(&req, &session)
                            .str_field("err", refusal.message(admission.limits()))
                            .status(&["error", "overloaded", "done"])
                            .build(),
                    );
                    return;
                }
            };
            let cancelled = Arc::new(InterruptFlag::new(false));
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
                ticket,
            };
            if job_tx.send(job).is_err() {
                // Interpreter thread is gone; the server is shutting down.
            }
        }
    }
}

/// The ops this server understands, advertised to clients.
const OPS: &[&str] = &[
    "analyze-last-stacktrace",
    "clone",
    "close",
    "completions",
    "describe",
    "eval",
    "interrupt",
    "load-file",
    "lookup",
    "ls-sessions",
    "macroexpand",
    "stacktrace",
];

/// The Clojure language level this dialect implements, as reported to nREPL
/// clients.
///
/// 1.11 is what the core actually provides: `update-keys`, `update-vals`,
/// `abs`, `parse-long`, `parse-double` and `random-uuid` are all present, and
/// no 1.12 addition (`partitionv`, `splitv-at`, …) is. `iteration` is the one
/// 1.11 name still missing; raise this only when the level below it is whole.
const CLOJURE_LEVEL: &str = "1.11.0";

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
    // The Clojure language level this dialect implements. Clients read
    // `versions.clojure` to decide which version-gated features to enable;
    // without it CIDER reports "Can't determine Clojure version" and turns
    // parts of itself off. It answers "which Clojure does this speak", not
    // "which implementation is this" — that is what `cljrs` above is for.
    versions.insert(b"clojure".to_vec(), version(CLOJURE_LEVEL));

    Response::for_request(&req.clone(), req.session.as_deref().unwrap_or("none"))
        .field("ops", Bencode::Dict(ops))
        .field("versions", Bencode::Dict(versions))
        .status(&["done"])
        .build()
}
