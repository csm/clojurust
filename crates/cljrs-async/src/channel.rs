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
use std::collections::BTreeMap;
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
    /// Value placed into the empty slot; the token identifies its queue entry
    /// so the putter can detect when that particular value is consumed.
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
    queue: BTreeMap<u64, Value>,
    next_value: u64,
    next_waiter: u64,
    /// Buffered capacity; `0` means unbuffered (rendezvous).
    capacity: usize,
    closed: bool,
    takes: BTreeMap<u64, Waiter>,
    puts: BTreeMap<u64, Waiter>,
    // Retain tracing links after a waiter commits, until its future is consumed.
    registrations: BTreeMap<u64, Waiter>,
    publications: Vec<Publication>,
    failure: Option<ChannelFailure>,
}
#[derive(Debug)]
enum Publication {
    Queue(u64),
    Selection(Rc<Selection>),
}
#[derive(Clone, Copy, Debug)]
pub(crate) enum ChannelFailure {
    Gas,
    Memory(cljrs_gc::MemoryLimitExceeded),
}
impl ChannelFailure {
    fn error(self) -> cljrs_value::ValueError {
        match self {
            Self::Gas => cljrs_value::ValueError::GasExhausted,
            Self::Memory(error) => cljrs_value::ValueError::MemoryLimitExceeded(error),
        }
    }
}

/// A CSP channel. See the module docs for the buffering model.
#[derive(Debug)]
pub struct CljChannel {
    state: Mutex<ChannelState>,
    #[cfg(feature = "no-gc")]
    owner: cljrs_gc::region::AllocationOwner,
}

#[derive(Debug)]
struct Selection {
    done: Cell<bool>,
    result: RefCell<Option<(Value, usize)>>,
    waker: RefCell<Option<Waker>>,
    take_order: Cell<Option<u64>>,
}
impl Selection {
    fn new() -> Rc<Self> {
        Rc::new(Self {
            done: Cell::new(false),
            result: RefCell::new(None),
            waker: RefCell::new(None),
            take_order: Cell::new(None),
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
    // None = take, Some(value) = put. Publish only after an operation commits.
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
    registrations: Vec<u64>,
    default: Option<Value>,
}
impl Future for Select<'_> {
    type Output = (Value, usize);
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        *self.selection.waker.borrow_mut() = Some(cx.waker().clone());
        if !self.selection.done.get() && self.registrations.is_empty() {
            for (index, op) in self.ops.iter().enumerate() {
                if op.channel().attempt(op.value(), &self.selection, index) {
                    break;
                }
            }
        }
        let completed = self.selection.result.borrow_mut().take();
        if let Some(result) = completed {
            self.selection.take_order.set(None);
            #[cfg(feature = "no-gc")]
            cljrs_gc::region::RegionLease::for_value(&result.0).protect_current();
            return Poll::Ready(result);
        }
        if let Some(default) = self.default.take() {
            return Poll::Ready((default, usize::MAX));
        }
        if self.registrations.is_empty() {
            let registrations = self
                .ops
                .iter()
                .enumerate()
                .map(|(index, op)| {
                    let value = op.value().cloned();
                    let mut state = op.channel().state.lock().unwrap();
                    let id = state.next_waiter;
                    state.next_waiter += 1;
                    let waiter = Waiter {
                        selection: Rc::downgrade(&self.selection),
                        index,
                        value,
                    };
                    if waiter.value.is_some() {
                        state.puts.insert(id, waiter.clone());
                    } else {
                        state.takes.insert(id, waiter.clone());
                    }
                    state.registrations.insert(id, waiter);
                    id
                })
                .collect();
            self.registrations = registrations;
        }
        Poll::Pending
    }
}
impl Drop for Select<'_> {
    fn drop(&mut self) {
        self.selection.done.set(true);
        for (op, id) in self.ops.iter().zip(&self.registrations) {
            let mut state = op.channel().state.lock().unwrap();
            state.takes.remove(id);
            state.puts.remove(id);
            state.registrations.remove(id);
        }
        // A producer may have committed while this future was parked. Return
        // an unobserved take to its original FIFO position, even after close.
        if let Some(order) = self.selection.take_order.take() {
            let (value, index) = self.selection.result.borrow_mut().take().unwrap();
            let channel = self.ops[index].channel();
            let mut state = channel.state.lock().unwrap();
            state.queue.insert(order, value);
            let wakes = state.service();
            drop(state);
            channel.publish_committed();
            wake(wakes);
        }
    }
}

