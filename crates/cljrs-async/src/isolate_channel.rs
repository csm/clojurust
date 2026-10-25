//! Cross-isolate channel — the copy boundary for Phase B2.
//!
//! A cross-isolate channel transfers `Value`s between two isolates by
//! **structured-cloning** them at the boundary:
//!
//! 1. The sender calls `IsolateSender::send` (or `send_async`), which calls
//!    `serialize` to produce a `SerializedValue` (a `Send + Sync` owned
//!    intermediate).
//! 2. The serialized form waits in a byte-bounded queue shared by both ends.
//! 3. The receiver calls `IsolateReceiver::recv` (async) or
//!    `IsolateReceiver::try_recv` (non-blocking), which calls `deserialize` to
//!    allocate a fresh `Value` in the *current* isolate's GC heap.
//!
//! `GcPtr` is `!Send`, so the compiler prevents accidentally sharing a pointer
//! across the boundary. The only crossing mechanism is this channel.
//!
//! Non-shareable values (`Resource`, `Atom`, mutable state, closures) are
//! rejected at `send` time with a [`CloneError`] rather than a panic.
//!
//! # Memory accounting
//!
//! Each queued message holds a process-governor charge
//! (`MemoryClass::MessageQueue`) for its [`SerializedValue::wire_size`].  The
//! charge is taken before the message is enqueued and returned after the
//! receiver deserializes it, or when the channel closes.  The channel also
//! bounds its queued bytes: [`IsolateSender::send`] returns
//! [`CloneError::Full`] when a message does not fit, and
//! [`IsolateSender::send_async`] waits for room.  The default limit is the
//! governor's `queue_limit` (`CLJRS_ISOLATE_QUEUE_LIMIT_MB`).
//!
//! # Usage
//!
//! ```rust,ignore
//! let (tx, rx) = isolate_channel();
//!
//! // Isolate A sends
//! tx.send(&Value::Long(42)).unwrap();
//!
//! // Isolate B receives (inside its LocalSet)
//! let v = rx_clone.recv().await.unwrap();
//! assert_eq!(v, Value::Long(42));
//! ```

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use cljrs_gc::{MemoryCharge, MemoryClass, governor};
use cljrs_value::Value;
use cljrs_value::clone::{CloneError, SerializedValue, deserialize, serialize};
use tokio::sync::Notify;

/// A queued message and the governor charge for its wire bytes.
struct AccountedMessage {
    value: SerializedValue,
    charge: MemoryCharge,
}

struct State {
    queue: VecDeque<AccountedMessage>,
    /// Wire bytes of queued messages plus the message being deserialized.
    queued_bytes: usize,
    /// Live `IsolateSender` handles.
    senders: usize,
    /// The receiver closed or dropped the channel.
    closed: bool,
}

struct Shared {
    state: Mutex<State>,
    limit: usize,
    /// Signalled on each enqueue and when the last sender goes away.
    recv_ready: Notify,
    /// Signalled when queued bytes fall or the channel closes.
    space_ready: Notify,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Release `bytes` of queue capacity and wake waiting senders.
    fn release(&self, bytes: usize) {
        let mut st = self.lock();
        st.queued_bytes = st.queued_bytes.saturating_sub(bytes);
        drop(st);
        self.space_ready.notify_waiters();
    }
}

/// Sending end of a cross-isolate channel. Cloneable — multiple senders are
/// allowed.
pub struct IsolateSender {
    shared: Arc<Shared>,
}

/// Receiving end of a cross-isolate channel. Must live on the destination
/// isolate's thread so that `deserialize` allocates into the right GC heap.
/// Dropping it closes the channel and releases every queued message.
pub struct IsolateReceiver {
    shared: Arc<Shared>,
}

/// Outcome of a non-blocking [`IsolateReceiver::try_recv_status`].
pub enum IsolateRecv {
    /// A value was dequeued and deserialized into the current isolate's heap.
    Value(Value),
    /// The channel is currently empty but at least one sender is still alive.
    Empty,
    /// All senders have been dropped (or the channel was closed) and the
    /// queue is drained.
    Disconnected,
}

// Both ends hold only `Arc<Shared>`, whose contents are `Send + Sync`
// (`SerializedValue` and `MemoryCharge` are), so either end may move to the
// destination thread.

