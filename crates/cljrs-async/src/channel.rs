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
//! Takes, puts, and alternatives share selection tokens. A matching pair
//! commits both tokens before it wakes either task. Cancellation removes
//! uncommitted registrations. Parked fibers and ordinary blocking callers
//! use the same event-driven operations.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::rc::{Rc, Weak};
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::{NativeObject, NativeObjectBox, Value, gc_native_object};

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
    waiters: Vec<Waiter>,
}

/// A CSP channel. See the module docs for the buffering model.
#[derive(Debug)]
pub struct CljChannel {
    state: Mutex<ChannelState>,
}

#[derive(Debug)]
struct Selection {
    done: Cell<bool>,
    result: RefCell<Option<(Value, usize)>>,
    waker: RefCell<Option<Waker>>,
}
impl Selection {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            done: Cell::new(false),
            result: RefCell::new(None),
            waker: RefCell::new(None),
        })
    }
    fn finish(&self, value: Value, index: usize) -> Option<Waker> {
        assert!(!self.done.replace(true));
        *self.result.borrow_mut() = Some((value, index));
        self.waker.borrow_mut().take()
    }
}
#[derive(Clone, Debug)]
struct Waiter {
    selection: Weak<Selection>,
    index: usize,
    // None = take, Some(value) = put. Values are published before registration.
    value: Option<Value>,
}

/// One take or put alternative in an atomic channel selection.
pub(crate) enum SelectOp<'a> {
    Take(&'a CljChannel),
    Put(&'a CljChannel, Value),
}
impl SelectOp<'_> {
    fn channel(&self) -> &CljChannel {
        match self {
            Self::Take(ch) | Self::Put(ch, _) => ch,
        }
    }
    fn value(&self) -> Option<&Value> {
        match self {
            Self::Take(_) => None,
            Self::Put(_, value) => Some(value),
        }
    }
}

struct Select<'a> {
    ops: Vec<SelectOp<'a>>,
    selection: Rc<Selection>,
    registered: bool,
    default: Option<Value>,
}
impl Future for Select<'_> {
    type Output = (Value, usize);
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        *self.selection.waker.borrow_mut() = Some(cx.waker().clone());
        if !self.selection.done.get() {
            for (index, op) in self.ops.iter().enumerate() {
                if op.channel().attempt(op.value(), &self.selection, index) {
                    break;
                }
            }
        }
        if let Some(result) = self.selection.result.borrow_mut().take() {
            return Poll::Ready(result);
        }
        if let Some(default) = self.default.take() {
            return Poll::Ready((default, usize::MAX));
        }
        if !self.registered {
            for (index, op) in self.ops.iter().enumerate() {
                let value = op.value().cloned().map(cljrs_value::publish::publish_value);
                op.channel().state.lock().unwrap().waiters.push(Waiter {
                    selection: Rc::downgrade(&self.selection),
                    index,
                    value,
                });
                op.channel().wake_selectors();
            }
            self.registered = true;
        }
        Poll::Pending
    }
}
impl Drop for Select<'_> {
    fn drop(&mut self) {
        self.selection.done.set(true);
        if self.registered {
            let weak = Rc::downgrade(&self.selection);
            for op in &self.ops {
                op.channel()
                    .state
                    .lock()
                    .unwrap()
                    .waiters
                    .retain(|w| !w.selection.ptr_eq(&weak));
            }
        }
    }
}

/// Exactly one operation commits. An available default commits no operation.
pub(crate) async fn select(ops: Vec<SelectOp<'_>>, default: Option<Value>) -> (Value, usize) {
    Select {
        ops,
        selection: Selection::new(),
        registered: false,
        default,
    }
    .await
}

impl CljChannel {
    // Selection tokens are isolate-local. Matching two different tokens and
    // committing both happens without a yield, under the channel state lock.
    fn attempt(&self, put: Option<&Value>, selection: &Rc<Selection>, index: usize) -> bool {
        let mut state = self.state.lock().unwrap();
        let mut wakes = Vec::new();
        let result = if let Some(value) = put {
            if state.closed {
                Some(Value::Bool(false))
            } else if state.queue.is_empty()
                && let Some((peer, peer_index)) = state
                    .waiters
                    .iter()
                    .filter(|w| w.value.is_none())
                    .filter_map(|w| w.selection.upgrade().map(|peer| (peer, w.index)))
                    .find(|(peer, _)| !peer.done.get() && !Rc::ptr_eq(peer, selection))
            {
                wakes.extend(peer.finish(
                    cljrs_value::publish::publish_value(value.clone()),
                    peer_index,
                ));
                Some(Value::Bool(true))
            } else if state.capacity > 0 && state.queue.len() < state.capacity {
                state
                    .queue
                    .push_back(cljrs_value::publish::publish_value(value.clone()));
                Some(Value::Bool(true))
            } else {
                None
            }
        } else if let Some(value) = state.queue.pop_front() {
            state.taken = state.taken.wrapping_add(1);
            Some(value)
        } else if !state.closed
            && let Some((peer, peer_index, value)) = state
                .waiters
                .iter()
                .filter_map(|w| {
                    w.value.as_ref().and_then(|value| {
                        w.selection
                            .upgrade()
                            .map(|peer| (peer, w.index, value.clone()))
                    })
                })
                .find(|(peer, _, _)| !peer.done.get() && !Rc::ptr_eq(peer, selection))
        {
            wakes.extend(peer.finish(Value::Bool(true), peer_index));
            Some(value)
        } else if state.closed {
            Some(Value::Nil)
        } else {
            None
        };
        let committed = result.is_some();
        if let Some(value) = result {
            wakes.extend(selection.finish(value, index));
        }
        drop(state);
        for waker in wakes {
            waker.wake();
        }
        if committed {
            self.wake_selectors();
        }
        committed
    }