/// Exactly one operation commits. An available default commits no operation.
pub(crate) async fn select(ops: Vec<SelectOp<'_>>, default: Option<Value>) -> (Value, usize) {
    Select {
        ops,
        selection: Selection::new(),
        registrations: Vec::new(),
        default,
    }
    .await
}

fn wake(wakes: Vec<Waker>) {
    for waker in wakes {
        waker.wake();
    }
}

// Ordered maps provide FIFO matching and O(log N) cancellation without
// scanning unrelated waiters. A selector cannot rendezvous with itself.
fn pop_waiter(
    waiters: &mut BTreeMap<u64, Waiter>,
    exclude: Option<&Rc<Selection>>,
) -> Option<(u64, Waiter, Rc<Selection>)> {
    loop {
        let (&id, waiter) = waiters.iter().find(|(_, waiter)| {
            !waiter.selection.upgrade().is_some_and(|peer| {
                !peer.done.get() && exclude.is_some_and(|s| Rc::ptr_eq(s, &peer))
            })
        })?;
        let peer = waiter.selection.upgrade();
        let waiter = waiters.remove(&id).unwrap();
        if let Some(peer) = peer
            && !peer.done.get()
        {
            return Some((id, waiter, peer));
        }
    }
}

impl ChannelState {
    fn value_order(&mut self) -> u64 {
        let order = self.next_value;
        self.next_value += 1;
        order
    }

    fn service(&mut self) -> Vec<Waker> {
        let mut wakes = Vec::new();
        loop {
            if !self.queue.is_empty()
                && let Some((_, waiter, peer)) = pop_waiter(&mut self.takes, None)
            {
                let (order, value) = self.queue.pop_first().unwrap();
                peer.take_order.set(Some(order));
                wakes.extend(peer.finish(value, waiter.index));
                self.publications.push(Publication::Selection(peer));
                continue;
            }
            if self.closed {
                while let Some((_, waiter, peer)) = pop_waiter(&mut self.puts, None) {
                    wakes.extend(peer.finish(Value::Bool(false), waiter.index));
                }
                while let Some((_, waiter, peer)) = pop_waiter(&mut self.takes, None) {
                    wakes.extend(peer.finish(Value::Nil, waiter.index));
                }
                break;
            }
            let Some((id, put, producer)) = pop_waiter(&mut self.puts, None) else {
                break;
            };
            if self.queue.is_empty()
                && let Some((_, take, consumer)) = pop_waiter(&mut self.takes, Some(&producer))
            {
                let order = self.value_order();
                consumer.take_order.set(Some(order));
                wakes.extend(consumer.finish(put.value.unwrap(), take.index));
                self.publications.push(Publication::Selection(consumer));
                wakes.extend(producer.finish(Value::Bool(true), put.index));
            } else if self.capacity > 0 && self.queue.len() < self.capacity {
                let order = self.value_order();
                self.queue.insert(order, put.value.unwrap());
                self.publications.push(Publication::Queue(order));
                wakes.extend(producer.finish(Value::Bool(true), put.index));
            } else {
                self.puts.insert(id, put);
                break;
            }
        }
        wakes
    }
}

impl CljChannel {
    fn publish(&self, value: Value) -> Value {
        #[cfg(feature = "no-gc")]
        {
            if self.owner.is_permanent() {
                cljrs_value::publish::publish_value(value)
            } else {
                self.owner.protect(&value);
                value
            }
        }
        #[cfg(not(feature = "no-gc"))]
        {
            cljrs_value::publish::publish_value(value)
        }
    }
    fn publish_committed(&self) {
        let pending = std::mem::take(&mut self.state.lock().unwrap().publications);
        for target in pending {
            match target {
                Publication::Queue(order) => {
                    let value = self.state.lock().unwrap().queue.get(&order).cloned();
                    if let Some(value) = value {
                        let published = self.publish(value);
                        self.state.lock().unwrap().queue.insert(order, published);
                    }
                }
                Publication::Selection(selection) => {
                    let result = selection.result.borrow().clone();
                    if let Some((value, index)) = result {
                        let published = self.publish(value);
                        *selection.result.borrow_mut() = Some((published, index));
                    }
                }
            }
        }
    }
    pub(crate) fn fail(&self, failure: ChannelFailure) {
        self.state.lock().unwrap().failure = Some(failure);
        self.close();
    }
    pub(crate) fn failure(&self) -> Option<cljrs_value::ValueError> {
        self.state
            .lock()
            .unwrap()
            .failure
            .map(ChannelFailure::error)
    }
    /// Take a value and preserve a producer's resource-limit failure.
    pub async fn take_result(&self) -> cljrs_value::ValueResult<Value> {
        let value = self.take().await;
        match self.failure() {
            Some(error) => Err(error),
            None => Ok(value),
        }
    }

