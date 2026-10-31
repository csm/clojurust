//! Native `clojure.core.async` builtins.
//!
//! The futures-returning primitives (`timeout`, `alts`, `take!`, `put!`) return
//! a `Value::Future` immediately and resolve on the `LocalSet` executor, so they
//! compose with `await` exactly like an `^:async` call. `close!`, `poll!`, and
//! `offer!` act on a channel synchronously and return their result directly.
//! `async-spawn` runs a thunk on the `LocalSet` as an async task (the runtime
//! backing the `go` macro).

use rand::seq::SliceRandom;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cljrs_gc::GcPtr;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::interp::destructure::value_to_seq_vec;
use cljrs_value::{
    Arity, NativeFn, NativeObjectBox, PersistentVector, Value, ValueError, ValueResult,
    gc_native_object,
};

use crate::channel::{CHANNEL_TAG, CljChannel, CljMult, MULT_TAG, RvOffer, RvStatus};
use crate::eval_async::{await_value, spawn_future_with_roots};

/// One branch of an `alts` race: awaits a future and tags it with its index.
type AltBranch = Pin<Box<dyn Future<Output = (EvalResult, usize)>>>;

/// Register the async native functions into the given namespace.
pub(crate) fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        // Phase D: timeout and alts
        ("timeout", Arity::Fixed(1), builtin_timeout),
        ("alts", Arity::Fixed(1), builtin_alts),
        ("alts!!", Arity::Variadic { min: 1 }, builtin_alts_blocking),
        // Phase E: channels
        ("chan", Arity::Variadic { min: 0 }, builtin_chan),
        ("take!", Arity::Fixed(1), builtin_take),
        ("put!", Arity::Fixed(2), builtin_put),
        ("close!", Arity::Fixed(1), builtin_close),
        ("poll!", Arity::Fixed(1), builtin_poll),
        ("offer!", Arity::Fixed(2), builtin_offer),
        ("async-spawn", Arity::Fixed(1), builtin_async_spawn),
        // Phase F: higher-level utilities
        ("join-all", Arity::Fixed(1), builtin_join_all),
        ("thread-call", Arity::Fixed(1), builtin_thread_call),
        ("onto-chan!", Arity::Fixed(2), builtin_onto_chan),
        ("to-chan!", Arity::Fixed(1), builtin_to_chan),
        ("mult", Arity::Fixed(1), builtin_mult),
        ("tap!", Arity::Variadic { min: 2 }, builtin_tap),
        ("untap!", Arity::Fixed(2), builtin_untap),
        ("untap-all!", Arity::Fixed(1), builtin_untap_all),
        // Phase H: blocking sync-context ops
        ("<!!", Arity::Fixed(1), builtin_take_blocking),
        (">!!", Arity::Fixed(2), builtin_put_blocking),
    ];
    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }
}

/// `(timeout ms)` — a Future that delivers `nil` after `ms` milliseconds.
fn builtin_timeout(args: &[Value]) -> ValueResult<Value> {
    let ms = match args.first() {
        Some(Value::Long(n)) => (*n).max(0) as u64,
        other => {
            return Err(ValueError::WrongType {
                expected: "long (timeout ms)",
                got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            });
        }
    };
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        platform_sleep(ms).await;
        Ok(Value::Nil)
    }))
}

