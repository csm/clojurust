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
//! The async `put` and `take` methods use `tokio::sync::Notify` for wakeups:
//! a waiting task parks until a producer/consumer fires the relevant `Notify`,
//! so no spinning or busy-polling occurs on the executor.

use std::any::Any;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{NativeObject, NativeObjectBox, Value, gc_native_object};
use tokio::sync::Notify;

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
    /// Fires when a value is added to the queue or the channel closes.
    /// Wakes blocking takers waiting for data.
    not_empty: Condvar,
    /// Fires when a value is removed from the queue or the channel closes.
    /// Wakes blocking putters waiting for space, and rendezvous putters waiting
    /// for their offered value to be consumed.
    not_full: Condvar,
    /// Async wakeup for `take`: fired when an item is enqueued or the channel closes.
    async_not_empty: Notify,
    /// Async wakeup for `put`: fired when an item is dequeued or the channel closes.
    async_not_full: Notify,
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
            not_empty: Condvar::new(),
            not_full: Condvar::new(),
            async_not_empty: Notify::new(),
            async_not_full: Notify::new(),
        }
    }

    /// True for an unbuffered (rendezvous) channel.
    pub(crate) fn is_rendezvous(&self) -> bool {
        self.state.lock().unwrap().capacity == 0
    }

    /// Mark the channel closed. Idempotent. Pending and future takes drain any
    /// buffered values and then observe `nil`.
    pub fn close(&self) {
        self.state.lock().unwrap().closed = true;
        // Wake blocking (sync) callers.
        self.not_empty.notify_all();
        self.not_full.notify_all();
        // Wake one async taker/putter; they cascade the notification on exit.
        self.async_not_empty.notify_one();
        self.async_not_full.notify_one();
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
            drop(st);
            self.not_full.notify_all();
            self.async_not_full.notify_one();
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
            drop(st);
            self.not_empty.notify_all();
            self.async_not_empty.notify_one();
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
            drop(st);
            self.not_empty.notify_all();
            self.async_not_empty.notify_one();
            RvOffer::Offered(token)
        } else {
            RvOffer::Full
        }
    }

    /// Blocking take for synchronous (non-async) callers.
    ///
    /// Parks the calling OS thread on a `Condvar` until a value is available,
    /// the channel is closed, or the 1 ms poll interval elapses (so the caller
    /// remains responsive even when invoked from the LocalSet executor thread).
    /// Returns `Value::Nil` on a closed, drained channel.
    pub fn take_blocking(&self) -> Value {
        let mut guard = self.state.lock().unwrap();
        loop {
            if let Some(v) = guard.queue.pop_front() {
                guard.taken = guard.taken.wrapping_add(1);
                drop(guard);
                self.not_full.notify_all();
                return v;
            }
            if guard.closed {
                return Value::Nil;
            }
            // 1 ms timeout keeps the loop responsive on the LocalSet thread where
            // notify_all may never fire (async producers can't run while blocked).
            let (g, _) = self
                .not_empty
                .wait_timeout(guard, Duration::from_millis(1))
                .unwrap();
            guard = g;
        }
    }

    /// Blocking put for synchronous (non-async) callers.
    ///
    /// Parks the calling OS thread until the value is accepted (buffered or
    /// handed off in a rendezvous) or the channel is closed.
    /// Returns `true` on success, `false` if the channel was closed.
    pub fn put_blocking(&self, v: Value) -> bool {
        let mut guard = self.state.lock().unwrap();
        let cap = guard.capacity;

        if cap == 0 {
            // Rendezvous: phase 1 — claim the single slot.
            let token = loop {
                if guard.closed {
                    return false;
                }
                if guard.queue.is_empty() {
                    let t = guard.taken;
                    guard.queue.push_back(v.clone());
                    drop(guard);
                    self.not_empty.notify_all();
                    break t;
                }
                let (g, _) = self
                    .not_full
                    .wait_timeout(guard, Duration::from_millis(1))
                    .unwrap();
                guard = g;
            };
            // Phase 2 — wait for a taker to consume our offered value.
            guard = self.state.lock().unwrap();
            loop {
                if guard.taken != token {
                    return true;
                }
                if guard.closed {
                    guard.queue.pop_front(); // cancel pending offer
                    drop(guard);
                    self.not_full.notify_all();
                    return false;
                }
                let (g, _) = self
                    .not_full
                    .wait_timeout(guard, Duration::from_millis(1))
                    .unwrap();
                guard = g;
            }
        } else {
            // Buffered: wait for room.
            loop {
                if guard.closed {
                    return false;
                }
                if guard.queue.len() < cap {
                    guard.queue.push_back(v);
                    drop(guard);
                    self.not_empty.notify_all();
                    return true;
                }
                let (g, _) = self
                    .not_full
                    .wait_timeout(guard, Duration::from_millis(1))
                    .unwrap();
                guard = g;
            }
        }
    }

    /// Asynchronously take a value from the channel, cooperatively yielding to
    /// the `LocalSet` executor until a value is available or the channel closes.
    /// Returns `Value::Nil` when the channel is closed and drained.
    ///
    /// This is the async counterpart to [`Self::take_blocking`] and the
    /// building block other native crates use to consume channel values.
    /// Must run within a Tokio `LocalSet` context.
    pub async fn take(&self) -> Value {
        loop {
            if let Some(v) = self.try_take() {
                if matches!(v, Value::Nil) {
                    // Channel closed: cascade so other parked takers wake too.
                    self.async_not_empty.notify_one();
                }
                return v;
            }
            self.async_not_empty.notified().await;
        }
    }

    /// Asynchronously put `v` onto the channel, cooperatively yielding to the
    /// `LocalSet` executor until the value is accepted — buffered (capacity >= 1)
    /// or handed off to a taker (rendezvous). Resolves `true` on success, or
    /// `false` if the channel is (or becomes) closed before the value lands.
    ///
    /// This is the async counterpart to [`Self::put_blocking`] and the building
    /// block other native crates use to stream produced values onto a channel.
    /// Must run within a Tokio `LocalSet` context.
    pub async fn put(&self, v: Value) -> bool {
        if self.is_rendezvous() {
            // Phase 1: claim the channel's single slot.
            let token = loop {
                match self.rv_offer(&v) {
                    RvOffer::Offered(t) => break t,
                    RvOffer::Closed => return false,
                    RvOffer::Full => {}
                }
                self.async_not_full.notified().await;
            };
            // Phase 2: wait for a taker to consume the offered value.
            loop {
                match self.rv_status(token) {
                    RvStatus::Taken => return true,
                    RvStatus::ClosedUntaken => return false,
                    RvStatus::Waiting => {}
                }
                self.async_not_full.notified().await;
            }
        } else {
            loop {
                if let Some(accepted) = self.try_put_buffered(&v) {
                    return accepted;
                }
                self.async_not_full.notified().await;
            }
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

// ── GcPtr<NativeObjectBox> helpers for channel consumers ─────────────────────

/// Create a fresh `CljChannel` wrapped as a `Value::NativeObject`-ready pointer.
pub fn make_chan(capacity: usize) -> GcPtr<NativeObjectBox> {
    gc_native_object(CljChannel::new(capacity))
}

/// Borrow the `CljChannel` out of a `NativeObjectBox`.
///
/// Panics if `obj` does not hold a `CljChannel`; that is always the case for
/// objects created by [`make_chan`] or the `(chan)` builtin.
pub fn chan_ref(obj: &NativeObjectBox) -> &CljChannel {
    obj.downcast_ref::<CljChannel>()
        .expect("NativeObjectBox holds a CljChannel")
}

/// Asynchronously put `v` onto `ch`, yielding until accepted.
/// Returns `false` if the channel is closed before the value lands.
pub async fn chan_put(ch: &GcPtr<NativeObjectBox>, v: Value) -> bool {
    chan_ref(ch.get()).put(v).await
}

/// Deliver exactly one value on a promise (capacity-1) channel, then close it.
pub async fn chan_deliver(ch: &GcPtr<NativeObjectBox>, v: Value) {
    let _ = chan_ref(ch.get()).put(v).await;
    chan_ref(ch.get()).close();
}

/// Asynchronously take a value from `ch`, yielding until one is available.
/// Returns `Value::Nil` when the channel is closed and drained.
pub async fn chan_take(ch: &GcPtr<NativeObjectBox>) -> Value {
    chan_ref(ch.get()).take().await
}
