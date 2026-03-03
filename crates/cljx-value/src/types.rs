//! Stub types for Phase 4/7 that are referenced by the Value enum.

#![allow(unused)]

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Mutex};

use cljx_gc::GcPtr;
use cljx_reader::Form;

use crate::Value;

// ── Var ───────────────────────────────────────────────────────────────────────

/// A Clojure var — a namespace-interned mutable root binding.
#[derive(Debug)]
pub struct Var {
    pub namespace: Arc<str>,
    pub name: Arc<str>,
    pub value: Mutex<Option<Value>>,
    pub is_macro: bool,
}

impl Var {
    pub fn new(namespace: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: Mutex::new(None),
            is_macro: false,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }

    pub fn deref(&self) -> Option<Value> {
        self.value.lock().unwrap().clone()
    }

    pub fn bind(&self, v: Value) {
        *self.value.lock().unwrap() = Some(v);
    }
}

impl cljx_gc::Trace for Var {}

// ── Atom ──────────────────────────────────────────────────────────────────────

/// A Clojure atom — a thread-safe mutable reference.
#[derive(Debug)]
pub struct Atom {
    pub value: Mutex<Value>,
}

impl Atom {
    pub fn new(v: Value) -> Self {
        Self {
            value: Mutex::new(v),
        }
    }

    pub fn deref(&self) -> Value {
        self.value.lock().unwrap().clone()
    }

    pub fn reset(&self, v: Value) -> Value {
        let mut guard = self.value.lock().unwrap();
        *guard = v.clone();
        v
    }
}

impl cljx_gc::Trace for Atom {}

// ── Namespace ─────────────────────────────────────────────────────────────────

/// A Clojure namespace with intern table, refers, and aliases.
#[derive(Debug)]
pub struct Namespace {
    pub name: Arc<str>,
    /// Vars interned directly in this namespace.
    pub interns: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,
    /// Vars referred from other namespaces (e.g. clojure.core).
    pub refers: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,
    /// Namespace aliases: short-name → full namespace name.
    pub aliases: Mutex<HashMap<Arc<str>, Arc<str>>>,
}

impl Namespace {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            interns: Mutex::new(HashMap::new()),
            refers: Mutex::new(HashMap::new()),
            aliases: Mutex::new(HashMap::new()),
        }
    }
}

impl cljx_gc::Trace for Namespace {}

// ── NativeFn ──────────────────────────────────────────────────────────────────

/// A Rust function callable from Clojure.
pub type NativeFnPtr = fn(&[Value]) -> crate::error::ValueResult<Value>;

#[derive(Clone, Debug)]
pub enum Arity {
    Fixed(usize),
    Variadic { min: usize },
}

#[derive(Debug)]
pub struct NativeFn {
    pub name: Arc<str>,
    pub arity: Arity,
    pub func: NativeFnPtr,
}

impl NativeFn {
    pub fn new(name: impl Into<Arc<str>>, arity: Arity, func: NativeFnPtr) -> Self {
        Self {
            name: name.into(),
            arity,
            func,
        }
    }
}

impl cljx_gc::Trace for NativeFn {}

// ── CljxFnArity ───────────────────────────────────────────────────────────────

/// One arity branch of a Clojure function.
#[derive(Debug, Clone)]
pub struct CljxFnArity {
    /// Simple parameter names (no `&`).
    pub params: Vec<Arc<str>>,
    /// The name after `&`, if any.
    pub rest_param: Option<Arc<str>>,
    /// The body forms for this arity.
    pub body: Vec<Form>,
}

// ── CljxFn ────────────────────────────────────────────────────────────────────

/// An interpreted Clojure closure with captured environment.
#[derive(Debug, Clone)]
pub struct CljxFn {
    pub name: Option<Arc<str>>,
    pub arities: Vec<CljxFnArity>,
    /// Names of closed-over bindings (parallel to `closed_over_vals`).
    pub closed_over_names: Vec<Arc<str>>,
    /// Values of closed-over bindings (parallel to `closed_over_names`).
    pub closed_over_vals: Vec<Value>,
    /// True if this function was defined with `defmacro`.
    pub is_macro: bool,
}

impl CljxFn {
    pub fn new(
        name: Option<Arc<str>>,
        arities: Vec<CljxFnArity>,
        closed_over_names: Vec<Arc<str>>,
        closed_over_vals: Vec<Value>,
        is_macro: bool,
    ) -> Self {
        Self {
            name,
            arities,
            closed_over_names,
            closed_over_vals,
            is_macro,
        }
    }
}

impl cljx_gc::Trace for CljxFn {}

// ── Thunk / LazySeq ───────────────────────────────────────────────────────────

/// A deferred computation that produces a `Value` when forced.
pub trait Thunk: Send + Sync + std::fmt::Debug {
    fn force(&self) -> Value;
}

/// Internal state of a lazy sequence cell.
pub enum LazySeqState {
    /// Thunk not yet evaluated.
    Pending(Box<dyn Thunk>),
    /// Result cached after first force.
    Forced(Value),
}

/// A lazy sequence that forces its thunk exactly once and caches the result.
pub struct LazySeq {
    pub state: Mutex<LazySeqState>,
}

impl LazySeq {
    pub fn new(thunk: Box<dyn Thunk>) -> Self {
        Self {
            state: Mutex::new(LazySeqState::Pending(thunk)),
        }
    }

    /// Realize the sequence: force the thunk on first call, return cached value on subsequent calls.
    pub fn realize(&self) -> Value {
        let mut guard = self.state.lock().unwrap();
        if let LazySeqState::Forced(v) = &*guard {
            return v.clone();
        }
        // Replace the pending state with a temporary Forced(Nil), then force the thunk.
        let prev = mem::replace(&mut *guard, LazySeqState::Forced(Value::Nil));
        let LazySeqState::Pending(thunk) = prev else {
            unreachable!("state was not Pending")
        };
        let result = thunk.force();
        *guard = LazySeqState::Forced(result.clone());
        result
    }
}

impl std::fmt::Debug for LazySeq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LazySeq(...)")
    }
}

impl cljx_gc::Trace for LazySeq {}

// ── CljxCons ──────────────────────────────────────────────────────────────────

/// A lazy cons cell: head element + tail (may be a `LazySeq`, `List`, or `Nil`).
///
/// Used when `cons` is called with a `LazySeq` or `Cons` tail, enabling lazy
/// sequences without eagerly realizing them.
#[derive(Debug, Clone)]
pub struct CljxCons {
    pub head: Value,
    pub tail: Value,
}

impl cljx_gc::Trace for CljxCons {}
