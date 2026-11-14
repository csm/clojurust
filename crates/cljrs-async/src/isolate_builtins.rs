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
//! - `(isolate-chan)` / `(isolate-chan limit-bytes)` → `[tx rx]` — a
//!   sender/receiver pair. The sender is cloneable (multi-producer); the
//!   receiver is single-consumer and must be used from the isolate that will
//!   deserialize into its heap. The channel holds at most `limit-bytes` of
//!   queued wire bytes (default: `CLJRS_ISOLATE_QUEUE_LIMIT_MB`).
//! - `(isolate-put! tx v)` → `true` once `v` is copied and enqueued; `false` if
//!   the receiver is gone. Throws if `v` holds isolate-local state (a closure,
//!   atom, native resource, …) that cannot cross, or if the channel is full.
//! - `(isolate-poll! rx)` → the next value (independently deep-copied into this
//!   isolate's heap), or `nil` if the channel is empty or closed. Never parks.
//! - `(isolate-take! rx)` → a `Future` resolving to the next value, or `nil`
//!   once the channel is closed and drained. Use with `await` in an async body.
//!
//! - `(spawn-isolate 'my.ns/entry arg*)` → a `Future` of the entry function's
//!   result. Starts a new isolate (own OS thread, heap, collector, and a
//!   freshly built runtime), `require`s `my.ns` there, and calls `entry` with
//!   deep copies of the arguments. See [`builtin_spawn_isolate`].
//!
//! A channel end passed as a direct argument of `spawn-isolate` moves to the
//! new isolate instead of being copied: a sender is cloned, a receiver is
//! transferred and stops working on the isolate that gave it away.

use std::any::Any;
use std::sync::{Arc, Mutex};

use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_runtime::env::env::GlobalEnv;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_value::clone::{CloneError, SerializedValue, deserialize, serialize};
use cljrs_value::{
    Arity, NativeFn, NativeObject, NativeObjectBox, PersistentVector, Value, ValueError,
    ValueResult, gc_native_object,
};

use crate::isolate_channel::{
    IsolateReceiver, IsolateRecv, IsolateSender, isolate_channel, isolate_channel_with_limit,
};
use crate::task::{spawn_future, spawn_future_with_roots};

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
    /// `None` once `spawn-isolate` has moved the receiver to another isolate.
    rx: Mutex<Option<IsolateReceiver>>,
}

/// The error for using a receiver that `spawn-isolate` gave to another isolate.
fn moved_receiver(op: &str) -> ValueError {
    ValueError::Other(format!(
        "{op}: this isolate-channel receiver was moved to another isolate by spawn-isolate"
    ))
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
        (
            "isolate-chan",
            Arity::Variadic { min: 0 },
            builtin_isolate_chan,
        ),
        ("isolate-put!", Arity::Fixed(2), builtin_isolate_put),
        ("isolate-poll!", Arity::Fixed(1), builtin_isolate_poll),
        ("isolate-take!", Arity::Fixed(1), builtin_isolate_take),
        #[cfg(not(target_arch = "wasm32"))]
        (
            "spawn-isolate",
            Arity::Variadic { min: 1 },
            builtin_spawn_isolate,
        ),
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

/// `(isolate-chan)` / `(isolate-chan limit-bytes)` — create a cross-isolate
/// channel and return `[tx rx]`.
fn builtin_isolate_chan(args: &[Value]) -> ValueResult<Value> {
    let (tx, rx) = match args {
        [] => isolate_channel(),
        [Value::Long(n)] if *n > 0 => isolate_channel_with_limit(*n as usize),
        [Value::Long(n)] => {
            return Err(ValueError::Other(format!(
                "isolate-chan: the byte limit must be positive, got {n}"
            )));
        }
        [other] => {
            return Err(ValueError::WrongType {
                expected: "long",
                got: other.type_name().to_string(),
            });
        }
        _ => {
            return Err(ValueError::ArityError {
                name: "isolate-chan".to_string(),
                expected: "0 or 1".to_string(),
                got: args.len(),
            });
        }
    };
    let tx_obj = Value::NativeObject(gc_native_object(CljIsolateTx { tx }));
    let rx_obj = Value::NativeObject(gc_native_object(CljIsolateRx {
        rx: Mutex::new(Some(rx)),
    }));
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
        Err(e @ CloneError::Full { .. }) => Err(ValueError::Other(format!("isolate-put!: {e}"))),
        // Cancellation and resource limits: one mapping for every boundary.
        Err(
            e @ (CloneError::MemoryLimitExceeded(_)
            | CloneError::Cancelled
            | CloneError::GasExhausted),
        ) => Err(cljrs_value::clone::resource_value_error(&e).expect("resource clone error")),
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
    let receiver = guard
        .as_mut()
        .ok_or_else(|| moved_receiver("isolate-poll!"))?;
    Ok(match receiver.try_recv_status() {
        IsolateRecv::Value(v) => v,
        IsolateRecv::Empty | IsolateRecv::Disconnected => Value::Nil,
    })
}

/// `(isolate-take! rx)` — a `Future` resolving to the next value, or `nil` once
/// the channel is closed and drained. Waits without spinning while the channel
/// is empty: the task sleeps until a send or the last sender going away.
fn builtin_isolate_take(args: &[Value]) -> ValueResult<Value> {
    let rx = rx_arg(args)?;
    if rx_ref(rx.get())
        .rx
        .lock()
        .expect("isolate-receiver mutex poisoned")
        .is_none()
    {
        return Err(moved_receiver("isolate-take!"));
    }
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        loop {
            let (status, signal) = {
                let mut guard = rx_ref(rx.get())
                    .rx
                    .lock()
                    .expect("isolate-receiver mutex poisoned");
                let Some(receiver) = guard.as_mut() else {
                    return Err(EvalError::Runtime(
                        moved_receiver("isolate-take!").to_string(),
                    ));
                };
                (receiver.try_recv_status(), receiver.signal())
            };
            match status {
                IsolateRecv::Value(v) => return Ok(v),
                IsolateRecv::Disconnected => {
                    // Let any other pending take see the disconnect too.
                    signal.pass_on();
                    return Ok(Value::Nil);
                }
                IsolateRecv::Empty => signal.wait().await,
            }
        }
    }))
}

