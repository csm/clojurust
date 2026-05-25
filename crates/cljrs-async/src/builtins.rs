//! Native `clojure.core.async` builtins: `timeout` and `alts`.
//!
//! Both return a `Value::Future` immediately and resolve on the `LocalSet`
//! executor, so they compose with `await` exactly like an `^:async` call.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use cljrs_env::env::GlobalEnv;
use cljrs_env::error::EvalResult;
use cljrs_gc::GcPtr;
use cljrs_interp::destructure::value_to_seq_vec;
use cljrs_value::{Arity, NativeFn, PersistentVector, Value, ValueError, ValueResult};

use crate::eval_async::{await_value, spawn_future};

/// One branch of an `alts` race: awaits a future and tags it with its index.
type AltBranch = Pin<Box<dyn Future<Output = (EvalResult, usize)>>>;

/// Register the async native functions into the given namespace.
pub(crate) fn register(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        ("timeout", Arity::Fixed(1), builtin_timeout),
        ("alts", Arity::Fixed(1), builtin_alts),
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
