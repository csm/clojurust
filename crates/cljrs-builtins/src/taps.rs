//! Tap system: add-tap, remove-tap, tap>
//!
//! `tap>` enqueues a value (bounded queue, drops on overflow).
//! A background drain thread delivers values to all registered tap fns.

use cljrs_env::taps::{add_tap, remove_tap, send};
use cljrs_value::{Value, ValueResult};

/// (add-tap f) — register a tap fn. Returns nil.
pub fn builtin_add_tap(args: &[Value]) -> ValueResult<Value> {
    let f = args[0].clone();
    add_tap(f);
    Ok(Value::Nil)
}

/// (remove-tap f) — unregister a tap fn. Returns nil.
pub fn builtin_remove_tap(args: &[Value]) -> ValueResult<Value> {
    let f = &args[0];
    remove_tap(f);
    Ok(Value::Nil)
}

/// (tap> val) — enqueue a value. Returns true if enqueued, false if dropped.
pub fn builtin_tap_send(args: &[Value]) -> ValueResult<Value> {
    let val = args[0].clone();
    let sent = send(val);
    Ok(Value::Bool(sent))
}
