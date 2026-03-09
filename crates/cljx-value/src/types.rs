//! Stub types for Phase 4/7 that are referenced by the Value enum.

#![allow(unused)]

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Condvar, Mutex};

use cljx_gc::GcPtr;
use cljx_reader::Form;

use crate::Value;

// ── Protocol ──────────────────────────────────────────────────────────────────

/// Inner map type for protocol implementations: method_name → impl fn.
pub type MethodMap = HashMap<Arc<str>, Value>;

/// A Clojure protocol — an interface-like construct with named methods.
#[derive(Debug)]
pub struct Protocol {
    pub name: Arc<str>,
    pub ns: Arc<str>,
    pub methods: Vec<ProtocolMethod>,
    /// type_tag → { method_name → impl fn }
    pub impls: Mutex<HashMap<Arc<str>, MethodMap>>,
}

impl Protocol {
    pub fn new(name: Arc<str>, ns: Arc<str>, methods: Vec<ProtocolMethod>) -> Self {
        Self {
            name,
            ns,
            methods,
            impls: Mutex::new(HashMap::new()),
        }
    }
}

impl cljx_gc::Trace for Protocol {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        let impls = self.impls.lock().unwrap();
        for method_map in impls.values() {
            for v in method_map.values() {
                v.trace(visitor);
            }
        }
    }
}

/// One method signature declared in a `defprotocol`.
#[derive(Debug, Clone)]
pub struct ProtocolMethod {
    pub name: Arc<str>,
    pub min_arity: usize,
    pub variadic: bool,
}

impl cljx_gc::Trace for ProtocolMethod {
    fn trace(&self, _: &mut cljx_gc::MarkVisitor) {}
}

// ── ProtocolFn ────────────────────────────────────────────────────────────────

/// Callable that dispatches a single protocol method on the type of `args[0]`.
#[derive(Debug)]
pub struct ProtocolFn {
    pub protocol: GcPtr<Protocol>,
    pub method_name: Arc<str>,
    pub min_arity: usize,
    pub variadic: bool,
}

impl cljx_gc::Trace for ProtocolFn {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        use cljx_gc::GcVisitor as _;
        visitor.visit(&self.protocol);
    }
}

// ── MultiFn ───────────────────────────────────────────────────────────────────

/// A Clojure multimethod — arbitrary dispatch via a user-supplied function.
#[derive(Debug)]
pub struct MultiFn {
    pub name: Arc<str>,
    pub dispatch_fn: Value,
    /// pr_str(dispatch-val) → implementation fn
    pub methods: Mutex<HashMap<String, Value>>,
    /// recorded preferences (for future derive/hierarchy)
    pub prefers: Mutex<HashMap<String, Vec<String>>>,
    /// normally ":default"
    pub default_dispatch: String,
}

impl MultiFn {
    pub fn new(name: Arc<str>, dispatch_fn: Value, default_dispatch: String) -> Self {
        Self {
            name,
            dispatch_fn,
            methods: Mutex::new(HashMap::new()),
            prefers: Mutex::new(HashMap::new()),
            default_dispatch,
        }
    }
}

impl cljx_gc::Trace for MultiFn {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        self.dispatch_fn.trace(visitor);
        let methods = self.methods.lock().unwrap();
        for v in methods.values() {
            v.trace(visitor);
        }
    }
}

// ── Var ───────────────────────────────────────────────────────────────────────

/// A Clojure var — a namespace-interned mutable root binding.
#[derive(Debug)]
pub struct Var {
    pub namespace: Arc<str>,
    pub name: Arc<str>,
    pub value: Mutex<Option<Value>>,
    pub is_macro: bool,
    /// Metadata map (e.g. `{:dynamic true}`).
    pub meta: Mutex<Option<Value>>,
}

