//! Evaluation-time error types.

use cljx_value::Value;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("runtime error: {0}")]
    Runtime(String),

    #[error("unbound symbol: {0}")]
    UnboundSymbol(String),

    #[error("arity error calling {name}: expected {expected}, got {got}")]
    Arity {
        name: String,
        expected: String,
        got: usize,
    },

    #[error("not callable: {0}")]
    NotCallable(String),

    /// A value thrown via `throw` or `ex-info`.
    #[error("{0}")]
    Thrown(Value),

    #[error("read error: {0}")]
    Read(#[from] cljx_types::error::CljxError),

    /// Internal signal for `recur` — caught by the loop/fn trampoline.
    /// Never propagated to user code.
    #[doc(hidden)]
    #[error("internal: recur outside loop or fn")]
    Recur(Vec<Value>),
}

pub type EvalResult<T = Value> = Result<T, EvalError>;
