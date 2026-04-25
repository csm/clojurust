//! Stub types for Phase 4/7 that are referenced by the Value enum.

#![allow(unused)]

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Condvar, Mutex};

use cljrs_gc::GcPtr;
use cljrs_reader::Form;

use crate::Value;

// ── No-GC debug provenance helper ────────────────────────────────────────────

/// In `no-gc` debug builds: return `true` if the top-level `GcPtr` inside
/// `value` (if any) was allocated by the global `StaticArena`.
///
/// Primitives (`Nil`, `Bool`, `Long`, `Double`, `Char`) contain no `GcPtr`
/// and always return `true`.  `Resource` is Arc-managed and also returns
/// `true`.  All other variants have a `GcPtr` that is checked against the
/// static arena's chunk range.
///
/// This check is intentionally **shallow** (top-level pointer only).  If the
/// value was produced inside a `StaticCtxGuard`, ALL allocations during its
/// evaluation go to the static arena — so a static top-level pointer implies
/// static contents.
#[cfg(all(feature = "no-gc", debug_assertions))]
pub(crate) fn value_gcptr_is_static(value: &Value) -> bool {
    use crate::value::MapValue;
    use crate::value::SetValue;
    match value {
        // Inline scalars — no GcPtr.
        Value::Nil
        | Value::Bool(_)
        | Value::Long(_)
        | Value::Double(_)
        | Value::Char(_)
        | Value::Uuid(_) => true,
        // Arc-managed — not GcPtr.
        Value::Resource(_) => true,
        // GcPtr variants.
        Value::BigInt(p) => p.is_static_alloc(),
        Value::BigDecimal(p) => p.is_static_alloc(),
        Value::Ratio(p) => p.is_static_alloc(),
        Value::Str(p) => p.is_static_alloc(),
        Value::Pattern(p) => p.is_static_alloc(),
        Value::Matcher(p) => p.is_static_alloc(),
        Value::Symbol(p) => p.is_static_alloc(),
        Value::Keyword(p) => p.is_static_alloc(),
        Value::List(p) => p.is_static_alloc(),
        Value::Vector(p) => p.is_static_alloc(),
        Value::Queue(p) => p.is_static_alloc(),
        Value::Map(m) => match m {
            MapValue::Array(p) => p.is_static_alloc(),
            MapValue::Hash(p) => p.is_static_alloc(),
            MapValue::Sorted(p) => p.is_static_alloc(),
        },
        Value::Set(s) => match s {
            SetValue::Hash(p) => p.is_static_alloc(),
            SetValue::Sorted(p) => p.is_static_alloc(),
        },
        Value::NativeFunction(p) => p.is_static_alloc(),
        Value::Fn(p) | Value::Macro(p) => p.is_static_alloc(),
        Value::BoundFn(p) => p.is_static_alloc(),
        Value::Var(p) => p.is_static_alloc(),
        Value::Atom(p) => p.is_static_alloc(),
        Value::Namespace(p) => p.is_static_alloc(),
        Value::LazySeq(p) => p.is_static_alloc(),
        Value::Cons(p) => p.is_static_alloc(),
        Value::Protocol(p) => p.is_static_alloc(),
        Value::ProtocolFn(p) => p.is_static_alloc(),
        Value::MultiFn(p) => p.is_static_alloc(),
        Value::Volatile(p) => p.is_static_alloc(),
        Value::Delay(p) => p.is_static_alloc(),
        Value::Promise(p) => p.is_static_alloc(),
        Value::Future(p) => p.is_static_alloc(),
        Value::Agent(p) => p.is_static_alloc(),
        Value::TypeInstance(p) => p.is_static_alloc(),
        Value::ObjectArray(p) => p.is_static_alloc(),
        Value::NativeObject(p) => p.is_static_alloc(),
        Value::Error(p) => p.is_static_alloc(),
        Value::TransientMap(p) => p.is_static_alloc(),
        Value::TransientVector(p) => p.is_static_alloc(),
        Value::TransientSet(p) => p.is_static_alloc(),
        // Primitive arrays — no meaningful pointer check needed.
        Value::BooleanArray(_)
        | Value::ByteArray(_)
        | Value::ShortArray(_)
        | Value::IntArray(_)
        | Value::LongArray(_)
        | Value::FloatArray(_)
        | Value::DoubleArray(_)
        | Value::CharArray(_) => true,
        // Wrapper variants.
        Value::Reduced(inner) | Value::WithMeta(inner, _) => value_gcptr_is_static(inner),
    }
}

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

