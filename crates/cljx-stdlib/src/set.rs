//! Native implementations for `clojure.set`.

use std::sync::Arc;

use crate::register_fns;
use cljx_gc::GcPtr;
use cljx_value::value::SetValue;
use cljx_value::{Arity, MapValue, PersistentHashSet, Value, ValueError, ValueResult};

pub fn register(globals: &Arc<cljx_eval::GlobalEnv>, ns: &str) {
    register_fns!(
        globals,
        ns,
        [
            ("union", Arity::Variadic { min: 0 }, union),
            ("intersection", Arity::Variadic { min: 1 }, intersection),
            ("difference", Arity::Variadic { min: 1 }, difference),
            ("subset?", Arity::Fixed(2), subset_q),
            ("superset?", Arity::Fixed(2), superset_q),
            ("select", Arity::Fixed(2), select),
            ("map-invert", Arity::Fixed(1), map_invert),
        ]
    );
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn get_set(v: &Value) -> ValueResult<&SetValue> {
    match v {
        Value::Set(s) => Ok(s),
        other => Err(ValueError::WrongType {
            expected: "set",
            got: other.type_name().to_string(),
        }),
    }
}

// ── Implementations ────────────────────────────────────────────────────────────

/// `(union s1 s2 ...)` — all elements in any set.
fn union(args: &[Value]) -> ValueResult<Value> {
    // TODO - use first arg to influence type. First sorted-set -> sorted-set
    let mut result = PersistentHashSet::empty();
    for arg in args {
        let s = get_set(arg)?;
        for v in s.iter() {
            result.conj_mut(v.clone());
        }
    }
    Ok(Value::Set(SetValue::Hash(GcPtr::new(result))))
}

/// `(intersection s1 s2 ...)` — elements present in every set.
fn intersection(args: &[Value]) -> ValueResult<Value> {
    // TODO - use first arg to influence type. First sorted-set -> sorted-set.
    if args.is_empty() {
        return Ok(Value::Set(SetValue::Hash(GcPtr::new(
            PersistentHashSet::empty(),
        ))));
    }
    let first = get_set(&args[0])?;
    let mut result = PersistentHashSet::from_iter(first.iter().cloned());
    for arg in &args[1..] {
        let s = get_set(arg)?;
        let mut next = PersistentHashSet::empty();
        for v in result.iter() {
            if s.contains(v) {
                next.conj_mut(v.clone());
            }
        }
        result = next;
    }
    Ok(Value::Set(SetValue::Hash(GcPtr::new(result))))
}

/// `(difference s1 s2 ...)` — elements in s1 not in any subsequent set.
fn difference(args: &[Value]) -> ValueResult<Value> {
    // TODO - use first arg to influence type. First sorted-set -> sorted-set
    if args.is_empty() {
        return Ok(Value::Set(SetValue::Hash(GcPtr::new(
            PersistentHashSet::empty(),
        ))));
    }
    let first = get_set(&args[0])?;
    let mut result = PersistentHashSet::from_iter(first.iter().cloned());
    for arg in &args[1..] {
        let s = get_set(arg)?;
        let mut next = PersistentHashSet::empty();
        for v in result.iter() {
            if !s.contains(v) {
                next.conj_mut(v.clone());
            }
        }
        result = next;
    }
    Ok(Value::Set(SetValue::Hash(GcPtr::new(result))))
}

/// `(subset? s1 s2)` — true if every element of s1 is in s2.
fn subset_q(args: &[Value]) -> ValueResult<Value> {
    let s1 = get_set(&args[0])?;
    let s2 = get_set(&args[1])?;
    let result = s1.iter().all(|v| s2.contains(v));
    Ok(Value::Bool(result))
}

/// `(superset? s1 s2)` — true if s1 contains every element of s2.
fn superset_q(args: &[Value]) -> ValueResult<Value> {
    let s1 = get_set(&args[0])?;
    let s2 = get_set(&args[1])?;
    let result = s2.iter().all(|v| s1.contains(v));
    Ok(Value::Bool(result))
}

/// `(select pred set)` — subset of elements satisfying pred.
/// Note: pred must be a Value::Fn or NativeFunction — we can't call it from
/// a pure builtin.  This is intercepted in apply.rs when the callee has name
/// "clojure.set/select".  This stub just returns an error.
fn select(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "clojure.set/select sentinel should not be called directly".to_string(),
    })
}

/// `(map-invert m)` — return map with keys and vals swapped.
fn map_invert(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Map(m) => {
            let mut result = MapValue::empty();
            m.for_each(|k, v| {
                result = result.assoc(v.clone(), k.clone());
            });
            Ok(Value::Map(result))
        }
        other => Err(ValueError::WrongType {
            expected: "map",
            got: other.type_name().to_string(),
        }),
    }
}