/// Sleep for `ms` milliseconds using the best available platform timer.
#[cfg(not(target_arch = "wasm32"))]
async fn platform_sleep(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

/// On WASM use the browser's timer instead of tokio's OS-level clock.
#[cfg(target_arch = "wasm32")]
async fn platform_sleep(ms: u64) {
    gloo_timers::future::sleep(Duration::from_millis(ms)).await;
}

/// `(alts coll)` — a Future delivering `[value index]` for whichever future in
/// `coll` resolves first.
fn builtin_alts(args: &[Value]) -> ValueResult<Value> {
    let futures = match args.first() {
        Some(v) => value_to_seq_vec(v)?,
        None => Vec::new(),
    };
    Ok(spawn_future_with_roots(futures.clone(), async move {
        alts_select(futures).await
    }))
}

/// Await all `futures` concurrently; return `[value index]` of the first to
/// complete. An empty input resolves to `nil`. A future that completes with an
/// ordinary error contributes `nil`; resource-limit errors propagate.
#[allow(clippy::result_large_err)]
async fn alts_select(futures: Vec<Value>) -> EvalResult {
    if futures.is_empty() {
        return Ok(Value::Nil);
    }
    let branches: Vec<AltBranch> = futures
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            Box::pin(async move {
                let result = await_value(v).await;
                (result, i)
            }) as AltBranch
        })
        .collect();
    let ((result, idx), _, _) = futures_util::future::select_all(branches).await;
    let value = match result {
        Err(
            error @ (EvalError::GasExhausted
            | EvalError::Cancelled
            | EvalError::MemoryLimitExceeded(_)),
        ) => {
            return Err(error);
        }
        other => other.unwrap_or(Value::Nil),
    };
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter([
        value,
        Value::Long(idx as i64),
    ]))))
}

// ── Channels ────────────────────────────────────────────────────────────────

/// Extract the channel `GcPtr` from the first argument, erroring if it is not a
/// channel. The returned pointer is cloned so it can be moved into a spawned
/// task.
fn channel_arg(args: &[Value]) -> ValueResult<GcPtr<NativeObjectBox>> {
    match args.first() {
        Some(Value::NativeObject(obj)) if obj.get().type_tag() == CHANNEL_TAG => Ok(obj.clone()),
        other => Err(ValueError::WrongType {
            expected: "channel",
            got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
        }),
    }
}

/// Borrow the `CljChannel` out of a verified channel native object.
fn chan_ref(obj: &NativeObjectBox) -> &CljChannel {
    obj.downcast_ref::<CljChannel>()
        .expect("channel native object holds a CljChannel")
}

/// `(chan)` / `(chan n)` — create a channel. No argument (or `0`) yields an
/// unbuffered rendezvous channel; a positive `n` yields a buffered channel.
fn builtin_chan(args: &[Value]) -> ValueResult<Value> {
    let capacity = match args.first() {
        None | Some(Value::Nil) => 0,
        Some(Value::Long(n)) if *n >= 0 => *n as usize,
        Some(other) => {
            return Err(ValueError::WrongType {
                expected: "non-negative long (chan capacity)",
                got: other.type_name().to_string(),
            });
        }
    };
    if args.len() > 1 {
        return Err(ValueError::Other(
            "(chan n xf) transducer channels are not yet supported".into(),
        ));
    }
    Ok(Value::NativeObject(gc_native_object(CljChannel::new(
        capacity,
    ))))
}

/// `(take! ch)` — a Future that resolves to the next value on `ch`, or `nil`
/// once `ch` is closed and drained. Parks until a value is available using
/// event-driven notification rather than busy-polling.
fn builtin_take(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        chan_ref(ch.get())
            .take_result()
            .await
            .map_err(cljrs_runtime::env::error::value_error_to_eval_error)
    }))
}

/// `(put! ch val)` — a Future that resolves `true` once `val` is delivered (for
/// a rendezvous channel) or buffered, or `false` if `ch` is closed. Parks
/// (yields) while a buffered channel is full or a rendezvous awaits a taker.
fn builtin_put(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    let val = args.get(1).cloned().unwrap_or(Value::Nil);
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        Ok(Value::Bool(chan_ref(ch.get()).put(val).await))
    }))
}

/// `(close! ch)` — close the channel. Returns `nil`.
fn builtin_close(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    chan_ref(ch.get()).close();
    Ok(Value::Nil)
}

/// `(poll! ch)` — non-blocking take. Returns a buffered value, or `nil` if the
/// channel is empty or closed. Never parks.
fn builtin_poll(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    let channel = chan_ref(ch.get());
    let value = channel.try_take().unwrap_or(Value::Nil);
    if let Some(error) = channel.failure() {
        return Err(error);
    }
    Ok(value)
}