/// Create a linked `(IsolateSender, IsolateReceiver)` pair whose byte limit is
/// the governor's default per-channel queue limit.
///
/// The sender may be cloned and used from any thread or isolate. The receiver
/// must be moved to the destination isolate *before* any `recv` call so that
/// deserialized `GcPtr`s are allocated in the correct heap.
pub fn isolate_channel() -> (IsolateSender, IsolateReceiver) {
    isolate_channel_with_limit(governor().config().queue_limit)
}

/// Create a channel that holds at most `limit` wire bytes of queued messages.
///
/// # Panics
/// Panics if `limit` is zero.
pub fn isolate_channel_with_limit(limit: usize) -> (IsolateSender, IsolateReceiver) {
    assert!(limit > 0, "isolate channel byte limit must be non-zero");
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            queue: VecDeque::new(),
            queued_bytes: 0,
            senders: 1,
            closed: false,
        }),
        limit,
        recv_ready: Notify::new(),
        space_ready: Notify::new(),
    });
    (
        IsolateSender {
            shared: shared.clone(),
        },
        IsolateReceiver { shared },
    )
}

impl IsolateSender {
    /// Serialize `v` and enqueue it without waiting.
    ///
    /// This is the compatibility form of the original unbounded send; it is
    /// the same operation as [`Self::try_send`].
    ///
    /// Returns `Err(CloneError::NotShareable { .. })` if `v` contains a
    /// non-shareable type (mutable state, closures, native resources),
    /// `Err(CloneError::Disconnected)` if the receiver has been dropped,
    /// `Err(CloneError::Full { .. })` if the message does not fit, and
    /// `Err(CloneError::MemoryLimitExceeded(_))` if the strict process
    /// governor refuses its queue charge.
    pub fn send(&self, v: &Value) -> Result<(), CloneError> {
        self.try_send(v)
    }

    /// Serialize `v` and enqueue it, or return `CloneError::Full` when the
    /// channel's byte limit has no room for it (`MemoryLimitExceeded` when
    /// the strict process governor refuses it).
    pub fn try_send(&self, v: &Value) -> Result<(), CloneError> {
        let sv = serialize_metered(v)?;
        self.enqueue(sv).map_err(|(_, e)| {
            if is_rejection(&e) {
                governor().record_queue_rejection();
            }
            e
        })
    }

    /// Serialize `v` and enqueue it, waiting while the channel is full.
    ///
    /// A message larger than the channel's whole limit can never fit, so it
    /// returns `CloneError::Full` at once.  A strict-governor refusal
    /// (`CloneError::MemoryLimitExceeded`) also returns at once: the
    /// governor has no capacity wakeup to wait on.
    pub async fn send_async(&self, v: &Value) -> Result<(), CloneError> {
        let mut sv = serialize_metered(v)?;
        let mut waiting_since: Option<Instant> = None;
        loop {
            // Register for the wakeup before checking, so a release between
            // the check and the await is not lost.
            let notified = self.shared.space_ready.notified();
            let mut notified = std::pin::pin!(notified);
            notified.as_mut().enable();
            match self.enqueue(sv) {
                Ok(()) => {
                    if let Some(start) = waiting_since {
                        governor().record_queue_wait(start.elapsed());
                    }
                    return Ok(());
                }
                Err((back, CloneError::Full { bytes, .. })) if bytes <= self.shared.limit => {
                    sv = back;
                    waiting_since.get_or_insert_with(Instant::now);
                    notified.await;
                }
                Err((_, e)) => {
                    if is_rejection(&e) {
                        governor().record_queue_rejection();
                    }
                    return Err(e);
                }
            }
        }
    }

    /// The channel's byte limit.
    pub fn limit(&self) -> usize {
        self.shared.limit
    }

    /// Wire bytes currently queued (including a message being received).
    pub fn queued_bytes(&self) -> usize {
        self.shared.lock().queued_bytes
    }

    /// Reserve the message's wire bytes and enqueue it.  On failure, return
    /// the message so an async sender can retry.
    fn enqueue(&self, sv: SerializedValue) -> Result<(), (SerializedValue, CloneError)> {
        let bytes = sv.wire_size();
        let mut st = self.shared.lock();
        if st.closed {
            return Err((sv, CloneError::Disconnected));
        }
        let full = CloneError::Full {
            bytes,
            queued: st.queued_bytes,
            limit: self.shared.limit,
        };
        if st.queued_bytes.saturating_add(bytes) > self.shared.limit {
            return Err((sv, full));
        }
        let charge = match governor().reserve_shared(MemoryClass::MessageQueue, bytes) {
            Ok(charge) => charge,
            Err(refused) => return Err((sv, CloneError::MemoryLimitExceeded(refused))),
        };
        st.queued_bytes += bytes;
        st.queue.push_back(AccountedMessage { value: sv, charge });
        drop(st);
        self.shared.recv_ready.notify_one();
        Ok(())
    }
}

