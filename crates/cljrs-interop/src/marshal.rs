//! Type marshalling: `Value` ↔ Rust type conversions.
//!
//! Provides `FromValue` and `IntoValue` traits with implementations for
//! common Rust types.

use cljrs_gc::GcPtr;
use cljrs_value::{Value, ValueError, ValueResult};
use num_bigint::BigInt;

/// Convert a Rust value into a Clojure `Value`.
pub trait IntoValue {
    fn into_value(self) -> Value;
}

/// Extract a Rust value from a Clojure `Value`.
pub trait FromValue: Sized {
    fn from_value(v: &Value) -> ValueResult<Self>;
}

// ── IntoValue impls ──────────────────────────────────────────────────────────

impl IntoValue for Value {
    fn into_value(self) -> Value {
        self
    }
}

impl IntoValue for () {
    fn into_value(self) -> Value {
        Value::Nil
    }
}

impl IntoValue for bool {
    fn into_value(self) -> Value {
        Value::Bool(self)
    }
}

impl IntoValue for i64 {
    fn into_value(self) -> Value {
        Value::Long(self)
    }
}

impl IntoValue for f64 {
    fn into_value(self) -> Value {
        Value::Double(self)
    }
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::Str(GcPtr::new(self))
    }
}

impl IntoValue for &str {
    fn into_value(self) -> Value {
        Value::Str(GcPtr::new(self.to_string()))
    }
}

impl IntoValue for BigInt {
    fn into_value(self) -> Value {
        Value::BigInt(GcPtr::new(self))
    }
}

impl<T: IntoValue> IntoValue for Option<T> {
    fn into_value(self) -> Value {
        match self {
            Some(v) => v.into_value(),
            None => Value::Nil,
        }
    }
}

impl IntoValue for Vec<Value> {
    fn into_value(self) -> Value {
        Value::Vector(GcPtr::new(cljrs_value::PersistentVector::from_iter(self)))
    }
}

// ── FromValue impls ──────────────────────────────────────────────────────────

impl FromValue for Value {
    fn from_value(v: &Value) -> ValueResult<Self> {
        Ok(v.clone())
    }
}

impl FromValue for bool {
    fn from_value(v: &Value) -> ValueResult<Self> {
        match v {
            Value::Bool(b) => Ok(*b),
            _ => Err(type_error("bool", v)),
        }
    }
}

impl FromValue for i64 {
    fn from_value(v: &Value) -> ValueResult<Self> {
        match v {
            Value::Long(n) => Ok(*n),
            _ => Err(type_error("i64", v)),
        }
    }
}

impl FromValue for f64 {
    fn from_value(v: &Value) -> ValueResult<Self> {
        match v {
            Value::Double(d) => Ok(*d),
            Value::Long(n) => Ok(*n as f64),
            _ => Err(type_error("f64", v)),
        }
    }
}

impl FromValue for String {
    fn from_value(v: &Value) -> ValueResult<Self> {
        match v {
            Value::Str(s) => Ok(s.get().clone()),
            _ => Err(type_error("string", v)),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(v: &Value) -> ValueResult<Self> {
        match v {
            Value::Nil => Ok(None),
            other => Ok(Some(T::from_value(other)?)),
        }
    }
}

fn type_error(expected: &'static str, got: &Value) -> ValueError {
    ValueError::WrongType {
        expected,
        got: got.type_name().to_string(),
    }
}
