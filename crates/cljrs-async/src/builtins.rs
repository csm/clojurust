//! Native `clojure.core.async` builtins.
//!
//! The futures-returning primitives (`timeout`, `alts`, `take!`, `put!`) return
//! a `Value::Future` immediately and resolve on the `LocalSet` executor, so they
//! compose with `await` exactly like an `^:async` call. `close!`, `poll!`, and
//! `offer!` act on a channel synchronously and return their result directly.
//! `async-spawn` runs a thunk on the `LocalSet` as an async task (the runtime
//! backing the `go` macro).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_interp::destructure::value_to_seq_vec;
use cljrs_value::{
    Arity, NativeFn, NativeObjectBox, PersistentVector, Value, ValueError, ValueResult,
    gc_native_object,
};

use crate::channel::{CHANNEL_TAG, CljChannel, RvOffer, RvStatus};
use crate::eval_async::{await_value, spawn_future};

/// One branch of an `alts` race: awaits a future and tags it with its index.
type AltBranch = Pin<Box<dyn Future<Output = (EvalResult, usize)>>>;

/// Register the async native functions into the given namespace.
pub(crate) fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        ("timeout", Arity::Fixed(1), builtin_timeout),
        ("alts", Arity::Fixed(1), builtin_alts),
        ("chan", Arity::Variadic { min: 0 }, builtin_chan),
        ("take!", Arity::Fixed(1), builtin_take),
        ("put!", Arity::Fixed(2), builtin_put),
        ("close!", Arity::Fixed(1), builtin_close),
        ("poll!", Arity::Fixed(1), builtin_poll),
        ("offer!", Arity::Fixed(2), builtin_offer),
        ("async-spawn", Arity::Fixed(1), builtin_async_spawn),
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
    Ok(spawn_future(async move {
        tokio::time::sleep(Duration::from_millis(ms)).await;
        Ok(Value::Nil)
    }))
}

/// `(alts coll)` — a Future delivering `[value index]` for whichever future in
/// `coll` resolves first.
fn builtin_alts(args: &[Value]) -> ValueResult<Value> {
    let futures = match args.first() {
        Some(v) => value_to_seq_vec(v),
        None => Vec::new(),
    };
    Ok(spawn_future(async move { Ok(alts_select(futures).await) }))
}

/// Await all `futures` concurrently; return `[value index]` of the first to
/// complete. An empty input resolves to `nil`. A future that completes with an
/// error contributes `nil` as its value.
async fn alts_select(futures: Vec<Value>) -> Value {
    if futures.is_empty() {
        return Value::Nil;
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
    let value = result.unwrap_or(Value::Nil);
    Value::Vector(GcPtr::new(PersistentVector::from_iter([
        value,
        Value::Long(idx as i64),
    ])))
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
/// once `ch` is closed and drained. Parks (yields) until a value is available.
fn builtin_take(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    Ok(spawn_future(async move {
        loop {
            if let Some(v) = chan_ref(ch.get()).try_take() {
                return Ok(v);
            }
            tokio::task::yield_now().await;
        }
    }))
}

/// `(put! ch val)` — a Future that resolves `true` once `val` is delivered (for
/// a rendezvous channel) or buffered, or `false` if `ch` is closed. Parks
/// (yields) while a buffered channel is full or a rendezvous awaits a taker.
fn builtin_put(args: &[Value]) -> ValueResult<Value> {
    let ch = channel_arg(args)?;
    let val = args.get(1).cloned().unwrap_or(Value::Nil);
    let rendezvous = chan_ref(ch.get()).is_rendezvous();
    Ok(spawn_future(async move {
        if rendezvous {
            // Phase 1: place the value into the channel's single slot.
            let token = loop {
                match chan_ref(ch.get()).rv_offer(&val) {
                    RvOffer::Offered(t) => break t,
                    RvOffer::Closed => return Ok(Value::Bool(false)),
                    RvOffer::Full => {}
                }
                tokio::task::yield_now().await;
            };
            // Phase 2: wait for a taker to consume it (the handoff).
            loop {
                match chan_ref(ch.get()).rv_status(token) {
                    RvStatus::Taken => return Ok(Value::Bool(true)),
                    RvStatus::ClosedUntaken => return Ok(Value::Bool(false)),
                    RvStatus::Waiting => {}
                }
                tokio::task::yield_now().await;
            }
        } else {
            loop {
                if let Some(accepted) = chan_ref(ch.get()).try_put_buffered(&val) {
                    return Ok(Value::Bool(accepted));
                }
                tokio::task::yield_now().await;
            }
        }
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
    Ok(chan_ref(ch.get()).try_take().unwrap_or(Value::Nil))
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

/// `(async-spawn thunk)` — run a zero-arg function as an async task on the
/// `LocalSet`, returning a `Value::Future`. The thunk body runs in an async
/// context, so `await` inside it yields. This is the runtime behind `go`.
fn builtin_async_spawn(args: &[Value]) -> ValueResult<Value> {
    let thunk = args.first().cloned().unwrap_or(Value::Nil);
    let (globals, ns) = cljrs_env::callback::capture_eval_context()
        .ok_or_else(|| ValueError::Other("async-spawn called outside an eval context".into()))?;
    let rt = globals.async_runtime().ok_or_else(|| {
        ValueError::Other(
            "async-spawn requires an async runtime (call cljrs_async::init first)".into(),
        )
    })?;
    let call_env = Env::new(globals, &ns);
    Ok(rt.spawn_async_call(thunk, Vec::new(), call_env))
}