/// `(offer! ch val)` — non-blocking put. Returns `true` if `val` was buffered
/// immediately, `false` otherwise (full, closed, or a rendezvous channel that
/// cannot guarantee an immediate taker). Never parks.
fn builtin_offer(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    let val = args.get(1).cloned().unwrap_or(Value::Nil);
    let obj = ch.get();
    let chan = chan_ref(obj);
    if chan.is_rendezvous() {
        return Ok(Value::Bool(false));
    }
    Ok(Value::Bool(chan.try_put_buffered(&val).unwrap_or(false)))
}

// ── Phase H: blocking sync-context channel ops (<!! / >!!) ──────────────────

/// `(<!! ch)` — blocking take from a synchronous (non-async) context.
///
/// Parks a fiber until data arrives, or blocks the OS thread outside a fiber.
/// Returns nil on a closed, drained channel. Colored async callers use take!.
fn builtin_take_blocking(args: &[Value]) -> ValueResult<Value> {
    if cljrs_runtime::env::callback::current_is_async() && cljrs_value::parking::current().is_none()
    {
        return Err(ValueError::Other(
            "<!! (blocking take) must not be called inside an ^:async function; \
             use (await (take! ch)) instead"
                .into(),
        ));
    }
    let ch = channel_arg(args)?;
    cljrs_value::parking::block_on(chan_ref(ch.get()).take_result())
}

/// `(>!! ch val)` — blocking put from a synchronous (non-async) context.
///
/// Parks a fiber until accepted, or blocks the OS thread outside a fiber.
/// Returns true on success and false when closed. Colored async callers use put!.
fn builtin_put_blocking(args: &[Value]) -> ValueResult<Value> {
    if cljrs_runtime::env::callback::current_is_async() && cljrs_value::parking::current().is_none()
    {
        return Err(ValueError::Other(
            ">!! (blocking put) must not be called inside an ^:async function; \
             use (await (put! ch val)) instead"
                .into(),
        ));
    }
    let ch = channel_arg(args)?;
    let val = args.get(1).cloned().unwrap_or(Value::Nil);
    Ok(Value::Bool(chan_ref(ch.get()).put_blocking(val)))
}

/// `(async-spawn thunk)` — run a zero-arg function as an async task on the
/// `LocalSet`, returning a `Value::Future`. The thunk body runs in an async
/// context, so `await` inside it yields. This is the runtime behind `go`.
fn builtin_async_spawn(args: &[Value]) -> ValueResult<Value> {
    let thunk = args.first().cloned().unwrap_or(Value::Nil);
    let (globals, ns) = cljrs_runtime::env::callback::capture_eval_context()
        .ok_or_else(|| ValueError::Other("async-spawn called outside an eval context".into()))?;
    let rt = globals.async_runtime().ok_or_else(|| {
        ValueError::Other(
            "async-spawn requires an async runtime (call cljrs_async::init first)".into(),
        )
    })?;
    let call_env = Env::new(globals, &ns);
    Ok(rt.spawn_async_call(thunk, Vec::new(), call_env))
}

// ── Phase F: higher-level async utilities ────────────────────────────────────

/// `(join-all futures-seq)` — await all futures in `futures-seq` concurrently,
/// returning a vector of their resolved values. The first error in any future
/// propagates immediately.
fn builtin_join_all(args: &[Value]) -> ValueResult<Value> {
    let futures = match args.first() {
        Some(v) => value_to_seq_vec(v)?,
        None => Vec::new(),
    };
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        let branches: Vec<_> = futures
            .into_iter()
            .map(|v| Box::pin(await_value(v)))
            .collect();
        let results = futures_util::future::join_all(branches).await;
        let mut values = Vec::with_capacity(results.len());
        for r in results {
            values.push(r?);
        }
        Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
            values,
        ))))
    }))
}