// ── spawn-isolate ────────────────────────────────────────────────────────────

/// One `spawn-isolate` argument in the form that crosses to the new thread.
#[cfg(not(target_arch = "wasm32"))]
enum ArgWire {
    /// Plain data, deep-copied.
    Value(SerializedValue),
    /// A clone of an isolate-channel sender.
    Tx(IsolateSender),
    /// An isolate-channel receiver, moved out of the spawning isolate.
    Rx(IsolateReceiver),
}

/// Why a spawned isolate produced no result.
#[cfg(not(target_arch = "wasm32"))]
enum IsolateFailure {
    /// The entry function threw a value that could be copied back.
    Thrown(SerializedValue),
    /// Anything else, as text.
    Message(String),
}

#[cfg(not(target_arch = "wasm32"))]
type IsolateOutcome = Result<SerializedValue, IsolateFailure>;

/// Stack size of a spawned isolate's OS thread. Matches the CLI's main thread
/// so code that runs on the main isolate also fits on a spawned one.
#[cfg(not(target_arch = "wasm32"))]
const ISOLATE_STACK_SIZE: usize = 64 * 1024 * 1024;

/// `(spawn-isolate 'my.ns/entry arg*)` — run `my.ns/entry` on a new isolate
/// and return a `Future` of its result.
///
/// The new isolate does not share the caller's heap or its loaded code. It
/// gets a runtime built by the host's [`IsolateFactory`], loads `my.ns` with
/// `require`, and calls the var's value with the arguments. This is why the
/// entry point is a **qualified symbol** and not a function: a closure holds
/// pointers into the caller's heap and cannot be rebuilt elsewhere, but a
/// name can be resolved again.
///
/// Arguments and the result are deep-copied, with the rules of `isolate-put!`.
/// An isolate-channel end given as a direct argument crosses as itself: a
/// sender is cloned, a receiver moves to the new isolate. A value that cannot
/// cross is an error here, before any thread starts. A result that cannot
/// cross, a throw in the entry function, and a failure to load the namespace
/// all fail the returned future.
///
/// [`IsolateFactory`]: cljrs_runtime::env::env::IsolateFactory
#[cfg(not(target_arch = "wasm32"))]
// `EvalResult`'s error is returned by value from every evaluator entry point.
#[allow(clippy::result_large_err)]
fn builtin_spawn_isolate(args: &[Value]) -> ValueResult<Value> {
    let (ns, name): (Arc<str>, Arc<str>) = match args.first().map(Value::unwrap_meta) {
        Some(Value::Symbol(sym)) => {
            let sym = sym.get();
            match (&sym.namespace, &sym.version) {
                (Some(ns), None) => (ns.clone(), sym.name.clone()),
                (None, _) => {
                    return Err(ValueError::Other(format!(
                        "spawn-isolate: the entry point must be a namespace-qualified symbol \
                         such as 'my.ns/{}; the new isolate finds the function by loading \
                         its namespace",
                        sym.name
                    )));
                }
                (Some(_), Some(_)) => {
                    return Err(ValueError::Other(
                        "spawn-isolate: a versioned symbol is not supported as an entry point"
                            .into(),
                    ));
                }
            }
        }
        Some(other) => {
            return Err(ValueError::Other(format!(
                "spawn-isolate: the entry point must be a namespace-qualified symbol, got a \
                 value of type `{}`; a function cannot cross an isolate boundary, so name \
                 it instead: (spawn-isolate 'my.ns/entry arg ...)",
                other.type_name()
            )));
        }
        None => unreachable!("spawn-isolate is registered with a minimum arity of 1"),
    };
    let factory = cljrs_runtime::env::callback::capture_eval_context()
        .and_then(|(globals, _)| globals.isolate_factory())
        .ok_or_else(|| {
            ValueError::Other(
                "spawn-isolate: this runtime cannot start isolates (its host installed no \
                 isolate factory)"
                    .into(),
            )
        })?;

    // Encode every argument before touching a receiver, so a rejected
    // argument leaves the caller's channels as they were.
    let start = web_time::Instant::now();
    let mut copied_bytes = 0u64;
    let mut wire = Vec::with_capacity(args.len() - 1);
    let mut receivers: Vec<GcPtr<NativeObjectBox>> = Vec::new();
    for (index, arg) in args[1..].iter().enumerate() {
        match arg {
            Value::NativeObject(obj) if obj.get().type_tag() == ISOLATE_TX_TAG => {
                wire.push(Some(ArgWire::Tx(tx_ref(obj.get()).tx.clone())));
            }
            Value::NativeObject(obj) if obj.get().type_tag() == ISOLATE_RX_TAG => {
                let present = rx_ref(obj.get())
                    .rx
                    .lock()
                    .expect("isolate-receiver mutex poisoned")
                    .is_some();
                let repeated = receivers.iter().any(|seen| GcPtr::ptr_eq(seen, obj));
                if !present || repeated {
                    return Err(ValueError::Other(format!(
                        "spawn-isolate: argument {} is an isolate-channel receiver that {}",
                        index + 1,
                        if repeated {
                            "is passed twice; a receiver has one consumer"
                        } else {
                            "was already moved to another isolate"
                        }
                    )));
                }
                receivers.push(obj.clone());
                wire.push(None);
            }
            other => match serialize(other) {
                Ok(sv) => {
                    copied_bytes += sv.byte_size() as u64;
                    wire.push(Some(ArgWire::Value(sv)));
                }
                Err(e @ CloneError::NotShareable { .. }) => {
                    return Err(ValueError::Other(format!(
                        "spawn-isolate: argument {}: {e}; arguments are copied to the new \
                         isolate and must be plain data or isolate-channel ends",
                        index + 1
                    )));
                }
                Err(e) => {
                    return Err(cljrs_value::clone::resource_value_error(&e)
                        .unwrap_or_else(|| ValueError::Other(format!("spawn-isolate: {e}"))));
                }
            },
        }
    }
    cljrs_gc::GC_STATS.record_boundary_crossing(copied_bytes, start.elapsed());
    let mut receivers = receivers.into_iter();
    let wire: Vec<ArgWire> = wire
        .into_iter()
        .map(|slot| {
            slot.unwrap_or_else(|| {
                let obj = receivers.next().expect("one receiver per empty slot");
                let rx = rx_ref(obj.get())
                    .rx
                    .lock()
                    .expect("isolate-receiver mutex poisoned")
                    .take()
                    .expect("receiver presence was checked above");
                ArgWire::Rx(rx)
            })
        })
        .collect();

    let (done, result) = tokio::sync::oneshot::channel::<IsolateOutcome>();
    let label = format!("{ns}/{name}");
    let entry = label.clone();
    // The join handle is dropped: the isolate is detached, and its outcome
    // arrives through `done`.
    crate::isolate::Isolate::new(format!("isolate:{label}"))
        .stack_size(ISOLATE_STACK_SIZE)
        .spawn_root(move || {
            let _ = done.send(run_isolate_entry(&factory, &ns, &name, wire));
            Ok(Value::Nil)
        });

    Ok(spawn_future(async move {
        match result.await {
            Ok(Ok(sv)) => Ok(deserialize(sv)),
            Ok(Err(IsolateFailure::Thrown(sv))) => Err(EvalError::Thrown(deserialize(sv))),
            Ok(Err(IsolateFailure::Message(message))) => Err(EvalError::Runtime(format!(
                "isolate {entry} failed: {message}"
            ))),
            Err(_) => Err(EvalError::Runtime(format!(
                "isolate {entry} ended without a result (its thread panicked)"
            ))),
        }
    }))
}

