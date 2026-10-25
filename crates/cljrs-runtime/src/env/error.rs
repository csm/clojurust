//! Evaluation-time error types.

use cljrs_value::Value;
use cljrs_value::ValueError;

#[derive(Debug, thiserror::Error)]
pub enum EvalError {
    #[error("runtime error: {0}")]
    Runtime(String),

    /// The cooperative execution-credit budget was exhausted.
    #[error("gas exhausted")]
    GasExhausted,

    /// The process memory governor refused a governed allocation (strict
    /// mode), and a collection did not bring committed memory back within
    /// the hard limit.  Fixed-size; building it never allocates.  Like
    /// `GasExhausted`, `try`/`catch` does not intercept it.
    #[error("{0}")]
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),

    /// An operation attempted to use a capability unavailable to an isolated
    /// transaction function.
    #[error("effect forbidden in transaction function: {0}")]
    ForbiddenEffect(String),

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
                let error = match other {
                    EvalError::MemoryLimitExceeded(fault) => {
                        cljrs_value::ValueError::MemoryLimitExceeded(fault)
                    }
                    _ => cljrs_value::ValueError::Other(msg.clone()),
                };
                // Error reporting may use the critical memory reserve.
                Value::Error(cljrs_gc::GcPtr::new_critical(
                    cljrs_value::ExceptionInfo::new(error, msg, None, None),
                ))
            }
        }
    }
}

/// Surface a builtin's `ValueError` to the evaluator as a *catchable* condition.
///
/// Internal runtime errors (`IndexOutOfBounds`, `WrongType`, `ArityError`, …)
/// are normalized into an `EvalError::Thrown(Value::Error(..))` carrying the
/// original `ValueError` variant and its plain display message — no
/// `runtime error:` prefix — so that `(catch :default e ..)` /
/// `(catch Throwable e ..)` bind a value on which `ex-message` / `ex-data`
/// behave the same as for a user `throw` / `ex-info`. A `ValueError::Thrown`
/// (a builtin re-throwing a Clojure value) is surfaced as that exact value.
pub fn value_error_to_eval_error(err: ValueError) -> EvalError {
    match err {
        ValueError::Thrown(v) => EvalError::Thrown(v),
        ValueError::GasExhausted => EvalError::GasExhausted,
        ValueError::MemoryLimitExceeded(fault) => {
            // The error is back on a path that carries it; a copy left in
            // the runtime-fault slot (see `callback::invoke`) would report
            // it a second time.
            let _ = cljrs_gc::governor::take_runtime_fault();
            EvalError::MemoryLimitExceeded(fault)
        }
        other => {
            let msg = other.to_string();
            // Error reporting may use the critical memory reserve.
            EvalError::Thrown(Value::Error(cljrs_gc::GcPtr::new_critical(
                cljrs_value::ExceptionInfo::new(other, msg, None, None),
            )))
        }
    }
}

pub type EvalResult<T = Value> = Result<T, EvalError>;