/// Run a zero-arg function in a native fiber and return its result channel.
/// A non-nil result is delivered before closure; nil/errors only close it.
/// On wasm32, use the existing async task path.
#[allow(clippy::result_large_err)]
fn builtin_thread_call(args: &[Value]) -> ValueResult<Value> {
    let thunk = args.first().cloned().unwrap_or(Value::Nil);
    let (globals, ns) = cljrs_runtime::env::callback::capture_eval_context()
        .ok_or_else(|| ValueError::Other("thread-call called outside an eval context".into()))?;
    let rt = globals
        .async_runtime()
        .ok_or_else(|| ValueError::Other("thread-call requires an async runtime".into()))?;
    let result_ch = gc_native_object(CljChannel::new(1));
    let ch_val = Value::NativeObject(result_ch.clone());
    let call_env = Env::new(globals, &ns);
    #[cfg(not(target_arch = "wasm32"))]
    let body = crate::runtime::call_fiber(thunk, Vec::new(), call_env, vec![ch_val.clone()])
        .map_err(|error| ValueError::Other(error.to_string()))?
        .into_task();
    #[cfg(not(target_arch = "wasm32"))]
    let _ = rt;
    #[cfg(target_arch = "wasm32")]
    let body = crate::eval_async::await_value(
        rt.spawn_fiber_call(thunk, Vec::new(), call_env)
            .map_err(|error| ValueError::Other(error.to_string()))?,
    );
    let anchor = ch_val.clone();
    let delivery = spawn_future_with_roots(args.to_vec(), async move {
        let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
        match body.await {
            Ok(value) if !matches!(value, Value::Nil) => {
                chan_ref(result_ch.get()).try_put_buffered(&value);
            }
            Ok(_) => {}
            Err(error @ EvalError::Cancelled) => {
                chan_ref(result_ch.get()).fail(crate::channel::ChannelFailure::Cancelled);
                return Err(error);
            }
            Err(error @ EvalError::GasExhausted) => {
                chan_ref(result_ch.get()).fail(crate::channel::ChannelFailure::Gas);
                return Err(error);
            }
            Err(EvalError::MemoryLimitExceeded(fault)) => {
                chan_ref(result_ch.get()).fail(crate::channel::ChannelFailure::Memory(fault));
                return Err(EvalError::MemoryLimitExceeded(fault));
            }
            Err(error) => eprintln!("[clojurust thread error] {error}"),
        }
        chan_ref(result_ch.get()).close();
        Ok(Value::Nil)
    });
    if let Value::Future(future) = delivery {
        future.get().mark_observed();
    }
    Ok(ch_val)
}