    fn wake_selectors(&self) {
        let wakers: Vec<_> = self
            .state
            .lock()
            .unwrap()
            .waiters
            .iter()
            .filter_map(|w| w.selection.upgrade())
            .filter(|s| !s.done.get())
            .filter_map(|s| s.waker.borrow().clone())
            .collect();
        for waker in wakers {
            waker.wake();
        }
    }

    /// Create a channel. `capacity == 0` is a rendezvous (unbuffered) channel.
    pub fn new(capacity: usize) -> Self {
        Self {
            state: Mutex::new(ChannelState {
                queue: VecDeque::new(),
                capacity,
                closed: false,
                taken: 0,
                waiters: Vec::new(),
            }),
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
        // Closure makes every outstanding operation ready.
        self.wake_selectors();
    }

    /// Non-blocking take.
    ///
    /// - `Some(v)` — a buffered or rendezvous-offered value was removed.
    /// - `Some(Value::Nil)` — the channel is closed and drained.
    /// - `None` — the channel is open and empty (would block).
    pub(crate) fn try_take(&self) -> Option<Value> {
        let selection = Selection::new();
        self.attempt(None, &selection, 0);
        selection.result.borrow_mut().take().map(|(value, _)| value)
    }

    /// Non-blocking buffered put (capacity >= 1).
    ///
    /// - `Some(true)` — accepted into the buffer.
    /// - `Some(false)` — the channel is closed.
    /// - `None` — the buffer is full (would block).
    pub(crate) fn try_put_buffered(&self, v: &Value) -> Option<bool> {
        let selection = Selection::new();
        self.attempt(Some(v), &selection, 0);
        selection
            .result
            .borrow_mut()
            .take()
            .map(|(value, _)| matches!(value, Value::Bool(true)))
    }

    /// Offer a value into a rendezvous channel's single slot.
    pub(crate) fn rv_offer(&self, v: &Value) -> RvOffer {
        let mut st = self.state.lock().unwrap();
        if st.closed {
            return RvOffer::Closed;
        }
        if st.queue.is_empty() {
            let token = st.taken;
            // GC builds: heap-promotion fallback (see `try_put_buffered`).
            st.queue
                .push_back(cljrs_value::publish::publish_value(v.clone()));
            drop(st);
            self.wake_selectors();
            RvOffer::Offered(token)
        } else {
            RvOffer::Full
        }
    }

    /// Take a value, parking a fiber or blocking an ordinary caller.
    pub fn take_blocking(&self) -> Value {
        cljrs_value::parking::block_on(self.take())
    }

    /// Put a value, parking a fiber or blocking an ordinary caller.
    pub fn put_blocking(&self, v: Value) -> bool {
        let _root = cljrs_runtime::env::gc_roots::root_value(&v);
        cljrs_value::parking::block_on(self.put(v.clone()))
    }

    /// Wait for a value or channel closure without blocking the executor.
    pub async fn take(&self) -> Value {
        select(vec![SelectOp::Take(self)], None).await.0
    }

    /// Wait until a put commits or the channel closes. Cancellation retracts
    /// every uncommitted offer, including rendezvous values.
    pub async fn put(&self, value: Value) -> bool {
        let _root = cljrs_runtime::env::gc_roots::root_value(&value);
        matches!(
            select(vec![SelectOp::Put(self, value.clone())], None)
                .await
                .0,
            Value::Bool(true)
        )
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
        for waiter in &st.waiters {
            if let Some(value) = &waiter.value {
                value.trace(visitor);
            }
            if let Some(selection) = waiter.selection.upgrade()
                && let Some((value, _)) = &*selection.result.borrow()
            {
                value.trace(visitor);
            }
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

#[cfg(test)]
mod notification_tests {
    use super::*;

    #[tokio::test(flavor = "current_thread")]
    async fn committed_take_result_stays_traced_until_the_receiver_is_polled() {
        let channel = make_chan(1);
        let anchor = Value::NativeObject(channel.clone());
        let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
        let ch = chan_ref(channel.get());
        let mut take = std::pin::pin!(ch.take());
        let waker = Waker::noop();
        let mut cx = Context::from_waker(waker);
        assert!(take.as_mut().poll(&mut cx).is_pending());
        let frame = cljrs_gc::push_alloc_frame();
        assert_eq!(
            ch.try_put_buffered(&Value::string("waiting result")),
            Some(true)
        );
        drop(frame);
        #[cfg(not(feature = "no-gc"))]
        for _ in 0..3 {
            cljrs_gc::request_gc();
            cljrs_runtime::env::gc_roots::async_gc_collect();
        }
        assert_eq!(format!("{}", take.await), "\"waiting result\"");
    }
}
