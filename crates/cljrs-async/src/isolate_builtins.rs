//! Clojure-level surface for the cross-isolate copy boundary (Phase B2).
//!
//! These builtins expose the [`crate::isolate_channel`] copy boundary to Clojure
//! as a **distinct constructor**, following the source-visibility rule from
//! `docs/isolate-boundary-plan.md`:
//!
//! > **Distinct-at-construction, not per-message.**
//!
//! A value sent through an *isolate* channel is deep-copied across the boundary;
//! the programmer knows a copy happens because the thing they are holding is an
//! isolate-channel end (`(isolate-chan)`), not an in-isolate `(chan)`. The copy
//! is metered into `GC_STATS` (see `IsolateSender::send`) and a value that cannot
//! cross raises a **located** error at the `isolate-put!` site.
//!
//! ## API
//!
//! - `(isolate-chan)` → `[tx rx]` — a sender/receiver pair. The sender is
//!   cloneable (multi-producer); the receiver is single-consumer and must be
//!   used from the isolate that will deserialize into its heap.
//! - `(isolate-put! tx v)` → `true` once `v` is copied and enqueued; `false` if
//!   the receiver is gone. Throws if `v` holds isolate-local state (a closure,
//!   atom, native resource, …) that cannot cross.
//! - `(isolate-poll! rx)` → the next value (independently deep-copied into this
//!   isolate's heap), or `nil` if the channel is empty or closed. Never parks.
//! - `(isolate-take! rx)` → a `Future` resolving to the next value, or `nil`
//!   once the channel is closed and drained. Use with `await` in an async body.
//!
//! Until a Clojure-level isolate-spawn primitive exists (deferred with the
//! `pfuture`/`spawn` parallel primitive), both ends typically live on the same
//! isolate, so the channel behaves as an unbounded queue that still pays — and
//! meters — the deep copy. That makes the boundary observable from Clojure today.

use std::any::Any;
use std::sync::{Arc, Mutex};

use cljrs_env::env::GlobalEnv;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_value::clone::CloneError;
use cljrs_value::{
    Arity, NativeFn, NativeObject, NativeObjectBox, PersistentVector, Value, ValueError,
    ValueResult, gc_native_object,
};

use crate::eval_async::spawn_future;
use crate::isolate_channel::{IsolateReceiver, IsolateRecv, IsolateSender, isolate_channel};

/// Native type tag for the sending end of a cross-isolate channel.
pub(crate) const ISOLATE_TX_TAG: &str = "IsolateSender";
/// Native type tag for the receiving end of a cross-isolate channel.
pub(crate) const ISOLATE_RX_TAG: &str = "IsolateReceiver";

/// Sending end of a cross-isolate channel, exposed as a `Value::NativeObject`.
/// `IsolateSender` holds only `Send + Sync` data (a tokio sender of serialized
/// values), so the `Trace` impl is a no-op.
#[derive(Debug)]
pub struct CljIsolateTx {
    tx: IsolateSender,
}