/// `(onto-chan! ch coll)` — put every value from `coll` onto `ch` and then
/// close it. Returns a `Future` that resolves to `ch` when all values have
/// been delivered (or closes early if `ch` is already closed). Works for both
/// buffered and rendezvous channels.
fn builtin_onto_chan(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    let coll = args.get(1).cloned().unwrap_or(Value::Nil);
    let values = value_to_seq_vec(&coll)?;
    let rendezvous = chan_ref(ch.get()).is_rendezvous();
    Ok(spawn_future_with_roots(args.to_vec(), async move {
        for v in values {
            if rendezvous {
                let token = loop {
                    match chan_ref(ch.get()).rv_offer(&v) {
                        RvOffer::Offered(t) => break t,
                        RvOffer::Closed => return Ok(Value::NativeObject(ch)),
                        RvOffer::Full => {}
                    }
                    tokio::task::yield_now().await;
                };
                loop {
                    match chan_ref(ch.get()).rv_status(token) {
                        RvStatus::Taken => break,
                        RvStatus::ClosedUntaken => return Ok(Value::NativeObject(ch)),
                        RvStatus::Waiting => {}
                    }
                    tokio::task::yield_now().await;
                }
            } else {
                loop {
                    match chan_ref(ch.get()).try_put_buffered(&v) {
                        Some(true) => break,
                        Some(false) => return Ok(Value::NativeObject(ch)),
                        None => {}
                    }
                    tokio::task::yield_now().await;
                }
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::NativeObject(ch))
    }))
}

/// `(to-chan! coll)` — create a buffered channel, seed it with all values from
/// `coll` in a background task, then close it. The channel is returned
/// immediately.
fn builtin_to_chan(args: &[Value]) -> ValueResult<Value> {
    let coll = args.first().cloned().unwrap_or(Value::Nil);
    let values = value_to_seq_vec(&coll)?;
    let capacity = values.len().max(1);
    let ch = gc_native_object(CljChannel::new(capacity));
    let ch_val = Value::NativeObject(ch.clone());
    spawn_future_with_roots(args.to_vec(), async move {
        for v in values {
            loop {
                match chan_ref(ch.get()).try_put_buffered(&v) {
                    Some(true) => break,
                    Some(false) => return Ok(Value::Nil),
                    None => {}
                }
                tokio::task::yield_now().await;
            }
        }
        chan_ref(ch.get()).close();
        Ok(Value::Nil)
    });
    Ok(ch_val)
}

// ── Mult helpers ─────────────────────────────────────────────────────────────

fn mult_arg(args: &[Value], idx: usize) -> ValueResult<GcPtr<NativeObjectBox>> {
    match args.get(idx) {
        Some(Value::NativeObject(obj)) if obj.get().type_tag() == MULT_TAG => Ok(obj.clone()),
        other => Err(ValueError::WrongType {
            expected: "mult",
            got: other.map(|v| v.type_name().to_string()).unwrap_or_default(),
        }),
    }
}

fn mult_ref(obj: &NativeObjectBox) -> &CljMult {
    obj.downcast_ref::<CljMult>()
        .expect("mult native object holds a CljMult")
}

/// `(mult source-ch)` — create a broadcast multiplexer backed by `source-ch`.
/// Starts a background task that reads from `source-ch` and forwards each
/// value to all registered taps. Taps are added with `tap!`.
fn builtin_mult(args: &[Value]) -> ValueResult<Value> {
    let source_ch = channel_arg(args)?;
    let mult = gc_native_object(CljMult::new());
    let mult_val = Value::NativeObject(mult.clone());

    spawn_future_with_roots(args.to_vec(), async move {
        loop {
            // Take the next value from the source channel.
            let v = loop {
                match chan_ref(source_ch.get()).try_take() {
                    // `nil` from try_take means the channel is closed and drained.
                    Some(Value::Nil) => {
                        let taps = mult_ref(mult.get()).taps.lock().unwrap().clone();
                        for (tap_ch, close_on_done) in &taps {
                            if *close_on_done {
                                chan_ref(tap_ch.get()).close();
                            }
                        }
                        return Ok(Value::Nil);
                    }
                    Some(v) => break v,
                    None => {}
                }
                tokio::task::yield_now().await;
            };

            // Snapshot the tap list to avoid holding the lock during puts.
            let taps: Vec<(GcPtr<NativeObjectBox>, bool)> =
                mult_ref(mult.get()).taps.lock().unwrap().clone();

            for (tap_ch, _) in &taps {
                if chan_ref(tap_ch.get()).is_rendezvous() {
                    let token = loop {
                        match chan_ref(tap_ch.get()).rv_offer(&v) {
                            RvOffer::Offered(t) => break Some(t),
                            RvOffer::Closed => break None,
                            RvOffer::Full => {}
                        }
                        tokio::task::yield_now().await;
                    };
                    if let Some(token) = token {
                        loop {
                            match chan_ref(tap_ch.get()).rv_status(token) {
                                RvStatus::Taken | RvStatus::ClosedUntaken => break,
                                RvStatus::Waiting => {}
                            }
                            tokio::task::yield_now().await;
                        }
                    }
                } else {
                    loop {
                        if chan_ref(tap_ch.get()).try_put_buffered(&v).is_some() {
                            break;
                        }
                        tokio::task::yield_now().await;
                    }
                }
            }
        }
    });

    Ok(mult_val)
}

/// `(tap! mult ch)` / `(tap! mult ch close?)` — register `ch` as a tap on
/// `mult`. If `close?` is `true` (the default), `ch` is closed when the source
/// channel closes.
fn builtin_tap(args: &[Value]) -> ValueResult<Value> {
    let mult_obj = mult_arg(args, 0)?;
    let tap_ch = channel_arg(&args[1..])?;
    let close_on_done = match args.get(2) {
        None | Some(Value::Bool(true)) => true,
        Some(Value::Nil) | Some(Value::Bool(false)) => false,
        _ => true,
    };
    mult_ref(mult_obj.get())
        .taps
        .lock()
        .unwrap()
        .push((tap_ch, close_on_done));
    Ok(Value::NativeObject(mult_obj))
}

/// `(untap! mult ch)` — remove `ch` from `mult`'s tap list.
fn builtin_untap(args: &[Value]) -> ValueResult<Value> {
    let mult_obj = mult_arg(args, 0)?;
    let tap_ch = channel_arg(&args[1..])?;
    mult_ref(mult_obj.get())
        .taps
        .lock()
        .unwrap()
        .retain(|(ch, _)| !GcPtr::ptr_eq(ch, &tap_ch));
    Ok(Value::NativeObject(mult_obj))
}

/// `(untap-all! mult)` — remove all taps from `mult`.
fn builtin_untap_all(args: &[Value]) -> ValueResult<Value> {
    let mult_obj = mult_arg(args, 0)?;
    mult_ref(mult_obj.get()).taps.lock().unwrap().clear();
    Ok(Value::NativeObject(mult_obj))
}

/// Select exactly one take or `[channel value]` put. Options are :priority
/// (input order) and :default (return immediately when nothing is ready).
fn builtin_alts_blocking(args: &[Value]) -> ValueResult<Value> {
    if cljrs_runtime::env::callback::current_is_async() && cljrs_value::parking::current().is_none()
    {
        return Err(ValueError::Other(
            "alts!! requires a synchronous context or a fiber".into(),
        ));
    }
    let alternatives = value_to_seq_vec(&args[0])?;
    let _roots = cljrs_runtime::env::gc_roots::root_values(&alternatives);
    let mut default = None;
    let mut priority = false;
    let mut option = |key: &Value, value: &Value| -> ValueResult<()> {
        match key {
            Value::Keyword(k)
                if k.get().namespace.is_none() && k.get().name.as_ref() == "default" =>
            {
                default = Some(value.clone())
            }
            Value::Keyword(k)
                if k.get().namespace.is_none() && k.get().name.as_ref() == "priority" =>
            {
                priority = !matches!(value, Value::Nil | Value::Bool(false))
            }
            _ => {
                return Err(ValueError::Other(
                    "alts!! accepts only :default and :priority options".into(),
                ));
            }
        }
        Ok(())
    };
    if let [Value::Map(options)] = &args[1..] {
        for (key, value) in options.iter() {
            option(key, value)?;
        }
    } else {
        if !(args.len() - 1).is_multiple_of(2) {
            return Err(ValueError::Other(
                "alts!! options require key/value pairs".into(),
            ));
        }
        for [key, value] in args[1..].as_chunks::<2>().0 {
            option(key, value)?;
        }
    }
    if alternatives.is_empty() && default.is_none() {
        return Err(ValueError::Other(
            "alts!! requires at least one channel or a :default option".into(),
        ));
    }
    let mut requests = Vec::with_capacity(alternatives.len());
    for alternative in &alternatives {
        let (channel, value) = if let Value::Vector(pair) = alternative {
            if pair.get().count() != 2 {
                return Err(ValueError::Other(
                    "alts!! put alternatives require [channel value]".into(),
                ));
            }
            let value = pair.get().nth(1).unwrap().clone();
            if matches!(value, Value::Nil) {
                return Err(ValueError::Other("cannot put nil on a channel".into()));
            }
            (
                channel_arg(std::slice::from_ref(pair.get().nth(0).unwrap()))?,
                Some(value),
            )
        } else {
            (channel_arg(std::slice::from_ref(alternative))?, None)
        };
        requests.push((channel, value));
    }
    if !priority {
        requests.shuffle(&mut rand::rng());
    }
    let ops = requests
        .iter()
        .map(|(channel, value)| {
            let ch = chan_ref(channel.get());
            match value {
                Some(value) => crate::channel::SelectOp::Put(ch, value.clone()),
                None => crate::channel::SelectOp::Take(ch),
            }
        })
        .collect();
    let (value, index) = cljrs_value::parking::block_on(crate::channel::select(ops, default));
    if index != usize::MAX
        && requests[index].1.is_none()
        && let Some(error) = chan_ref(requests[index].0.get()).failure()
    {
        return Err(error);
    }
    let channel = if index == usize::MAX {
        Value::keyword(cljrs_value::Keyword::simple("default"))
    } else {
        Value::NativeObject(requests[index].0.clone())
    };
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter([
        value, channel,
    ]))))
}