/// A refusal for lack of room (channel limit or process governor), as
/// opposed to a disconnected or unshareable message.
fn is_rejection(e: &CloneError) -> bool {
    matches!(
        e,
        CloneError::Full { .. } | CloneError::MemoryLimitExceeded(_)
    )
}

/// Serialize `v`, metering the crossing (bytes copied + serialize time) into
/// the shared GC stats so a silent fan-out copy is observable rather than
/// mystery latency. The error path is intentionally *not* metered — a
/// rejected value never crosses, so it copies nothing.
fn serialize_metered(v: &Value) -> Result<SerializedValue, CloneError> {
    let start = Instant::now();
    let sv = serialize(v)?;
    cljrs_gc::GC_STATS.record_boundary_crossing(sv.byte_size() as u64, start.elapsed());
    Ok(sv)
}

impl Clone for IsolateSender {
    fn clone(&self) -> Self {
        self.shared.lock().senders += 1;
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl Drop for IsolateSender {
    fn drop(&mut self) {
        let mut st = self.shared.lock();
        st.senders -= 1;
        let last = st.senders == 0;
        drop(st);
        if last {
            self.shared.recv_ready.notify_one();
        }
    }
}

impl fmt::Debug for IsolateSender {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolateSender")
            .field("limit", &self.shared.limit)
            .field("queued_bytes", &self.queued_bytes())
            .finish()
    }
}

impl IsolateReceiver {
    /// Asynchronously receive the next value, deserializing it into the current
    /// isolate's GC heap. Returns `None` when all senders have been dropped.
    pub async fn recv(&mut self) -> Option<Value> {
        loop {
            match self.try_recv_status() {
                IsolateRecv::Value(v) => return Some(v),
                IsolateRecv::Disconnected => return None,
                // `notify_one` stores a permit when no task waits, so a send
                // between the check and this await still wakes us.
                IsolateRecv::Empty => self.shared.recv_ready.notified().await,
            }
        }
    }

    /// Non-blocking receive attempt. Returns `None` if the channel is currently
    /// empty or all senders have been dropped.
    pub fn try_recv(&mut self) -> Option<Value> {
        match self.try_recv_status() {
            IsolateRecv::Value(v) => Some(v),
            IsolateRecv::Empty | IsolateRecv::Disconnected => None,
        }
    }

    /// Non-blocking receive that distinguishes "empty" from "disconnected" —
    /// the Clojure-level `isolate-take!`/`isolate-poll!` builtins need to tell a
    /// transient empty queue (keep waiting) from a closed channel (yield `nil`).
    pub fn try_recv_status(&mut self) -> IsolateRecv {
        let msg = {
            let mut st = self.shared.lock();
            match st.queue.pop_front() {
                Some(msg) => msg,
                None if st.closed || st.senders == 0 => return IsolateRecv::Disconnected,
                None => return IsolateRecv::Empty,
            }
        };
        // The charge and the queue capacity stay held until deserialization
        // finishes (or unwinds): the wire form and the new heap objects
        // coexist until then.
        let _release = Dequeued {
            shared: &self.shared,
            bytes: msg.charge.bytes(),
            charge: Some(msg.charge),
        };
        IsolateRecv::Value(deserialize(msg.value))
    }

    /// Close the channel: drop every queued message (returning its charge)
    /// and make later sends fail with `CloneError::Disconnected`.
    pub fn close(&mut self) {
        let drained = {
            let mut st = self.shared.lock();
            st.closed = true;
            let drained = std::mem::take(&mut st.queue);
            let bytes: usize = drained.iter().map(|m| m.charge.bytes()).sum();
            st.queued_bytes = st.queued_bytes.saturating_sub(bytes);
            drained
        };
        // Drop outside the lock: each charge returns its bytes to the governor.
        drop(drained);
        self.shared.space_ready.notify_waiters();
    }

    /// The channel's byte limit.
    pub fn limit(&self) -> usize {
        self.shared.limit
    }