impl Var {
    pub fn new(namespace: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: Mutex::new(None),
            is_macro: false,
            meta: Mutex::new(None),
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

    pub fn get_meta(&self) -> Option<Value> {
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Value) {
        *self.meta.lock().unwrap() = Some(m);
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

impl cljx_gc::Trace for Var {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        if let Some(v) = self.value.lock().unwrap().as_ref() {
            v.trace(visitor);
        }
        if let Some(m) = self.meta.lock().unwrap().as_ref() {
            m.trace(visitor);
        }
    }
}

// ── Atom ──────────────────────────────────────────────────────────────────────

/// A Clojure atom — a thread-safe mutable reference.
#[derive(Debug)]
pub struct Atom {
    pub value: Mutex<Value>,
    pub meta: Mutex<Option<Value>>,
    pub validator: Mutex<Option<Value>>,
}

impl Atom {
    pub fn new(v: Value) -> Self {
        Self {
            value: Mutex::new(v),
            meta: Mutex::new(None),
            validator: Mutex::new(None),
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

    pub fn get_meta(&self) -> Option<Value> {
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Option<Value>) {
        *self.meta.lock().unwrap() = m;
    }

    pub fn get_validator(&self) -> Option<Value> {
        self.validator.lock().unwrap().clone()
    }

    pub fn set_validator(&self, vf: Option<Value>) {
        *self.validator.lock().unwrap() = vf;
    }
}

impl cljx_gc::Trace for Atom {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        self.value.lock().unwrap().trace(visitor);
        if let Some(m) = self.meta.lock().unwrap().as_ref() {
            m.trace(visitor);
        }
        if let Some(vf) = self.validator.lock().unwrap().as_ref() {
            vf.trace(visitor);
        }
    }
}

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

impl cljx_gc::Trace for Namespace {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        use cljx_gc::GcVisitor as _;
        {
            let interns = self.interns.lock().unwrap();
            for var in interns.values() {
                visitor.visit(var);
            }
        }
        {
            let refers = self.refers.lock().unwrap();
            for var in refers.values() {
                visitor.visit(var);
            }
        }
    }
}

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

impl cljx_gc::Trace for NativeFn {
    fn trace(&self, _: &mut cljx_gc::MarkVisitor) {}
}

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
    /// Namespace in which this function was defined (for macro hygiene).
    pub defining_ns: Arc<str>,
}

impl CljxFn {
    pub fn new(
        name: Option<Arc<str>>,
        arities: Vec<CljxFnArity>,
        closed_over_names: Vec<Arc<str>>,
        closed_over_vals: Vec<Value>,
        is_macro: bool,
        defining_ns: Arc<str>,
    ) -> Self {
        Self {
            name,
            arities,
            closed_over_names,
            closed_over_vals,
            is_macro,
            defining_ns,
        }
    }
}

impl cljx_gc::Trace for CljxFn {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        for v in &self.closed_over_vals {
            v.trace(visitor);
        }
    }
}

// ── Thunk / LazySeq ───────────────────────────────────────────────────────────

/// A deferred computation that produces a `Value` when forced.
pub trait Thunk: Send + Sync + std::fmt::Debug + cljx_gc::Trace {
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

impl cljx_gc::Trace for LazySeq {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        let state = self.state.lock().unwrap();
        match &*state {
            LazySeqState::Pending(thunk) => thunk.trace(visitor),
            LazySeqState::Forced(v) => v.trace(visitor),
        }
    }
}

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

impl cljx_gc::Trace for CljxCons {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        self.head.trace(visitor);
        self.tail.trace(visitor);
    }
}

// ── Volatile ──────────────────────────────────────────────────────────────────

/// Non-atomic mutable cell (single-thread performance, no CAS).
pub struct Volatile {
    pub value: Mutex<Value>,
}

impl Volatile {
    pub fn new(v: Value) -> Self {
        Self {
            value: Mutex::new(v),
        }
    }

    pub fn deref(&self) -> Value {
        self.value.lock().unwrap().clone()
    }

    pub fn reset(&self, v: Value) -> Value {
        *self.value.lock().unwrap() = v.clone();
        v
    }
}

impl std::fmt::Debug for Volatile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Volatile")
    }
}

impl cljx_gc::Trace for Volatile {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        self.value.lock().unwrap().trace(visitor);
    }
}

// ── Delay ─────────────────────────────────────────────────────────────────────

/// Internal state of a delay cell.
pub enum DelayState {
    Pending(Box<dyn Thunk>),
    Forced(Value),
}

/// A lazy one-time computation (forced at most once, result cached).
pub struct Delay {
    pub state: Mutex<DelayState>,
}

impl Delay {
    pub fn new(thunk: Box<dyn Thunk>) -> Self {
        Self {
            state: Mutex::new(DelayState::Pending(thunk)),
        }
    }

