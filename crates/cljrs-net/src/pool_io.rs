//! Shared pool I/O types and async tasks for TCP and TLS.
//!
//! # Architecture
//!
//! The pool-based I/O model has two layers:
//!
//! - **Pool tasks** (`pool_reader`, `pool_writer`): run on the `WorkerPool`
//!   multi-thread runtime. They only touch `Vec<u8>`, `String`, and
//!   `mpsc`/`oneshot` channel types — **never `GcPtr` or `Value`**.
//!
//! - **LocalSet bridge tasks** (`read_bridge`, `write_bridge`): run on the
//!   `current_thread` + `LocalSet` executor. They convert between pool bytes
//!   and Clojure `Value`s, touching `GcPtr<NativeObjectBox>` and `Value` safely.
//!
//! The `PoolStreamSetup` struct is the handshake result produced on a pool thread
//! after a connection is established. It carries only `Send` data; the LocalSet
//! side reads it via a oneshot and creates Clojure channels from it.

use std::sync::Mutex;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

use cljrs_async::channel::{chan_put, chan_ref, chan_take};
use cljrs_gc::GcPtr;
use cljrs_value::{ExceptionInfo, NativeObjectBox, Value, ValueError};

// ── ReadMsg ───────────────────────────────────────────────────────────────────

/// Messages from a pool reader task to the LocalSet read-bridge.
pub(crate) enum ReadMsg {
    Data(Vec<u8>),
    Eof,
    Error(String),
}

// ── PoolStreamSetup ───────────────────────────────────────────────────────────

/// Everything the LocalSet needs to know after a pool connection is established.
///
/// Produced on a pool thread and sent via oneshot to the LocalSet bridge.
/// All fields are `Send`; no `GcPtr` or `Value` here.
pub(crate) struct PoolStreamSetup {
    pub remote_addr: String,
    pub local_addr: String,
    pub read_rx: mpsc::Receiver<ReadMsg>,
    pub write_tx: mpsc::Sender<Vec<u8>>,
    pub reader_abort: tokio::task::AbortHandle,
    pub writer_abort: tokio::task::AbortHandle,
}

pub(crate) type PoolSetupResult = Result<PoolStreamSetup, String>;

// ── Value helpers ─────────────────────────────────────────────────────────────

/// Wrap a byte slice as a `Value::ByteArray`.
pub(crate) fn bytes_value(bytes: &[u8]) -> Value {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    Value::ByteArray(GcPtr::new(Mutex::new(signed)))
}

/// Wrap an error message as a `Value::Error`.
pub(crate) fn net_error(msg: impl Into<String>) -> Value {
    let msg = msg.into();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

// ── Pool tasks (Send, no GcPtr) ───────────────────────────────────────────────

/// Runs on a pool thread: reads bytes from any `AsyncRead` and sends them as
/// `ReadMsg` messages to the LocalSet bridge.
///
/// Sends `ReadMsg::Eof` at clean EOF or `ReadMsg::Error` on I/O failure, then
/// the task exits.
pub(crate) async fn pool_reader<R>(mut reader: R, tx: mpsc::Sender<ReadMsg>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = vec![0u8; 8192];
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => {
                let _ = tx.send(ReadMsg::Eof).await;
                break;
            }
            Ok(n) => {
                let chunk = buf[..n].to_vec();
                if tx.send(ReadMsg::Data(chunk)).await.is_err() {
                    break; // bridge dropped
                }
            }
            Err(e) => {
                let _ = tx.send(ReadMsg::Error(format!("read error: {e}"))).await;
                break;
            }
        }
    }
}

/// Runs on a pool thread: receives `Vec<u8>` from the LocalSet write-bridge and
/// writes it to any `AsyncWrite`, shutting down gracefully when the sender drops.
pub(crate) async fn pool_writer<W>(mut writer: W, mut rx: mpsc::Receiver<Vec<u8>>)
where
    W: AsyncWrite + Unpin + Send + 'static,
{
    while let Some(bytes) = rx.recv().await {
        if writer.write_all(&bytes).await.is_err() {
            break;
        }
    }
    // Graceful half-close: drain (sender dropped) then shut down.
    let _ = writer.shutdown().await;
}

// ── LocalSet bridge tasks (touch GcPtr / Value) ───────────────────────────────

/// Runs on the LocalSet: bridges pool `ReadMsg` → `Value::ByteArray` →
/// puts on `in_chan`.
///
/// Closes `in_chan` when the pool reader signals EOF or an error. On error, puts
/// a `Value::Error` first so the consumer can observe it before the channel closes.
pub(crate) async fn read_bridge(mut rx: mpsc::Receiver<ReadMsg>, in_chan: GcPtr<NativeObjectBox>) {
    while let Some(msg) = rx.recv().await {
        match msg {
            ReadMsg::Data(bytes) => {
                if !chan_put(&in_chan, bytes_value(&bytes)).await {
                    break; // consumer closed :in
                }
            }
            ReadMsg::Eof => break,
            ReadMsg::Error(e) => {
                chan_put(&in_chan, net_error(e)).await;
                break;
            }
        }
    }
    chan_ref(in_chan.get()).close();
}

/// Runs on the LocalSet: takes `Value`s from `out_chan` → converts to `Vec<u8>`
/// → sends to the pool writer.
///
/// Exits when `out_chan` is closed (returns `Value::Nil`). Dropping `tx` signals
/// the pool writer to shut down.
pub(crate) async fn write_bridge(out_chan: GcPtr<NativeObjectBox>, tx: mpsc::Sender<Vec<u8>>) {
    loop {
        match chan_take(&out_chan).await {
            Value::Nil => break, // :out channel closed
            Value::ByteArray(arr) => {
                let bytes: Vec<u8> = arr.get().lock().unwrap().iter().map(|&b| b as u8).collect();
                if tx.send(bytes).await.is_err() {
                    break; // pool writer dropped
                }
            }
            Value::Str(s) => {
                let bytes = s.get().as_bytes().to_vec();
                if tx.send(bytes).await.is_err() {
                    break;
                }
            }
            _ => {} // ignore unsupported value types
        }
    }
    // Dropping `tx` here signals pool_writer to flush and shut down.
}