impl Trace for CljIsolateTx {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for CljIsolateTx {
    fn type_tag(&self) -> &str {
        ISOLATE_TX_TAG
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Receiving end of a cross-isolate channel. The receiver needs `&mut self` to
/// dequeue, so it lives behind a `Mutex` (which also makes the native object
/// `Sync`). Single-consumer: only one isolate should `isolate-take!` from it.
#[derive(Debug)]
pub struct CljIsolateRx {
    rx: Mutex<IsolateReceiver>,
}

impl Trace for CljIsolateRx {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for CljIsolateRx {
    fn type_tag(&self) -> &str {
        ISOLATE_RX_TAG
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Register the cross-isolate channel builtins into `ns`.
pub(crate) fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        ("isolate-chan", Arity::Fixed(0), builtin_isolate_chan),
        ("isolate-put!", Arity::Fixed(2), builtin_isolate_put),
        ("isolate-poll!", Arity::Fixed(1), builtin_isolate_poll),
        ("isolate-take!", Arity::Fixed(1), builtin_isolate_take),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

/// Borrow a verified isolate-sender out of the first argument.
fn tx_arg(args: &[Value]) -> ValueResult<GcPtr<NativeObjectBox>> {
    match args.first() {
        Some(Value::NativeObject(obj)) if obj.get().type_tag() == ISOLATE_TX_TAG => Ok(obj.clone()),
        other => Err(ValueError::WrongType {
            expected: "isolate-channel sender",
            got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
        }),
    }
}

/// Borrow a verified isolate-receiver out of the first argument.
fn rx_arg(args: &[Value]) -> ValueResult<GcPtr<NativeObjectBox>> {
    match args.first() {
        Some(Value::NativeObject(obj)) if obj.get().type_tag() == ISOLATE_RX_TAG => Ok(obj.clone()),
        other => Err(ValueError::WrongType {
            expected: "isolate-channel receiver",
            got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
        }),
    }
}

fn tx_ref(obj: &NativeObjectBox) -> &CljIsolateTx {
    obj.downcast_ref::<CljIsolateTx>()
        .expect("isolate-sender native object holds a CljIsolateTx")
}

fn rx_ref(obj: &NativeObjectBox) -> &CljIsolateRx {
    obj.downcast_ref::<CljIsolateRx>()
        .expect("isolate-receiver native object holds a CljIsolateRx")
}

/// `(isolate-chan)` — create a cross-isolate channel and return `[tx rx]`.
fn builtin_isolate_chan(_args: &[Value]) -> ValueResult<Value> {
    let (tx, rx) = isolate_channel();
    let tx_obj = Value::NativeObject(gc_native_object(CljIsolateTx { tx }));
    let rx_obj = Value::NativeObject(gc_native_object(CljIsolateRx { rx: Mutex::new(rx) }));
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter([
        tx_obj, rx_obj,
    ]))))
}

/// `(isolate-put! tx v)` — deep-copy `v` across the boundary and enqueue it.
/// Returns `true` on success, `false` if the receiver has been dropped, and
/// raises a **located** error if `v` cannot cross (the send site is the error
/// site, per the isolate-boundary plan).
fn builtin_isolate_put(args: &[Value]) -> ValueResult<Value> {
    let tx = tx_arg(args)?;
    let val = args.get(1).cloned().unwrap_or(Value::Nil);
    match tx_ref(tx.get()).tx.send(&val) {
        Ok(()) => Ok(Value::Bool(true)),
        Err(CloneError::Disconnected) => Ok(Value::Bool(false)),
        Err(e @ CloneError::NotShareable { .. }) => Err(ValueError::Other(format!(
            "isolate-put!: {e}; the value holds isolate-local state and cannot \
             cross an isolate boundary"
        ))),
    }
}

/// `(isolate-poll! rx)` — non-blocking take. Returns the next value (copied into
/// this isolate's heap) or `nil` when the channel is empty or closed.
fn builtin_isolate_poll(args: &[Value]) -> ValueResult<Value> {
    let rx = rx_arg(args)?;
    let mut guard = rx_ref(rx.get())
        .rx
        .lock()
        .expect("isolate-receiver mutex poisoned");
    Ok(match guard.try_recv_status() {
        IsolateRecv::Value(v) => v,
        IsolateRecv::Empty | IsolateRecv::Disconnected => Value::Nil,
    })
}

/// `(isolate-take! rx)` — a `Future` resolving to the next value, or `nil` once
/// the channel is closed and drained. Parks (yields) while the channel is empty.
fn builtin_isolate_take(args: &[Value]) -> ValueResult<Value> {
    let rx = rx_arg(args)?;
    Ok(spawn_future(async move {
        loop {
            let status = {
                let mut guard = rx_ref(rx.get())
                    .rx
                    .lock()
                    .expect("isolate-receiver mutex poisoned");
                guard.try_recv_status()
            };
            match status {
                IsolateRecv::Value(v) => return Ok(v),
                IsolateRecv::Disconnected => return Ok(Value::Nil),
                IsolateRecv::Empty => tokio::task::yield_now().await,
            }
        }
    }))
}