    // Selection tokens are isolate-local. Commit under the state lock, then
    // wake only the tasks whose operations completed.
    fn attempt(&self, put: Option<&Value>, selection: &Rc<Selection>, index: usize) -> bool {
        let put = put.cloned();
        let mut state = self.state.lock().unwrap();
        let mut wakes = state.service();
        let result = if let Some(value) = put {
            if state.closed {
                Some(Value::Bool(false))
            } else if state.queue.is_empty()
                && let Some((_, waiter, peer)) = pop_waiter(&mut state.takes, Some(selection))
            {
                let order = state.value_order();
                peer.take_order.set(Some(order));
                wakes.extend(peer.finish(value, waiter.index));
                state.publications.push(Publication::Selection(peer));
                Some(Value::Bool(true))
            } else if state.capacity > 0 && state.queue.len() < state.capacity {
                let order = state.value_order();
                state.queue.insert(order, value);
                state.publications.push(Publication::Queue(order));
                Some(Value::Bool(true))
            } else {
                None
            }
        } else if let Some((order, value)) = state.queue.pop_first() {
            selection.take_order.set(Some(order));
            state
                .publications
                .push(Publication::Selection(selection.clone()));
            Some(value)
        } else if !state.closed
            && let Some((_, waiter, peer)) = pop_waiter(&mut state.puts, Some(selection))
        {
            let order = state.value_order();
            selection.take_order.set(Some(order));
            wakes.extend(peer.finish(Value::Bool(true), waiter.index));
            state
                .publications
                .push(Publication::Selection(selection.clone()));
            Some(waiter.value.unwrap())
        } else if state.closed {
            Some(Value::Nil)
        } else {
            None
        };
        let committed = result.is_some();
        if let Some(value) = result {
            wakes.extend(selection.finish(value, index));
        }
        // Fill newly freed buffered slots from pending puts in registration
        // order before a fresh producer can acquire them.
        wakes.extend(state.service());
        drop(state);
        self.publish_committed();
        wake(wakes);
        committed
    }

