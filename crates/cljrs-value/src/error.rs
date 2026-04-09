#![allow(unused)]

use crate::collections::{TransientMap, TransientVector};
use crate::hash::{hash_combine_ordered, hash_string};
use crate::{ClojureHash, Keyword, MapValue, PersistentArrayMap, Value};
use cljrs_gc::{GcPtr, GcVisitor, MarkVisitor, Trace};
use std::backtrace::{Backtrace, BacktraceStatus};

/// Value-level errors: type mismatches, arity errors, out-of-bounds, etc.
///
/// These are deliberately free of miette/NamedSource so they can be
/// constructed without source-location context.  The evaluator wraps them
/// in `CljxError::EvalError` when it has a span.
#[derive(Debug, thiserror::Error, Clone)]
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

    #[error("out of range")]
    OutOfRange,

    #[error("transient already persisted")]
    TransientAlreadyPersisted,

    #[error("could not parse value")]
    Parse,

    #[error("thrown exception")]
    Thrown(Value),
}

pub type ValueResult<T> = Result<T, ValueError>;

#[derive(Clone, Debug)]
pub struct ExceptionInfo {
    pub(crate) error: ValueError,
    pub(crate) message: String,
    pub(crate) data: Option<MapValue>,
    pub(crate) cause: Option<GcPtr<ExceptionInfo>>,
}

impl ExceptionInfo {
    pub fn new(
        error: ValueError,
        message: String,
        data: Option<MapValue>,
        cause: Option<GcPtr<ExceptionInfo>>,
    ) -> Self {
        Self {
            error,
            message,
            data: data.as_ref().cloned(),
            cause: cause.as_ref().cloned(),
        }
    }

    fn to_via_map(&self) -> ValueResult<Value> {
        let map = TransientMap::new();
        map.assoc(
            Value::keyword(Keyword::simple("type")),
            Value::string(match self.error {
                ValueError::WrongType { .. } => "WrongType",
                ValueError::IndexOutOfBounds { .. } => "IndexOutOfBounds",
                ValueError::ArityError { .. } => "ArityError",
                ValueError::NotCallable { .. } => "NotCallable",
                ValueError::OddMap { .. } => "OddMap",
                ValueError::Unsupported => "Unsupported",
                ValueError::Other(_) => "Other",
                ValueError::OutOfRange => "OutOfRange",
                ValueError::TransientAlreadyPersisted => "TransientAlreadyPersisted",
                ValueError::Parse => "ParseError",
                ValueError::Thrown(_) => "Thrown",
            }),
        )?;
        map.assoc(
            Value::keyword(Keyword::simple("message")),
            Value::string(&self.message),
        )?;
        if let Some(info) = self.data.as_ref() {
            map.assoc(
                Value::keyword(Keyword::simple("data")),
                Value::Map(info.clone()),
            )?;
        }
        // TODO, add :at (source location)
        Ok(Value::Map(MapValue::Hash(GcPtr::new(map.persistent()?))))
    }

    pub fn to_map(&self) -> ValueResult<Value> {
        let map = TransientMap::new();
        map.assoc(
            Value::keyword(Keyword::simple("cause")),
            self.cause
                .as_ref()
                .map(|c| Value::Str(GcPtr::new(c.get().message.to_string())))
                .unwrap_or(Value::Str(GcPtr::new(self.message.to_string()))),
        )?;
        if let Some(info) = self.data.as_ref() {
            map.assoc(
                Value::keyword(Keyword::simple("data")),
                Value::Map(info.clone()),
            )?;
        }
        let via = TransientVector::new();
        via.append(self.to_via_map()?);
        let mut cur = self.cause.as_ref();
        while let Some(e) = cur {
            via.append(e.get().to_via_map()?);
            cur = e.get().cause.as_ref();
        }
        map.assoc(
            Value::keyword(Keyword::simple("via")),
            Value::Vector(GcPtr::new(via.persistent()?)),
        );
        let backtrace = Backtrace::capture();
        if matches!(backtrace.status(), BacktraceStatus::Captured) {
            // TODO, turn frames() into vector once stable?
            map.assoc(
                Value::keyword(Keyword::simple("trace")),
                Value::string(format!("{}", backtrace)),
            )?;
        }
        Ok(Value::Map(MapValue::Hash(GcPtr::new(map.persistent()?))))
    }

    pub fn message(&self) -> String {
        self.message.to_string()
    }

    pub fn data(&self) -> Option<MapValue> {
        self.data.as_ref().cloned()
    }

    pub fn cause(&self) -> Option<GcPtr<ExceptionInfo>> {
        self.cause.as_ref().cloned()
    }
}

impl Trace for ExceptionInfo {
    fn trace(&self, visitor: &mut MarkVisitor) {
        if let Some(cause) = self.cause.as_ref() {
            visitor.visit(cause);
        }
    }
}

impl ClojureHash for ExceptionInfo {
    fn clojure_hash(&self) -> u32 {
        let msg_hash = hash_string(self.message.as_ref());
        let cause_hash = self
            .cause
            .as_ref()
            .map(|c| c.get().clojure_hash())
            .unwrap_or(0);
        hash_combine_ordered(msg_hash, cause_hash)
    }
}
