//! Cross-isolate channel — the copy boundary for Phase B2.
//!
//! A cross-isolate channel transfers `Value`s between two isolates by
//! **structured-cloning** them at the boundary:
//!
//! 1. The sender calls `IsolateSender::send`, which calls `serialize` to produce
//!    a `SerializedValue` (a `Send + Sync` owned intermediate).
//! 2. The serialized form travels over a `tokio::sync::mpsc` channel (which is
//!    `Send` when the item type is `Send`).
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

use cljrs_value::Value;
use cljrs_value::clone::{CloneError, SerializedValue, deserialize, serialize};

/// Sending end of a cross-isolate channel. Cloneable — multiple senders are
/// allowed. All methods are synchronous and cheap (just serialization + channel
/// push).
#[derive(Clone)]
pub struct IsolateSender {
    tx: tokio::sync::mpsc::UnboundedSender<SerializedValue>,
}

/// Receiving end of a cross-isolate channel. Must live on the destination
/// isolate's thread so that `deserialize` allocates into the right GC heap.
pub struct IsolateReceiver {
    rx: tokio::sync::mpsc::UnboundedReceiver<SerializedValue>,
}

// IsolateSender is Send because SerializedValue: Send.
// IsolateReceiver is also Send (tokio guarantees this when the item is Send);
// it must be moved to the destination thread before use.

/// Create a linked `(IsolateSender, IsolateReceiver)` pair.
///
/// The sender may be cloned and used from any thread or isolate. The receiver
/// must be moved to the destination isolate *before* any `recv` call so that
/// deserialized `GcPtr`s are allocated in the correct heap.
pub fn isolate_channel() -> (IsolateSender, IsolateReceiver) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (IsolateSender { tx }, IsolateReceiver { rx })
}

impl IsolateSender {
    /// Serialize `v` and enqueue it for the receiving isolate.
    ///
    /// Returns `Err(CloneError::NotShareable { .. })` if `v` contains a
    /// non-shareable type (mutable state, closures, native resources).
    /// Returns `Err(CloneError::Disconnected)` if the receiver has been dropped.
    pub fn send(&self, v: &Value) -> Result<(), CloneError> {
        let start = std::time::Instant::now();
        let sv = serialize(v)?;
        // Meter the crossing (bytes copied + serialize time) into the shared
        // GC stats so a silent fan-out copy is observable rather than mystery
        // latency. The error path above is intentionally *not* metered — a
        // rejected value never crosses, so it copies nothing.
        cljrs_gc::GC_STATS.record_boundary_crossing(sv.byte_size() as u64, start.elapsed());
        self.tx.send(sv).map_err(|_| CloneError::Disconnected)
    }
}

impl IsolateReceiver {
    /// Asynchronously receive the next value, deserializing it into the current
    /// isolate's GC heap. Returns `None` when all senders have been dropped.
    pub async fn recv(&mut self) -> Option<Value> {
        self.rx.recv().await.map(deserialize)
    }

    /// Non-blocking receive attempt. Returns `None` if the channel is currently
    /// empty or all senders have been dropped.
    pub fn try_recv(&mut self) -> Option<Value> {
        self.rx.try_recv().ok().map(deserialize)
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
            assert!(after.boundary_crossings >= before.boundary_crossings + 1);
            assert!(after.boundary_bytes_copied > before.boundary_bytes_copied);
        });
        h.join().unwrap();
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
            let _vals: Vec<_> = (0_i64..30).map(|i| GcPtr::new(i)).collect();
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
            let _vals: Vec<_> = (0_i64..50).map(|i| GcPtr::new(i)).collect();
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