    /// Create a channel. `capacity == 0` is a rendezvous (unbuffered) channel.
    pub fn new(capacity: usize) -> Self {
        Self {
            #[cfg(feature = "no-gc")]
            owner: cljrs_gc::region::AllocationOwner::current(),
            state: Mutex::new(ChannelState {
                queue: BTreeMap::new(),
                next_value: 0,
                next_waiter: 0,
                capacity,
                closed: false,
                takes: BTreeMap::new(),
                puts: BTreeMap::new(),
                registrations: BTreeMap::new(),
                publications: Vec::new(),
                failure: None,
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
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        let wakes = state.service();
        drop(state);
        self.publish_committed();
        wake(wakes);
    }

    /// Non-blocking take.
    ///
    /// - `Some(v)` — a buffered or rendezvous-offered value was removed.
    /// - `Some(Value::Nil)` — the channel is closed and drained.
    /// - `None` — the channel is open and empty (would block).
    pub(crate) fn try_take(&self) -> Option<Value> {
        let selection = Selection::new();
        self.attempt(None, &selection, 0);
        let completed = selection.result.borrow_mut().take();
        completed.map(|(value, _)| {
            #[cfg(feature = "no-gc")]
            cljrs_gc::region::RegionLease::for_value(&value).protect_current();
            value
        })
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
            // GC builds: heap-promotion fallback (see `try_put_buffered`).
            let order = st.value_order();
            st.queue.insert(order, v.clone());
            st.publications.push(Publication::Queue(order));
            let wakes = st.service();
            drop(st);
            self.publish_committed();
            wake(wakes);
            RvOffer::Offered(order)
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
    /// A cancelled take can restore an older value ahead of this offer.
    /// Check and retract only the queue entry identified by the token.
    pub(crate) fn rv_status(&self, token: u64) -> RvStatus {
        let mut st = self.state.lock().unwrap();
        if !st.queue.contains_key(&token) {
            return RvStatus::Taken;
        }
        if st.closed {
            st.queue.remove(&token); // cancel only our still-pending offer
            return RvStatus::ClosedUntaken;
        }
        RvStatus::Waiting
    }
}

impl Trace for CljChannel {
    fn trace(&self, visitor: &mut MarkVisitor) {
        let st = self.state.lock().unwrap();
        for v in st.queue.values() {
            v.trace(visitor);
        }
        for waiter in st.registrations.values() {
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

    fn poll<F: Future + Unpin>(future: &mut F) -> Poll<F::Output> {
        Pin::new(future).poll(&mut Context::from_waker(Waker::noop()))
    }

    #[test]
    fn cancelled_committed_take_restores_the_value_in_fifo_order() {
        for capacity in [0, 1] {
            let ch = CljChannel::new(capacity);
            let mut first = Box::pin(select(vec![SelectOp::Take(&ch)], None));
            let mut second = Box::pin(select(vec![SelectOp::Take(&ch)], None));
            assert!(poll(&mut first).is_pending());
            assert!(poll(&mut second).is_pending());
            assert_eq!(ch.try_put_buffered(&Value::Long(1)), Some(true));
            assert_eq!(ch.try_put_buffered(&Value::Long(2)), Some(true));
            // Cancellation order must not change the original value order.
            drop(first);
            ch.close();
            drop(second);
            assert_eq!(ch.try_take(), Some(Value::Long(1)));
            assert_eq!(ch.try_take(), Some(Value::Long(2)));
            assert_eq!(ch.try_take(), Some(Value::Nil));
        }
    }

    #[test]
    fn cancelled_alts_take_returns_to_the_winning_channel_and_next_waiter() {
        let x = CljChannel::new(0);
        let y = CljChannel::new(0);
        let mut choice = Box::pin(select(vec![SelectOp::Take(&x), SelectOp::Take(&y)], None));
        assert!(poll(&mut choice).is_pending());
        assert_eq!(y.try_put_buffered(&Value::Long(42)), Some(true));
        let mut next = Box::pin(y.take());
        assert!(poll(&mut next).is_pending());
        drop(choice);
        assert_eq!(poll(&mut next), Poll::Ready(Value::Long(42)));
        assert_eq!(x.try_take(), None);
    }

    #[test]
    fn rendezvous_offer_close_retracts_only_its_own_value() {
        let ch = CljChannel::new(0);
        let mut take = Box::pin(ch.take());
        assert!(poll(&mut take).is_pending());
        let RvOffer::Offered(first) = ch.rv_offer(&Value::Long(1)) else {
            panic!("first offer was not accepted");
        };
        assert!(matches!(ch.rv_status(first), RvStatus::Taken));
        let RvOffer::Offered(second) = ch.rv_offer(&Value::Long(2)) else {
            panic!("second offer was not accepted");
        };
        drop(take);
        ch.close();
        assert!(matches!(ch.rv_status(second), RvStatus::ClosedUntaken));
        assert_eq!(ch.try_take(), Some(Value::Long(1)));
        assert_eq!(ch.try_take(), Some(Value::Nil));
    }

    #[test]
    fn rendezvous_offer_waits_for_its_own_value_after_a_take_is_restored() {
        let ch = CljChannel::new(0);
        let mut take = Box::pin(ch.take());
        assert!(poll(&mut take).is_pending());
        let RvOffer::Offered(first) = ch.rv_offer(&Value::Long(1)) else {
            panic!("first offer was not accepted");
        };
        assert!(matches!(ch.rv_status(first), RvStatus::Taken));
        let RvOffer::Offered(second) = ch.rv_offer(&Value::Long(2)) else {
            panic!("second offer was not accepted");
        };
        drop(take);
        assert_eq!(ch.try_take(), Some(Value::Long(1)));
        assert!(matches!(ch.rv_status(second), RvStatus::Waiting));
        assert_eq!(ch.try_take(), Some(Value::Long(2)));
        assert!(matches!(ch.rv_status(second), RvStatus::Taken));
        assert_eq!(ch.try_take(), None);
    }

    #[test]
    fn pending_buffered_puts_keep_fifo_when_polled_in_reverse_order() {
        let ch = CljChannel::new(1);
        assert_eq!(ch.try_put_buffered(&Value::Long(1)), Some(true));
        let mut first = Box::pin(ch.put(Value::Long(2)));
        let mut second = Box::pin(ch.put(Value::Long(3)));
        assert!(poll(&mut first).is_pending());
        assert!(poll(&mut second).is_pending());
        assert_eq!(ch.try_take(), Some(Value::Long(1)));
        assert!(poll(&mut second).is_pending());
        assert_eq!(ch.try_put_buffered(&Value::Long(4)), None);
        assert_eq!(ch.try_take(), Some(Value::Long(2)));
        assert_eq!(poll(&mut second), Poll::Ready(true));
        assert_eq!(poll(&mut first), Poll::Ready(true));
        assert_eq!(ch.try_take(), Some(Value::Long(3)));
    }

    #[test]
    fn transfer_wakes_only_the_selected_waiter() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        struct Counter(AtomicUsize);
        impl std::task::Wake for Counter {
            fn wake(self: Arc<Self>) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        let counter = Arc::new(Counter(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let ch = CljChannel::new(0);
        let mut takes: Vec<_> = (0..10_000).map(|_| Box::pin(ch.take())).collect();
        for take in &mut takes {
            assert!(take.as_mut().poll(&mut cx).is_pending());
        }
        assert_eq!(counter.0.load(Ordering::Relaxed), 0);
        assert_eq!(ch.try_put_buffered(&Value::Long(1)), Some(true));
        assert_eq!(counter.0.load(Ordering::Relaxed), 1);
        assert_eq!(takes[0].as_mut().poll(&mut cx), Poll::Ready(Value::Long(1)));
        drop(takes);
        let state = ch.state.lock().unwrap();
        assert!(state.takes.is_empty());
        assert!(state.registrations.is_empty());
    }

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

#[cfg(test)]
mod review_publication_tests {
    use super::*;
    #[test]
    fn failed_offers_and_losing_or_cancelled_alternatives_do_not_publish() {
        let full = CljChannel::new(1);
        assert_eq!(full.try_put_buffered(&Value::Long(0)), Some(true));
        let closed = CljChannel::new(0);
        closed.close();
        #[cfg(feature = "no-gc")]
        let _scope = cljrs_gc::alloc_ctx::ScratchGuard::new();
        let mut region = cljrs_gc::region::Region::new();
        let _region = unsafe { cljrs_gc::region::RegionGuard::new(&mut region) };
        let value = Value::ObjectArray(
            region.alloc(cljrs_value::ObjectArray(Mutex::new(vec![Value::Long(1)]))),
        );
        let poisons = cljrs_gc::stats::GC_STATS.snapshot().region_poisons;
        #[cfg(feature = "no-gc")]
        let retired =
            cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap();
        assert_eq!(full.try_put_buffered(&value), None);
        assert!(matches!(closed.rv_offer(&value), RvOffer::Closed));
        let ready = CljChannel::new(1);
        let mut selected = Box::pin(select(
            vec![
                SelectOp::Put(&full, value.clone()),
                SelectOp::Put(&ready, Value::Long(7)),
            ],
            None,
        ));
        let mut cx = Context::from_waker(Waker::noop());
        assert!(matches!(
            selected.as_mut().poll(&mut cx),
            Poll::Ready((Value::Bool(true), 1))
        ));
        drop(selected);
        let mut cancelled = Box::pin(select(vec![SelectOp::Put(&full, value)], None));
        assert!(cancelled.as_mut().poll(&mut cx).is_pending());
        drop(cancelled);
        assert_eq!(cljrs_gc::stats::GC_STATS.snapshot().region_poisons, poisons);
        #[cfg(feature = "no-gc")]
        assert_eq!(
            cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap(),
            retired
        );
    }
}
