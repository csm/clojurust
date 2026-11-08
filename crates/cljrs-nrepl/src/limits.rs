//! Resource limits: job admission, request size, and per-connection output.
//!
//! Without these, a client (or a burst of editor tooling requests) could make
//! the server accept unbounded work: every request became a [`Job`] on an
//! unbounded channel and per-session queues, an incomplete frame grew its
//! connection's read buffer without limit, and replies for a client that does
//! not read piled up in an unbounded channel.
//!
//! - [`Admission`] counts every accepted job until it is dropped — queued on
//!   the job channel, waiting in a session lane, running, or parked on a
//!   fiber. The network thread refuses a request that would exceed
//!   [`Limits::max_pending_jobs`] or [`Limits::max_session_jobs`] with an
//!   `overloaded` status instead of queueing it. `describe` and `interrupt`
//!   are answered on the network thread before admission, so a client can
//!   always interrupt work even while the server is saturated.
//! - [`Limits::max_request_bytes`] bounds one request frame, complete or not.
//! - [`ReplySender`] bounds the reply bytes queued for one connection; a
//!   client that stops reading is disconnected rather than buffered without
//!   limit.
//!
//! [`Job`]: crate::Job

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tokio::sync::{mpsc, watch};

use crate::bencode::{self, Bencode};

/// Limits on the work and buffering one server accepts.
///
/// The defaults are far above what an interactive editor needs, so they only
/// bite on runaway or hostile load. A limit of `0` for a job count refuses
/// every job of that kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Accepted jobs not yet finished, across all sessions: queued for the
    /// interpreter thread, waiting behind another job of their session, or
    /// running (including evaluations parked on a fiber).
    pub max_pending_jobs: usize,
    /// Accepted, unfinished jobs for one session (requests without a session
    /// share the `default` session).
    pub max_session_jobs: usize,
    /// Largest request frame, in bytes. A connection whose buffered input
    /// reaches this without completing a frame, or that sends a larger
    /// frame, is closed.
    pub max_request_bytes: usize,
    /// Reply bytes queued for one connection but not yet written to its
    /// socket. A reply that would exceed it closes the connection, unless
    /// nothing is queued (a single reply larger than the limit still goes
    /// out to a client that is keeping up).
    pub max_output_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_pending_jobs: 1024,
            max_session_jobs: 256,
            max_request_bytes: 64 * 1024 * 1024,
            max_output_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Why [`Admission::try_admit`] refused a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// [`Limits::max_pending_jobs`] reached.
    Server,
    /// [`Limits::max_session_jobs`] reached for this session.
    Session,
}

impl Refusal {
    pub(crate) fn message(self, limits: &Limits) -> String {
        match self {
            Refusal::Server => format!(
                "nrepl: server overloaded: {} jobs already pending; request refused\n",
                limits.max_pending_jobs
            ),
            Refusal::Session => format!(
                "nrepl: session overloaded: {} jobs already pending in this session; request refused\n",
                limits.max_session_jobs
            ),
        }
    }
}

#[derive(Default)]
struct Counts {
    total: usize,
    per_session: HashMap<String, usize>,
}

/// Counter of accepted, unfinished jobs; see the module docs.
pub(crate) struct Admission {
    limits: Limits,
    counts: Mutex<Counts>,
}

impl Admission {
    pub(crate) fn new(limits: Limits) -> Arc<Self> {
        Arc::new(Admission {
            limits,
            counts: Mutex::default(),
        })
    }

    pub(crate) fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Admit one job for `session`, or refuse it if a limit is reached. The
    /// job counts until the returned ticket is dropped.
    pub(crate) fn try_admit(self: &Arc<Self>, session: &str) -> Result<Ticket, Refusal> {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        if counts.total >= self.limits.max_pending_jobs {
            return Err(Refusal::Server);
        }
        let in_session = counts.per_session.get(session).copied().unwrap_or(0);
        if in_session >= self.limits.max_session_jobs {
            return Err(Refusal::Session);
        }
        counts.total += 1;
        counts
            .per_session
            .insert(session.to_string(), in_session + 1);
        Ok(Ticket {
            admission: self.clone(),
            session: session.to_string(),
        })
    }

    /// Accepted jobs not yet finished.
    pub(crate) fn pending(&self) -> usize {
        self.counts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .total
    }

    fn release(&self, session: &str) {
        let mut counts = self.counts.lock().unwrap_or_else(PoisonError::into_inner);
        counts.total = counts.total.saturating_sub(1);
        if let Some(n) = counts.per_session.get_mut(session) {
            *n -= 1;
            if *n == 0 {
                counts.per_session.remove(session);
            }
        }
    }
}

/// One admitted job's slot. Releases it on drop, wherever the job ends:
/// finished, dropped from a queue at shutdown, or unwound.
pub(crate) struct Ticket {
    admission: Arc<Admission>,
    session: String,
}

impl Drop for Ticket {
    fn drop(&mut self) {
        self.admission.release(&self.session);
    }
}

/// A connection's reply channel, bounded by queued bytes.
///
/// Replies are encoded on the sending side, so the byte count is exact. The
/// connection's writer task subtracts each reply once it is written. When a
/// reply would push the queue past [`Limits::max_output_bytes`], the reply is
/// dropped and the connection is closed (see [`ReplySender::closed`]): a
/// silent drop would leave the client waiting for a `done` that never comes.
#[derive(Clone)]
pub(crate) struct ReplySender {
    tx: mpsc::UnboundedSender<Vec<u8>>,
    queued: Arc<AtomicUsize>,
    limit: usize,
    close: Arc<watch::Sender<bool>>,
}