    /// Wire bytes currently queued.
    pub fn queued_bytes(&self) -> usize {
        self.shared.lock().queued_bytes
    }
}

impl Drop for IsolateReceiver {
    fn drop(&mut self) {
        self.close();
    }
}

impl fmt::Debug for IsolateReceiver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IsolateReceiver")
            .field("limit", &self.shared.limit)
            .field("queued_bytes", &self.queued_bytes())
            .finish()
    }
}

/// Returns a dequeued message's charge and queue capacity on drop.
struct Dequeued<'a> {
    shared: &'a Shared,
    bytes: usize,
    charge: Option<MemoryCharge>,
}

impl Drop for Dequeued<'_> {
    fn drop(&mut self) {
        drop(self.charge.take());
        self.shared.release(self.bytes);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::isolate::Isolate;
    use cljrs_gc::GcPtr;
    use cljrs_value::{Keyword, MapValue, PersistentList, PersistentVector};
    use std::sync::Arc;

    // Values are !Send (GcPtr contains NonNull).  All value creation must happen
    // *inside* an Isolate::spawn future body, not in a captured outer variable.
    // Assertions are also done inside the receiving isolate; panics propagate via
    // JoinHandle::join().unwrap().

    #[test]
    fn nil_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("nil-sender").spawn(move || async move {
            tx.send(&Value::Nil).unwrap();
        });
        let hr = Isolate::new("nil-receiver").spawn(move || async move {
            let mut rx = rx;
            assert_eq!(rx.recv().await.unwrap(), Value::Nil);
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn long_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("long-sender").spawn(move || async move {
            tx.send(&Value::Long(123)).unwrap();
        });
        let hr = Isolate::new("long-receiver").spawn(move || async move {
            let mut rx = rx;
            assert_eq!(rx.recv().await.unwrap(), Value::Long(123));
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn string_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("str-sender").spawn(move || async move {
            tx.send(&Value::string("hello across isolates")).unwrap();
        });
        let hr = Isolate::new("str-receiver").spawn(move || async move {
            let mut rx = rx;
            assert_eq!(
                rx.recv().await.unwrap(),
                Value::string("hello across isolates")
            );
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn keyword_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("kw-sender").spawn(move || async move {
            tx.send(&Value::keyword(Keyword::qualified("clojure.core", "map")))
                .unwrap();
        });
        let hr = Isolate::new("kw-receiver").spawn(move || async move {
            let mut rx = rx;
            assert_eq!(
                rx.recv().await.unwrap(),
                Value::keyword(Keyword::qualified("clojure.core", "map"))
            );
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn vector_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("vec-sender").spawn(move || async move {
            let v = Value::Vector(GcPtr::new(PersistentVector::from_iter([
                Value::Long(1),
                Value::Long(2),
                Value::string("three"),
            ])));
            tx.send(&v).unwrap();
        });
        let hr = Isolate::new("vec-receiver").spawn(move || async move {
            let mut rx = rx;
            let got = rx.recv().await.unwrap();
            let expected = Value::Vector(GcPtr::new(PersistentVector::from_iter([
                Value::Long(1),
                Value::Long(2),
                Value::string("three"),
            ])));
            assert_eq!(got, expected);
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn map_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("map-sender").spawn(move || async move {
            let v = Value::Map(MapValue::from_pairs(vec![
                (Value::keyword(Keyword::simple("a")), Value::Long(1)),
                (Value::keyword(Keyword::simple("b")), Value::string("two")),
            ]));
            tx.send(&v).unwrap();
        });
        let hr = Isolate::new("map-receiver").spawn(move || async move {
            let mut rx = rx;
            let got = rx.recv().await.unwrap();
            let expected = Value::Map(MapValue::from_pairs(vec![
                (Value::keyword(Keyword::simple("a")), Value::Long(1)),
                (Value::keyword(Keyword::simple("b")), Value::string("two")),
            ]));
            assert_eq!(got, expected);
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn list_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("list-sender").spawn(move || async move {
            let v = Value::List(GcPtr::new(PersistentList::from_iter([
                Value::Bool(true),
                Value::Nil,
                Value::Long(-7),
            ])));
            tx.send(&v).unwrap();
        });
        let hr = Isolate::new("list-receiver").spawn(move || async move {
            let mut rx = rx;
            let got = rx.recv().await.unwrap();
            let expected = Value::List(GcPtr::new(PersistentList::from_iter([
                Value::Bool(true),
                Value::Nil,
                Value::Long(-7),
            ])));
            assert_eq!(got, expected);
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    #[test]
    fn resource_rejected_at_boundary() {
        let (tx, _rx) = isolate_channel();
        let h = Isolate::new("resource-sender").spawn(move || async move {
            use cljrs_value::resource::ResourceHandle;
            use std::any::Any;
            #[derive(Debug)]
            struct FakeResource;
            impl cljrs_value::resource::Resource for FakeResource {
                fn resource_type(&self) -> &'static str {
                    "fake"
                }
                fn close(&self) -> cljrs_value::ValueResult<()> {
                    Ok(())
                }
                fn is_closed(&self) -> bool {
                    false
                }
                fn as_any(&self) -> &dyn Any {
                    self
                }
            }
            let r = Value::Resource(ResourceHandle(Arc::new(FakeResource)));
            assert!(matches!(tx.send(&r), Err(CloneError::NotShareable { .. })));
        });
        h.join().unwrap();
    }

    #[test]
    fn atom_rejected_at_boundary() {
        let (tx, _rx) = isolate_channel();
        let h = Isolate::new("atom-sender").spawn(move || async move {
            let a = Value::Atom(GcPtr::new(cljrs_value::Atom::new(Value::Nil)));
            assert!(matches!(tx.send(&a), Err(CloneError::NotShareable { .. })));
        });
        h.join().unwrap();
    }

    /// A var `def`'d in one isolate is observable by value from another after
    /// the structured-clone boundary (issue #171). Keyword identity preserved.
    #[test]
    fn var_with_value_root_crosses_boundary() {
        let (tx, rx) = isolate_channel();
        let hs = Isolate::new("var-sender").spawn(move || async move {
            let var = cljrs_value::Var::new("user", "answer");
            var.bind(Value::keyword(Keyword::qualified("ns", "kw")));
            tx.send(&Value::Var(GcPtr::new(var))).unwrap();
        });
        let hr = Isolate::new("var-receiver").spawn(move || async move {
            let mut rx = rx;
            let Value::Var(p) = rx.recv().await.unwrap() else {
                panic!("expected a Var on the receiving side");
            };
            assert_eq!(p.get().namespace.as_ref(), "user");
            assert_eq!(
                p.get().deref(),
                Some(Value::keyword(Keyword::qualified("ns", "kw")))
            );
        });
        hs.join().unwrap();
        hr.join().unwrap();
    }

    /// A var bound to a closure / native fn is explicitly isolate-local: it is
    /// rejected at the boundary rather than silently crossing unbound.
    #[test]
    fn var_with_fn_root_rejected_at_boundary() {
        let (tx, _rx) = isolate_channel();
        let h = Isolate::new("var-fn-sender").spawn(move || async move {
            let var = cljrs_value::Var::new("user", "f");
            var.bind(Value::NativeFunction(GcPtr::new(
                cljrs_value::NativeFn::new("f", cljrs_value::Arity::Fixed(0), |_| Ok(Value::Nil)),
            )));
            assert!(matches!(
                tx.send(&Value::Var(GcPtr::new(var))),
                Err(CloneError::NotShareable { type_name: "var" })
            ));
        });
        h.join().unwrap();
    }

    /// Sending a value across the boundary meters bytes + crossings into the
    /// shared GC stats. Other tests may also increment the global counters
    /// concurrently, so we assert monotonic *increase*, not exact totals.
    #[test]
    fn send_meters_the_crossing() {
        let (tx, _rx) = isolate_channel();
        let h = Isolate::new("meter-sender").spawn(move || async move {
            let before = cljrs_gc::GC_STATS.snapshot();
            let v = Value::Vector(GcPtr::new(PersistentVector::from_iter([
                Value::string("a metered payload"),
                Value::Long(7),
            ])));
            tx.send(&v).unwrap();
            let after = cljrs_gc::GC_STATS.snapshot();
            assert!(after.boundary_crossings > before.boundary_crossings);
            assert!(after.boundary_bytes_copied > before.boundary_bytes_copied);
        });
        h.join().unwrap();
    }

    fn run_local<F: std::future::Future>(f: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, f)
    }

    fn payload(n: usize) -> Value {
        Value::string("x".repeat(n))
    }

    #[test]
    fn bounded_channel_rejects_and_recovers() {
        let one = serialize(&payload(100)).unwrap().wire_size();
        let (tx, mut rx) = isolate_channel_with_limit(one * 2);
        tx.send(&payload(100)).unwrap();
        tx.send(&payload(100)).unwrap();
        assert_eq!(tx.queued_bytes(), one * 2);
        assert!(
            cljrs_gc::governor().class_bytes(MemoryClass::MessageQueue) >= one * 2,
            "queued messages are charged"
        );
        let full = tx.send(&payload(100));
        assert!(
            matches!(
                full,
                Err(CloneError::Full { bytes, queued, limit })
                    if (bytes, queued, limit) == (one, one * 2, one * 2)
            ),
            "the third message must be refused as Full"
        );
        assert_eq!(rx.try_recv(), Some(payload(100)));
        assert_eq!(rx.queued_bytes(), one, "receive returns capacity");
        tx.send(&payload(100)).unwrap();
    }

    #[test]
    fn oversized_message_is_full_even_for_async_send() {
        let (tx, _rx) = isolate_channel_with_limit(64);
        let err = run_local(async { tx.send_async(&payload(1000)).await }).unwrap_err();
        assert!(matches!(err, CloneError::Full { .. }));
        assert_eq!(tx.queued_bytes(), 0);
    }

    #[test]
    fn dropping_receiver_releases_queued_messages() {
        let (tx, rx) = isolate_channel_with_limit(1 << 20);
        for _ in 0..10 {
            tx.send(&payload(1000)).unwrap();
        }
        assert!(tx.queued_bytes() > 10_000);
        drop(rx);
        assert_eq!(tx.queued_bytes(), 0);
        assert!(matches!(
            tx.send(&Value::Nil),
            Err(CloneError::Disconnected)
        ));
    }

    #[test]
    fn close_releases_queued_messages() {
        let (tx, mut rx) = isolate_channel_with_limit(1 << 20);
        tx.send(&payload(10)).unwrap();
        rx.close();
        assert_eq!(rx.queued_bytes(), 0);
        assert!(matches!(rx.try_recv_status(), IsolateRecv::Disconnected));
        assert!(matches!(
            tx.send(&Value::Nil),
            Err(CloneError::Disconnected)
        ));
    }

    #[test]
    fn async_send_waits_for_capacity() {
        let one = serialize(&payload(100)).unwrap().wire_size();
        let (tx, mut rx) = isolate_channel_with_limit(one);
        let waits_before = cljrs_gc::governor().snapshot().queue_waits;
        let got = run_local(async move {
            let sender = tokio::task::spawn_local(async move {
                for _ in 0..3 {
                    tx.send_async(&payload(100)).await.unwrap();
                }
            });
            let mut got = Vec::new();
            while let Some(v) = rx.recv().await {
                got.push(v);
            }
            sender.await.unwrap();
            got
        });
        assert_eq!(got, vec![payload(100), payload(100), payload(100)]);
        assert!(cljrs_gc::governor().snapshot().queue_waits > waits_before);
    }

    #[test]
    fn last_sender_drop_disconnects() {
        let (tx, mut rx) = isolate_channel_with_limit(1 << 20);
        let tx2 = tx.clone();
        drop(tx);
        tx2.send(&Value::Long(1)).unwrap();
        drop(tx2);
        let got = run_local(async move { (rx.recv().await, rx.recv().await) });
        assert_eq!(got, (Some(Value::Long(1)), None));
    }

    /// Two isolates allocate independently; the receiver's heap is unaffected
    /// by the sender's heap. Verifies cross-isolate copy (not shared pointer).
    #[test]
    fn receiver_heap_is_independent() {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let (tx, rx) = isolate_channel();

        let b1 = barrier.clone();
        let hs = Isolate::new("b2-sender").spawn(move || async move {
            // Allocate 30 boxed i64s on the sender's heap.
            let _vals: Vec<_> = (0_i64..30).map(GcPtr::new).collect();
            // Send a shareable value to the receiver.
            tx.send(&Value::Long(999)).unwrap();
            b1.wait();
            // Sender heap has exactly 30 objects.
            assert_eq!(cljrs_gc::HEAP.count(), 30);
        });

        let b2 = barrier.clone();
        let hr = Isolate::new("b2-receiver").spawn(move || async move {
            let mut rx = rx;
            // Allocate 50 objects on the receiver's heap before receiving.
            let _vals: Vec<_> = (0_i64..50).map(GcPtr::new).collect();
            let v = rx.recv().await.unwrap();
            b2.wait();
            // Long is inlined (no GcPtr), so count stays at 50.
            assert_eq!(cljrs_gc::HEAP.count(), 50);
            assert_eq!(v, Value::Long(999));
        });

        hs.join().unwrap();
        hr.join().unwrap();
    }
}