    /// Force the delay and cache the result.
    pub fn force(&self) -> Value {
        let mut guard = self.state.lock().unwrap();
        if let DelayState::Forced(v) = &*guard {
            return v.clone();
        }
        let prev = mem::replace(&mut *guard, DelayState::Forced(Value::Nil));
        let DelayState::Pending(thunk) = prev else {
            unreachable!("state was not Pending")
        };
        let result = thunk.force();
        *guard = DelayState::Forced(result.clone());
        result
    }

    /// True if the delay has already been forced.
    pub fn is_realized(&self) -> bool {
        matches!(&*self.state.lock().unwrap(), DelayState::Forced(_))
    }
}

impl std::fmt::Debug for Delay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Delay")
    }
}

impl cljx_gc::Trace for Delay {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        let state = self.state.lock().unwrap();
        match &*state {
            DelayState::Pending(thunk) => thunk.trace(visitor),
            DelayState::Forced(v) => v.trace(visitor),
        }
    }
}

// ── CljxPromise ───────────────────────────────────────────────────────────────

/// A one-shot rendezvous (promise).
pub struct CljxPromise {
    pub value: Mutex<Option<Value>>,
    pub cond: Condvar,
}

impl CljxPromise {
    pub fn new() -> Self {
        Self {
            value: Mutex::new(None),
            cond: Condvar::new(),
        }
    }

    /// Deliver a value (no-op if already delivered).
    pub fn deliver(&self, v: Value) {
        let mut guard = self.value.lock().unwrap();
        if guard.is_none() {
            *guard = Some(v);
            self.cond.notify_all();
        }
    }

    /// Block until a value is available, then return it.
    pub fn deref_blocking(&self) -> Value {
        let mut guard = self.value.lock().unwrap();
        while guard.is_none() {
            guard = self.cond.wait(guard).unwrap();
        }
        guard.as_ref().unwrap().clone()
    }

    /// True if already delivered.
    pub fn is_realized(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }
}

impl Default for CljxPromise {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CljxPromise {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Promise")
    }
}

impl cljx_gc::Trace for CljxPromise {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        if let Some(v) = self.value.lock().unwrap().as_ref() {
            v.trace(visitor);
        }
    }
}

// ── CljxFuture ────────────────────────────────────────────────────────────────

/// Thread-pool future state.
pub enum FutureState {
    Running,
    Done(Value),
    Failed(String),
    Cancelled,
}

/// A future value computed asynchronously on another thread.
pub struct CljxFuture {
    pub state: Mutex<FutureState>,
    pub cond: Condvar,
}

impl CljxFuture {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FutureState::Running),
            cond: Condvar::new(),
        }
    }

    /// True if done, failed, or cancelled (not still running).
    pub fn is_done(&self) -> bool {
        !matches!(&*self.state.lock().unwrap(), FutureState::Running)
    }

    /// True if explicitly cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(&*self.state.lock().unwrap(), FutureState::Cancelled)
    }
}

impl Default for CljxFuture {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CljxFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Future")
    }
}

impl cljx_gc::Trace for CljxFuture {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        let state = self.state.lock().unwrap();
        if let FutureState::Done(v) = &*state {
            v.trace(visitor);
        }
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

/// A Clojure agent action: takes the current state, returns the new state.
pub type AgentFn = Box<dyn FnOnce(Value) -> Result<Value, String> + Send>;

/// Messages sent to an agent's worker thread.
pub enum AgentMsg {
    Update(AgentFn),
    Shutdown,
}

/// A Clojure agent — asynchronous state update queue.
pub struct Agent {
    /// Current state, shared between the Value::Agent handle and the worker thread.
    pub state: Arc<Mutex<Value>>,
    /// Last error, shared similarly.
    pub error: Arc<Mutex<Option<String>>>,
    /// Channel to send actions to the worker thread.
    pub sender: Mutex<std::sync::mpsc::SyncSender<AgentMsg>>,
}

impl Agent {
    pub fn get_state(&self) -> Value {
        self.state.lock().unwrap().clone()
    }

    pub fn get_error(&self) -> Option<String> {
        self.error.lock().unwrap().clone()
    }

    pub fn clear_error(&self) {
        *self.error.lock().unwrap() = None;
    }
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Agent")
    }
}

impl cljx_gc::Trace for Agent {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        // Trace through the Arc<Mutex<Value>> — the worker thread shares this Arc.
        self.state.lock().unwrap().trace(visitor);
    }
}