/// The receiving half of a [`ReplySender`], owned by the writer task.
pub(crate) struct ReplyReceiver {
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
    queued: Arc<AtomicUsize>,
}

/// Create a connection's reply channel and the signal that closes it.
pub(crate) fn reply_channel(limit: usize) -> (ReplySender, ReplyReceiver, watch::Receiver<bool>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let queued = Arc::new(AtomicUsize::new(0));
    let (close_tx, close_rx) = watch::channel(false);
    (
        ReplySender {
            tx,
            queued: queued.clone(),
            limit,
            close: Arc::new(close_tx),
        },
        ReplyReceiver { rx, queued },
        close_rx,
    )
}

/// Error from [`ReplySender::send`]: the reply was not queued.
#[derive(Debug)]
pub(crate) struct SendError;

impl ReplySender {
    /// Queue `msg` for the client, or drop it if the connection is closed or
    /// the reply would exceed the output limit (which closes the connection).
    pub(crate) fn send(&self, msg: Bencode) -> Result<(), SendError> {
        if *self.close.borrow() {
            return Err(SendError);
        }
        let bytes = bencode::encode_to_vec(&msg);
        let len = bytes.len();
        let mut queued = self.queued.load(Ordering::Acquire);
        loop {
            if queued > 0 && queued.saturating_add(len) > self.limit {
                self.close_connection();
                return Err(SendError);
            }
            match self.queued.compare_exchange_weak(
                queued,
                queued + len,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => queued = current,
            }
        }
        if self.tx.send(bytes).is_err() {
            self.queued.fetch_sub(len, Ordering::AcqRel);
            return Err(SendError);
        }
        Ok(())
    }

    /// Close the connection: its reader and writer tasks stop, and later
    /// sends are dropped.
    pub(crate) fn close_connection(&self) {
        if !self.close.send_replace(true) {
            eprintln!(
                "nrepl: closing connection: client is not reading replies ({} bytes queued, limit {})",
                self.queued.load(Ordering::Acquire),
                self.limit
            );
        }
    }

    /// Whether the connection was closed by [`ReplySender::close_connection`].
    pub(crate) fn closed(&self) -> bool {
        *self.close.borrow()
    }
}

/// One job's view of its connection's replies. Holds the job's admission
/// [`Ticket`] and releases it just before the job's final `done` reply is
/// queued, so a client that waits for `done` before sending its next request
/// never finds the slot still taken.
#[derive(Clone)]
pub(crate) struct JobReplies {
    sender: ReplySender,
    ticket: Arc<Mutex<Option<Ticket>>>,
}

impl JobReplies {
    pub(crate) fn new(sender: ReplySender, ticket: Ticket) -> Self {
        JobReplies {
            sender,
            ticket: Arc::new(Mutex::new(Some(ticket))),
        }
    }

    /// Queue `msg`; see [`ReplySender::send`]. A `done` status releases the
    /// job's admission slot first.
    pub(crate) fn send(&self, msg: Bencode) -> Result<(), SendError> {
        if is_done(&msg) {
            drop(
                self.ticket
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take(),
            );
        }
        self.sender.send(msg)
    }
}

fn is_done(msg: &Bencode) -> bool {
    let Some(Bencode::List(statuses)) = msg.as_dict().and_then(|d| d.get(b"status".as_slice()))
    else {
        return false;
    };
    statuses.iter().any(|s| s.as_str() == Some("done"))
}

impl ReplyReceiver {
    /// Next encoded reply, or `None` once every sender is gone.
    pub(crate) async fn recv(&mut self) -> Option<Vec<u8>> {
        self.rx.recv().await
    }

    /// Mark `len` bytes as written to the socket.
    pub(crate) fn written(&self, len: usize) {
        self.queued.fetch_sub(len, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(total: usize, session: usize) -> Limits {
        Limits {
            max_pending_jobs: total,
            max_session_jobs: session,
            ..Limits::default()
        }
    }

    #[test]
    fn admission_bounds_total_and_session_jobs() {
        let admission = Admission::new(limits(3, 2));
        let a1 = admission.try_admit("a").unwrap();
        let _a2 = admission.try_admit("a").unwrap();
        assert_eq!(admission.try_admit("a").err(), Some(Refusal::Session));
        let _b1 = admission.try_admit("b").unwrap();
        assert_eq!(admission.pending(), 3);
        assert_eq!(admission.try_admit("c").err(), Some(Refusal::Server));
        drop(a1);
        assert_eq!(admission.pending(), 2);
        let _c1 = admission.try_admit("c").unwrap();
    }

    #[test]
    fn reply_sender_closes_when_output_backs_up() {
        let (tx, mut rx, close) = reply_channel(64);
        let msg = || Bencode::str("x".repeat(40));
        // One reply over the limit still goes out when nothing is queued.
        tx.send(Bencode::str("y".repeat(100))).unwrap();
        let first = rx.rx.try_recv().unwrap();
        rx.written(first.len());
        tx.send(msg()).unwrap();
        assert!(tx.send(msg()).is_err(), "second reply exceeds the limit");
        assert!(tx.closed());
        assert!(*close.borrow());
        assert!(tx.send(Bencode::str("z")).is_err(), "closed stays closed");
    }
}