/// The body of a spawned isolate: build its runtime, load the entry
/// namespace, call the entry function, and encode the outcome for the
/// spawning isolate. Runs in the new isolate's root fiber.
#[cfg(not(target_arch = "wasm32"))]
// `EvalResult`'s error is returned by value from every evaluator entry point.
#[allow(clippy::result_large_err)]
fn run_isolate_entry(
    factory: &cljrs_runtime::env::env::IsolateFactory,
    ns: &Arc<str>,
    name: &Arc<str>,
    wire: Vec<ArgWire>,
) -> IsolateOutcome {
    use cljrs_runtime::env::env::{RequireRefer, RequireSpec};

    let message = |e: EvalError| IsolateFailure::Message(e.to_string());
    let globals = factory().map_err(|e| {
        IsolateFailure::Message(format!("could not build the isolate's runtime: {e}"))
    })?;
    let _context = cljrs_runtime::env::callback::install_eval_context_guard(
        globals.clone(),
        Arc::from("user"),
    );
    let spec = RequireSpec {
        ns: ns.clone(),
        version: None,
        alias: None,
        refer: RequireRefer::None,
    };
    cljrs_runtime::env::loader::load_ns(globals.clone(), &spec, "user").map_err(|e| {
        IsolateFailure::Message(format!(
            "could not load namespace {ns} in the new isolate ({e}); an entry point must \
             live in a namespace that `require` can load"
        ))
    })?;
    let entry = globals.lookup_in_ns(ns, name).ok_or_else(|| {
        IsolateFailure::Message(format!("namespace {ns} defines no var named {name}"))
    })?;

    // Everything allocated while decoding stays rooted by this frame until
    // the argument vector has a root of its own.
    let frame = cljrs_gc::push_alloc_frame();
    let args: Vec<Value> = wire
        .into_iter()
        .map(|arg| match arg {
            ArgWire::Value(sv) => deserialize(sv),
            ArgWire::Tx(tx) => Value::NativeObject(gc_native_object(CljIsolateTx { tx })),
            ArgWire::Rx(rx) => Value::NativeObject(gc_native_object(CljIsolateRx {
                rx: Mutex::new(Some(rx)),
            })),
        })
        .collect();
    let _arg_roots = cljrs_runtime::env::gc_roots::root_values(&args);
    drop(frame);

    let outcome: EvalResult = cljrs_runtime::env::callback::invoke(&entry, args.clone())
        .map_err(cljrs_runtime::env::error::value_error_to_eval_error)
        // An `^:async` entry point returns a future; its value is the result.
        .and_then(|value| crate::fiber::park_on(crate::task::await_value(value)));
    match outcome {
        Ok(value) => {
            let _root = cljrs_runtime::env::gc_roots::root_value(&value);
            let start = web_time::Instant::now();
            let sv = serialize(&value).map_err(|e| {
                IsolateFailure::Message(format!(
                    "the result cannot be returned to the spawning isolate: {e}"
                ))
            })?;
            cljrs_gc::GC_STATS.record_boundary_crossing(sv.byte_size() as u64, start.elapsed());
            Ok(sv)
        }
        Err(EvalError::Thrown(thrown)) => {
            let _root = cljrs_runtime::env::gc_roots::root_value(&thrown);
            Err(match serialize(&thrown) {
                Ok(sv) => IsolateFailure::Thrown(sv),
                Err(_) => IsolateFailure::Message(format!("Unhandled exception: {thrown}")),
            })
        }
        Err(e) => Err(message(e)),
    }
}
