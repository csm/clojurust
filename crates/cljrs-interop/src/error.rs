//! Error bridging: Rust `Result` → Clojure exceptions.

use cljrs_value::{Value, ValueError, ValueResult};

use crate::IntoValue;

/// Convert a `Result<T, E>` into `ValueResult<Value>`.
///
/// - `Ok(t)` is converted via `IntoValue::into_value`.
/// - `Err(e)` is converted to `ValueError::Other` via `Display`.
///
/// # Example
/// ```ignore
/// fn my_native(args: &[Value]) -> ValueResult<Value> {
///     let n: i64 = FromValue::from_value(&args[0])?;
///     wrap_result(some_fallible_op(n))
/// }
/// ```
pub fn wrap_result<T: IntoValue, E: std::fmt::Display>(r: Result<T, E>) -> ValueResult<Value> {
    match r {
        Ok(v) => Ok(v.into_value()),
        Err(e) => Err(ValueError::Other(e.to_string())),
    }
}
