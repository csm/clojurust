//! Error propagation through Rust's infallible equality, hash, and format traits.
//!
//! A trait operation stops immediately on realization failure. Evaluation and
//! native ABI boundaries catch only this private payload. Other panics and
//! coroutine cancellation continue unwinding unchanged.
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

#[derive(Debug)]
struct Failure(String);

/// Run an operation that can realize a lazy sequence through a Rust trait.
/// Ordinary panics and coroutine cancellation are not converted to errors.
pub fn catch<T>(operation: impl FnOnce() -> T) -> Result<T, String> {
    match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(value) => Ok(value),
        Err(payload) => match payload.downcast::<Failure>() {
            Ok(error) => Err(error.0),
            Err(payload) => resume_unwind(payload),
        },
    }
}

/// Realize for an infallible trait. Callers must use `catch` at their boundary.
pub(crate) fn for_trait(sequence: &crate::LazySeq) -> crate::Value {
    sequence
        .try_realize()
        .unwrap_or_else(|message| resume_unwind(Box::new(Failure(message))))
}

/// True for scalar values whose equality, hash, and display cannot run user code.
/// This is conservative: composite and mutable values use the rooted slow path.
pub fn is_inert(value: &crate::Value) -> bool {
    use crate::Value;
    matches!(
        value.unwrap_meta(),
        Value::Nil
            | Value::Bool(_)
            | Value::Long(_)
            | Value::Double(_)
            | Value::Char(_)
            | Value::Str(_)
            | Value::Keyword(_)
            | Value::Symbol(_)
            | Value::BigInt(_)
            | Value::BigDecimal(_)
            | Value::Ratio(_)
            | Value::Uuid(_)
            | Value::Pattern(_)
    )
}