impl cljrs_gc::Trace for Protocol {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let impls = self.impls.lock().unwrap();
            for method_map in impls.values() {
                for v in method_map.values() {
                    v.trace(visitor);
                }
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

impl cljrs_gc::Trace for ProtocolMethod {
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
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

impl cljrs_gc::Trace for ProtocolFn {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
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

impl cljrs_gc::Trace for MultiFn {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.dispatch_fn.trace(visitor);
        {
            let methods = self.methods.lock().unwrap();
            for v in methods.values() {
                v.trace(visitor);
            }
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
    pub watches: Mutex<Vec<(Value, Value)>>,
}

impl Var {
    pub fn new(namespace: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: Mutex::new(None),
            is_macro: false,
            meta: Mutex::new(None),
            watches: Mutex::new(Vec::new()),
        }
    }

    pub fn is_bound(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }

    pub fn deref(&self) -> Option<Value> {
        self.value.lock().unwrap().clone()
    }

    pub fn bind(&self, v: Value) {
        // In no-gc debug builds: assert the value being stored in this
        // program-lifetime Var came from the StaticArena, not a scratch region.
        // A region-local pointer would dangle after the function returns.
        #[cfg(all(feature = "no-gc", debug_assertions))]
        debug_assert!(
            value_gcptr_is_static(&v),
            "no-gc: Var::bind({}/{}) received a region-local value — store violations \
             indicate a missing StaticCtxGuard around the value expression",
            self.namespace,
            self.name
        );
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

impl cljrs_gc::Trace for Var {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let value = self.value.lock().unwrap();
            if let Some(v) = value.as_ref() {
                v.trace(visitor);
            }
        }
        {
            let meta = self.meta.lock().unwrap();
            if let Some(m) = meta.as_ref() {
                m.trace(visitor);
            }
        }
        {
            let watches = self.watches.lock().unwrap();
            for (key, f) in watches.iter() {
                key.trace(visitor);
                f.trace(visitor);
            }
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
    pub watches: Mutex<Vec<(Value, Value)>>,
}

impl Atom {
    pub fn new(v: Value) -> Self {
        Self {
            value: Mutex::new(v),
            meta: Mutex::new(None),
            validator: Mutex::new(None),
            watches: Mutex::new(Vec::new()),
        }
    }

    pub fn deref(&self) -> Value {
        self.value.lock().unwrap().clone()
    }

    pub fn reset(&self, v: Value) -> Value {
        // In no-gc debug builds: assert the new value came from the StaticArena.
        #[cfg(all(feature = "no-gc", debug_assertions))]
        debug_assert!(
            value_gcptr_is_static(&v),
            "no-gc: Atom::reset() received a region-local value — the new-value \
             expression must be computed inside a StaticCtxGuard (i.e. inside \
             the swap! / reset! call) so it is allocated in the static arena"
        );
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

impl cljrs_gc::Trace for Atom {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let value = self.value.lock().unwrap();
            value.trace(visitor);
        }
        {
            let meta = self.meta.lock().unwrap();
            if let Some(m) = meta.as_ref() {
                m.trace(visitor);
            }
        }
        {
            let validator = self.validator.lock().unwrap();
            if let Some(vf) = validator.as_ref() {
                vf.trace(visitor);
            }
        }
        {
            let watches = self.watches.lock().unwrap();
            for (key, f) in watches.iter() {
                key.trace(visitor);
                f.trace(visitor);
            }
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

impl cljrs_gc::Trace for Namespace {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
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
/// Legacy type alias kept for source compatibility. Bare `fn` pointers
/// implement `Fn` and can be passed anywhere a `NativeFnFunc` is expected.
pub type NativeFnPtr = fn(&[Value]) -> crate::error::ValueResult<Value>;

/// The callable stored inside a `NativeFn`. Supports both bare function
/// pointers and closures that capture state.
pub type NativeFnFunc = Arc<dyn Fn(&[Value]) -> crate::error::ValueResult<Value> + Send + Sync>;

#[derive(Clone, Debug)]
pub enum Arity {
    Fixed(usize),
    Variadic { min: usize },
}

pub struct NativeFn {
    pub name: Arc<str>,
    pub arity: Arity,
    pub func: NativeFnFunc,
}

impl NativeFn {
    /// Create from a bare function pointer (backwards-compatible).
    pub fn new(name: impl Into<Arc<str>>, arity: Arity, func: NativeFnPtr) -> Self {
        Self {
            name: name.into(),
            arity,
            func: Arc::new(func),
        }
    }

    /// Create from a closure or any `Fn(&[Value]) -> ValueResult<Value>`.
    pub fn with_closure(
        name: impl Into<Arc<str>>,
        arity: Arity,
        func: impl Fn(&[Value]) -> crate::error::ValueResult<Value> + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            arity,
            func: Arc::new(func),
        }
    }
}

impl std::fmt::Debug for NativeFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeFn")
            .field("name", &self.name)
            .field("arity", &self.arity)
            .field("func", &"<fn>")
            .finish()
    }
}

impl cljrs_gc::Trace for NativeFn {
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
}

// ── CljxFnArity ───────────────────────────────────────────────────────────────

/// One arity branch of a Clojure function.
#[derive(Debug, Clone)]
pub struct CljxFnArity {
    /// Simple parameter names (no `&`).
    /// For destructured params, these are gensym'd names.
    pub params: Vec<Arc<str>>,
    /// The name after `&`, if any.
    pub rest_param: Option<Arc<str>>,
    /// The body forms for this arity.
    pub body: Vec<Form>,
    /// Destructuring patterns: (param_index, original_form).
    /// After binding the gensym'd param, these patterns are applied
    /// via `bind_pattern` to destructure the value.
    pub destructure_params: Vec<(usize, Form)>,
    /// If the rest param is destructured, the original form.
    pub destructure_rest: Option<Form>,
    /// Unique ID for IR cache lookup (assigned by the evaluator).
    pub ir_arity_id: u64,
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

impl cljrs_gc::Trace for CljxFn {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for v in &self.closed_over_vals {
            v.trace(visitor);
        }
    }
}

// ── BoundFn ──────────────────────────────────────────────────────────────────

/// A function wrapped with captured dynamic bindings.
/// When called, the captured bindings are pushed as a frame before delegating
/// to the wrapped function. This means captured bindings override the caller's
/// for the same var, but vars not in the capture fall through normally.
#[derive(Debug)]
pub struct BoundFn {
    /// The wrapped callable.
    pub wrapped: Value,
    /// Captured dynamic bindings (merged flat frame; opaque to cljrs-value).
    pub captured_bindings: HashMap<usize, Value>,
}

impl cljrs_gc::Trace for BoundFn {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.wrapped.trace(visitor);
        for val in self.captured_bindings.values() {
            val.trace(visitor);
        }
    }
}

// ── Thunk / LazySeq ───────────────────────────────────────────────────────────

/// A deferred computation that produces a `Value` when forced.
pub trait Thunk: Send + Sync + std::fmt::Debug + cljrs_gc::Trace {
    fn force(&self) -> Result<Value, String>;
}

/// Internal state of a lazy sequence cell.
pub enum LazySeqState {
    /// Thunk not yet evaluated.
    Pending(Box<dyn Thunk>),
    /// Result cached after first force.
    Forced(Value),
    /// Thunk evaluation failed; error message is cached.
    Error(String),
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
    /// On error, returns `Value::Nil` and caches the error (retrievable via `error()`).
    pub fn realize(&self) -> Value {
        let thunk = {
            let mut guard = self.state.lock().unwrap();
            match &*guard {
                LazySeqState::Forced(v) => return v.clone(),
                LazySeqState::Error(_) => return Value::Nil,
                LazySeqState::Pending(_) => {}
            }
            // Replace the pending state with a temporary Forced(Nil), extract the thunk.
            let prev = mem::replace(&mut *guard, LazySeqState::Forced(Value::Nil));
            let LazySeqState::Pending(thunk) = prev else {
                unreachable!("state was not Pending")
            };
            thunk
            // guard dropped here — lock released before forcing
        };
        // Force the thunk WITHOUT holding the lock. This ensures GC's
        // lock().unwrap() in LazySeq::trace() will not deadlock.
        match thunk.force() {
            Ok(result) => {
                *self.state.lock().unwrap() = LazySeqState::Forced(result.clone());
                result
            }
            Err(msg) => {
                *self.state.lock().unwrap() = LazySeqState::Error(msg);
                Value::Nil
            }
        }
    }

    /// Return the cached error message, if the thunk failed.
    pub fn error(&self) -> Option<String> {
        let guard = self.state.lock().unwrap();
        if let LazySeqState::Error(e) = &*guard {
            Some(e.clone())
        } else {
            None
        }
    }
}

impl std::fmt::Debug for LazySeq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LazySeq(...)")
    }
}

impl cljrs_gc::Trace for LazySeq {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        // Safe to lock unconditionally: realize() drops the lock before entering
        // eval (thunk.force()), so the lock is never held across a GC safepoint.
        {
            let state = self.state.lock().unwrap();
            match &*state {
                LazySeqState::Pending(thunk) => thunk.trace(visitor),
                LazySeqState::Forced(v) => v.trace(visitor),
                LazySeqState::Error(_) => {}
            }
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

impl cljrs_gc::Trace for CljxCons {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
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
        // In no-gc debug builds: assert the new value came from the StaticArena.
        #[cfg(all(feature = "no-gc", debug_assertions))]
        debug_assert!(
            value_gcptr_is_static(&v),
            "no-gc: Volatile::reset() received a region-local value — ensure the \
             new-value expression is inside a StaticCtxGuard (vreset! handles this)"
        );
        *self.value.lock().unwrap() = v.clone();
        v
    }
}

impl std::fmt::Debug for Volatile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Volatile")
    }
}

impl cljrs_gc::Trace for Volatile {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let value = self.value.lock().unwrap();
            value.trace(visitor);
        }
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
    /// Returns the value on success, or an error message on failure.
    pub fn force(&self) -> Result<Value, String> {
        let thunk = {
            let mut guard = self.state.lock().unwrap();
            if let DelayState::Forced(v) = &*guard {
                return Ok(v.clone());
            }
            let prev = mem::replace(&mut *guard, DelayState::Forced(Value::Nil));
            let DelayState::Pending(thunk) = prev else {
                unreachable!("state was not Pending")
            };
            thunk
            // guard dropped here — lock released before forcing
        };
        // Force the thunk WITHOUT holding the lock so GC's lock().unwrap() in
        // Delay::trace() will not deadlock.
        let result = thunk.force()?;
        *self.state.lock().unwrap() = DelayState::Forced(result.clone());
        Ok(result)
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

impl cljrs_gc::Trace for Delay {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        // Safe to lock unconditionally: force() drops the lock before entering
        // eval (thunk.force()), so the lock is never held across a GC safepoint.
        {
            let state = self.state.lock().unwrap();
            match &*state {
                DelayState::Pending(thunk) => thunk.trace(visitor),
                DelayState::Forced(v) => v.trace(visitor),
            }
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

impl cljrs_gc::Trace for CljxPromise {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let value = self.value.lock().unwrap();
            if let Some(v) = value.as_ref() {
                v.trace(visitor);
            }
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

impl cljrs_gc::Trace for CljxFuture {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let state = self.state.lock().unwrap();
            if let FutureState::Done(v) = &*state {
                v.trace(visitor);
            }
        }
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

/// A Clojure agent action: takes the current state, returns the new state.
pub type AgentFn = Box<dyn FnOnce(Value) -> Result<Value, Value> + Send>;

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
    pub error: Arc<Mutex<Option<Value>>>,
    /// Channel to send actions to the worker thread.
    pub sender: Mutex<std::sync::mpsc::SyncSender<AgentMsg>>,
    pub watches: Mutex<Vec<(Value, Value)>>,
}

impl Agent {
    pub fn get_state(&self) -> Value {
        self.state.lock().unwrap().clone()
    }

    pub fn get_error(&self) -> Option<Value> {
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

impl cljrs_gc::Trace for Agent {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        {
            let state = self.state.lock().unwrap();
            state.trace(visitor);
        }
        {
            let error = self.error.lock().unwrap();
            if let Some(e) = error.as_ref() {
                e.trace(visitor);
            }
        }
        {
            let watches = self.watches.lock().unwrap();
            for (key, f) in watches.iter() {
                key.trace(visitor);
                f.trace(visitor);
            }
        }
    }
}
