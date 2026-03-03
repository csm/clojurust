//! Stub types for Phase 4/7 that are referenced by the Value enum.
//! These will be fleshed out in their respective phases.

#![allow(unused)]

use std::sync::{Arc, Mutex};

use cljx_gc::GcPtr;

use crate::Value;
use crate::collections::hash_map::PersistentHashMap;

// ── Var ───────────────────────────────────────────────────────────────────────

/// A Clojure var — a namespace-interned mutable root binding.
/// Phase 4 will add namespace resolution, metadata, and dynamic binding.
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
/// Phase 7 will add swap!, reset!, add-watch.
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

/// A Clojure namespace. Phase 4 will add intern tables and aliasing.
#[derive(Debug)]
pub struct Namespace {
    pub name: Arc<str>,
}

impl Namespace {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self { name: name.into() }
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

// ── CljxFn ────────────────────────────────────────────────────────────────────

/// An interpreted Clojure closure. Phase 4 will add the body (Form AST).
#[derive(Debug)]
pub struct CljxFn {
    pub name: Option<Arc<str>>,
    pub params: Vec<crate::symbol::Symbol>,
    pub is_variadic: bool,
    /// Captured environment values, in declaration order.
    pub closed_over: Vec<Value>,
}

impl CljxFn {
    pub fn new(
        name: Option<Arc<str>>,
        params: Vec<crate::symbol::Symbol>,
        is_variadic: bool,
        closed_over: Vec<Value>,
    ) -> Self {
        Self {
            name,
            params,
            is_variadic,
            closed_over,
        }
    }
}

impl cljx_gc::Trace for CljxFn {}
