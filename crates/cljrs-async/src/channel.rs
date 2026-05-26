//! `CljChannel` — a CSP channel exposed to Clojure as a `Value::NativeObject`.
//!
//! Channels keep the core `Value` enum free of async concerns: `(chan)` returns
//! a `Value::NativeObject` wrapping a `CljChannel`, and the channel builtins in
//! [`crate::builtins`] downcast through [`NativeObjectBox::downcast_ref`].
//!
//! A channel is a bounded FIFO queue guarded by a `Mutex`. Two capacity modes:
//!
//! - **Buffered** (`capacity >= 1`) — `put!` succeeds while the queue has room,
//!   `take!` succeeds while it is non-empty.
//! - **Rendezvous** (`capacity == 0`, the default `(chan)`) — a `put!` parks
//!   until a `take!` consumes its value, so producer and consumer hand off
//!   directly. Only one value is in flight at a time.
//!
//! The async builtins drive these via short, lock-scoped "try" steps and yield
//! the `LocalSet` executor between attempts, matching the cooperative polling
//! model used by [`crate::eval_async::await_value`].

use std::any::Any;
use std::collections::VecDeque;
use std::sync::Mutex;

use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{NativeObject, NativeObjectBox, Value};

/// The native type tag reported by [`NativeObject::type_tag`] for channels.
pub(crate) const CHANNEL_TAG: &str = "Channel";

/// The native type tag for broadcast multiplexers.
pub(crate) const MULT_TAG: &str = "Mult";

/// Outcome of a non-blocking rendezvous offer (capacity-0 `put!`).
pub(crate) enum RvOffer {
    /// Value placed into the empty slot; the carried token snapshots the
    /// take-count so the putter can detect when its value is consumed.
    Offered(u64),
    /// A value is already pending; the caller should yield and retry.
    Full,
    /// The channel is closed; the `put!` fails.
    Closed,
}

/// Status of a rendezvous offer that is waiting to be taken.
pub(crate) enum RvStatus {
    /// A taker consumed our value — `put!` resolves `true`.
    Taken,
    /// The channel closed before any taker arrived — `put!` resolves `false`.
    ClosedUntaken,
    /// Still waiting; the caller should yield and retry.
    Waiting,
}

#[derive(Debug)]
struct ChannelState {
    queue: VecDeque<Value>,
    /// Buffered capacity; `0` means unbuffered (rendezvous).
    capacity: usize,
    closed: bool,
    /// Monotonic count of successful takes, used to detect rendezvous handoff.
    taken: u64,
}

/// A CSP channel. See the module docs for the buffering model.
#[derive(Debug)]
pub struct CljChannel {
    state: Mutex<ChannelState>,
}

impl CljChannel {
    /// Create a channel. `capacity == 0` is a rendezvous (unbuffered) channel.
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(ChannelState {
                queue: VecDeque::new(),
                capacity,
                closed: false,
                taken: 0,
            }),
        }
    }

    /// True for an unbuffered (rendezvous) channel.
    pub(crate) fn is_rendezvous(&self) -> bool {
        self.state.lock().unwrap().capacity == 0
    }

    /// Mark the channel closed. Idempotent. Pending and future takes drain any
    /// buffered values and then observe `nil`.
    pub(crate) fn close(&self) {
        self.state.lock().unwrap().closed = true;
    }

    /// Non-blocking take.
    ///
    /// - `Some(v)` — a buffered or rendezvous-offered value was removed.
    /// - `Some(Value::Nil)` — the channel is closed and drained.
    /// - `None` — the channel is open and empty (would block).
    pub(crate) fn try_take(&self) -> Option<Value> {
        let mut st = self.state.lock().unwrap();
        if let Some(v) = st.queue.pop_front() {
            st.taken = st.taken.wrapping_add(1);
            return Some(v);
        }
        if st.closed {
            return Some(Value::Nil);
        }
        None
    }

    /// Non-blocking buffered put (capacity >= 1).
    ///
    /// - `Some(true)` — accepted into the buffer.
    /// - `Some(false)` — the channel is closed.
    /// - `None` — the buffer is full (would block).
    pub(crate) fn try_put_buffered(&self, v: &Value) -> Option<bool> {
        let mut st = self.state.lock().unwrap();
        if st.closed {
            return Some(false);
        }
        if st.queue.len() < st.capacity {
            st.queue.push_back(v.clone());
            return Some(true);
        }
        None
    }

    /// Offer a value into a rendezvous channel's single slot.
    pub(crate) fn rv_offer(&self, v: &Value) -> RvOffer {
        let mut st = self.state.lock().unwrap();
        if st.closed {
            return RvOffer::Closed;
        }
        if st.queue.is_empty() {
            let token = st.taken;
            st.queue.push_back(v.clone());
            RvOffer::Offered(token)
        } else {
            RvOffer::Full
        }
    }

    /// Check whether a rendezvous offer (made at `token`) has been taken.
    ///
    /// Because the slot stays full until our value is consumed, no other putter
    /// can offer in the meantime, so any increment of the take-count past
    /// `token` is our handoff.
    pub(crate) fn rv_status(&self, token: u64) -> RvStatus {
        let mut st = self.state.lock().unwrap();
        if st.taken != token {
            return RvStatus::Taken;
        }
        if st.closed {
            st.queue.pop_front(); // cancel our still-pending offer
            return RvStatus::ClosedUntaken;
        }
        RvStatus::Waiting
    }
}

impl Trace for CljChannel {
    fn trace(&self, visitor: &mut MarkVisitor) {
        let st = self.state.lock().unwrap();
        for v in st.queue.iter() {
            v.trace(visitor);
        }
    }
}

impl NativeObject for CljChannel {
    fn type_tag(&self) -> &str {
        CHANNEL_TAG
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

// ── Mult ─────────────────────────────────────────────────────────────────────

/// A broadcast multiplexer. Reads values from a source channel and forwards
/// each one to all registered tap channels. Created with `(mult source-ch)`;
/// taps are added/removed via `tap!`/`untap!`.
#[derive(Debug)]
pub struct CljMult {
    /// (tap_channel, close_on_done) pairs. `close_on_done` controls whether the
    /// tap channel is closed when the source channel closes.
    pub(crate) taps: Mutex<Vec<(GcPtr<NativeObjectBox>, bool)>>,
}

impl CljMult {
    pub fn new() -> Self {
        Self {
            taps: Mutex::new(Vec::new()),
        }
    }
}

impl Default for CljMult {
    fn default() -> Self {
        Self::new()
    }
}

impl Trace for CljMult {
    fn trace(&self, visitor: &mut MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
        for (ch, _) in self.taps.lock().unwrap().iter() {
            visitor.visit(ch);
        }
    }
}

impl NativeObject for CljMult {
    fn type_tag(&self) -> &str {
        MULT_TAG
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
