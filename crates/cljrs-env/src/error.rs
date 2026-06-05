//! Evaluation-time error types.

use cljrs_value::Value;

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
    Read(#[from] cljrs_types::error::CljxError),

    /// Internal signal for `recur` — caught by the loop/fn trampoline.
    /// Never propagated to user code.
    #[doc(hidden)]
    #[error("internal: recur outside loop or fn")]
    Recur(Vec<Value>),

    #[error(
        "commit {commit:?} failed signature verification — \
         refusing to execute versioned symbol (enable GPG/SSH trust or disable \
         :verify-commit-signatures): {reason}"
    )]
    CommitSignatureVerificationFailed { commit: String, reason: String },
}

impl EvalError {
    /// Convert this error into a Clojure error *value* (`Value::Error`).
    ///
    /// A `Thrown` value is returned unchanged (preserving its `ex-data` /
    /// `ex-cause`); any other error is wrapped in a fresh `ExceptionInfo` with
    /// the error's display string as the message. Used where an error must be
    /// stored as a value and later re-thrown — e.g. a failed `Future`'s state.
    pub fn to_error_value(self) -> Value {
        match self {
            EvalError::Thrown(v) => v,
            other => {
                let msg = other.to_string();
                Value::Error(cljrs_gc::GcPtr::new(cljrs_value::ExceptionInfo::new(
                    cljrs_value::ValueError::Other(msg.clone()),
                    msg,
                    None,
                    None,
                )))
            }
        }
    }
}

pub type EvalResult<T = Value> = Result<T, EvalError>;
