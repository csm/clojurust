#![allow(unused)]

/// Value-level errors: type mismatches, arity errors, out-of-bounds, etc.
///
/// These are deliberately free of miette/NamedSource so they can be
/// constructed without source-location context.  The evaluator wraps them
/// in `CljxError::EvalError` when it has a span.
#[derive(Debug, thiserror::Error)]
pub enum ValueError {
    #[error("wrong type: expected {expected}, got {got}")]
    WrongType { expected: &'static str, got: String },

    #[error("index out of bounds: {idx} >= {count}")]
    IndexOutOfBounds { idx: usize, count: usize },

    #[error("arity error: {name} expects {expected}, got {got}")]
    ArityError {
        name: String,
        expected: String,
        got: usize,
    },

    #[error("cannot call non-function value: {value}")]
    NotCallable { value: String },

    #[error("map must have an even number of forms, got {count}")]
    OddMap { count: usize },

    #[error("this feature is not yet supported")]
    Unsupported,

    #[error("{0}")]
    Other(String),
}

pub type ValueResult<T> = Result<T, ValueError>;
