//! Stub types for Phase 4/7 that are referenced by the Value enum.

#![allow(unused)]

use std::collections::HashMap;
use std::mem;
use std::sync::{Arc, Condvar, Mutex};

use cljrs_gc::GcPtr;
use cljrs_reader::Form;

use crate::TypeHint;
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
        // Arc-managed — not GcPtr; always considered static.
        Value::Resource(_) | Value::SharedAtom(_) | Value::ByteBlob(_) => true,
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
    /// `(defprotocol Name :extend-via-metadata true ...)` — when set, protocol
    /// dispatch consults the dispatch value's metadata (keyed by this
    /// protocol's `ProtocolFn`s) before falling back to type-tag impls.
    pub extend_via_metadata: bool,
}

impl Protocol {
    pub fn new(
        name: Arc<str>,
        ns: Arc<str>,
        methods: Vec<ProtocolMethod>,
        extend_via_metadata: bool,
    ) -> Self {
        Self {
            name,
            ns,
            methods,
            impls: Mutex::new(HashMap::new()),
            extend_via_metadata,
        }
    }
}

/// Global protocol-extension generation, bumped on every `impls` mutation
/// (`extend-type`, `extend-protocol`, `defrecord`/`reify` inline impls).
///
/// Inline caches for protocol dispatch (Phase 10.6, `rt_call_ic` in
/// `cljrs-compiler`'s rt_abi) tag each cached `(dispatch type → impl fn)`
/// entry with the generation observed at fill time; a later bump invalidates
/// every cache entry at once, so re-extending a protocol mid-session is
/// picked up on the next dispatch through any call site.
static PROTOCOL_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Current protocol-extension generation (see [`bump_protocol_generation`]).
pub fn protocol_generation() -> u64 {
    PROTOCOL_GENERATION.load(std::sync::atomic::Ordering::Acquire)
}

/// Invalidate all protocol-dispatch inline caches.  Must be called after
/// every mutation of any [`Protocol::impls`] map.
pub fn bump_protocol_generation() {
    PROTOCOL_GENERATION.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
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
///
/// ## Two-tier root (Phase B3, issue #171)
///
/// A var's *root* binding uses the same two-tier mechanism as `shared-atom`:
///
/// - **`value`** — the isolate-local, GC-backed fast path.  Every var deref,
///   the IR tier, and the JIT/AOT `rt_*` ABI read this slot; promotion never
///   touches it, so compiled inline caches and pointer-identity assumptions
///   baked into native code stay valid.
/// - **`shared_root`** — a `Send + Sync` cross-isolate mirror,
///   `Arc<ArcSwap<Option<SharedValue>>>`, reusing
///   [`crate::shared::SharedValue`].  `bind` (i.e. `def` / `alter-var-root` /
///   `set!`) *promotes-on-write*: if the new root value is promotable the cell
///   holds `Some(SharedValue)`, otherwise it is cleared to `None` (option (b)
///   of the ADR — non-promotable roots, e.g. closures, stay isolate-local).
///
/// The shared cell is what crosses the structured-clone boundary: a var
/// `def`'d in one isolate is observable *by value* from another, with keyword
/// /symbol identity preserved through the intern table.  See
/// `crate::clone` for the serialize/deserialize seam.
///
/// Dynamic `binding` is unchanged — it is already thread-local / per-isolate
/// and lives on the binding stack, not in the var root.
#[derive(Debug)]
pub struct Var {
    pub namespace: Arc<str>,
    pub name: Arc<str>,
    pub value: Mutex<Option<Value>>,
    /// Cross-isolate mirror of the root binding (Phase B3).  `None` when the
    /// var is unbound or its current root is not promotable.
    pub shared_root: Arc<arc_swap::ArcSwap<Option<crate::shared::SharedValue>>>,
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
            shared_root: Arc::new(arc_swap::ArcSwap::new(Arc::new(None))),
            is_macro: false,
            meta: Mutex::new(None),
            watches: Mutex::new(Vec::new()),
        }
    }

    /// Reconstruct a var on the receiving side of an isolate boundary.
    ///
    /// The `shared_root` cell is the *same* `Arc` as the sending isolate's, so
    /// both isolates share the cross-isolate root cell.  The local `value`
    /// fast-path slot is seeded by demoting the current shared snapshot, so an
    /// immediate `deref` observes the value the var carried at crossing time.
    pub fn from_shared_root(
        namespace: impl Into<Arc<str>>,
        name: impl Into<Arc<str>>,
        is_macro: bool,
        shared_root: Arc<arc_swap::ArcSwap<Option<crate::shared::SharedValue>>>,
    ) -> Self {
        let local = shared_root
            .load()
            .as_ref()
            .as_ref()
            .map(crate::shared::demote);
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: Mutex::new(local),
            shared_root,
            is_macro,
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

    /// Read the cross-isolate root by demoting the shared cell, ignoring the
    /// isolate-local fast path.  Returns `None` when the shared root is empty
    /// (unbound or non-promotable).  Used to observe writes another isolate
    /// made through the shared cell.
    pub fn deref_shared(&self) -> Option<Value> {
        self.shared_root
            .load()
            .as_ref()
            .as_ref()
            .map(crate::shared::demote)
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
        // GC builds: heap-promotion fallback — a region-allocated value bound
        // to a program-lifetime var is deep-copied to the heap (or the active
        // regions are retired when it cannot be).  One depth check when no
        // region is open.
        let v = crate::publish::publish_value(v);
        // Replace the binding, holding the lock only across the swap.  The
        // previous value (if any) is handed to the JIT rebind hook so it can
        // reclaim native code compiled for a now-superseded definition
        // (Phase 10.2 — code unloading).  `v.clone()` is O(1) for the only
        // values that carry compiled code (`Value::Fn`, a `GcPtr` clone).
        let prev = {
            let mut slot = self.value.lock().unwrap();
            slot.replace(v.clone())
        };
        // Promote-on-`def` (Phase B3): mirror the new root into the
        // cross-isolate cell when it is promotable, else clear the cell so it
        // never advertises a stale or non-shareable root.  `def` is rare and
        // global by nature, so this write-path cost is acceptable; the read
        // path (and the JIT) never touch the shared cell.
        let shared = crate::shared::promote(&v).ok();
        self.shared_root.store(Arc::new(shared));
        if let Some(prev) = prev {
            crate::jit_hooks::notify_var_rebind(&prev, &v);
        }
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
        // Heap-promotion fallback (GC builds): an atom is program-lifetime
        // shared state, so its initial value must not be region-allocated.
        let v = crate::publish::publish_value(v);
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
        // GC builds: heap-promotion fallback (see `Var::bind`).
        let v = crate::publish::publish_value(v);
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
    /// Absolute path of the source file this namespace was loaded from,
    /// populated by the loader.  Used by the versioned resolver to locate the
    /// file for `git show`.
    pub source_file: Mutex<Option<Arc<str>>>,
    /// Absolute path of the git repository root that contains `source_file`.
    pub git_repo_root: Mutex<Option<Arc<str>>>,
    /// `true` for namespaces loaded from a specific commit (`name@hash`).
    /// Versioned namespaces are immutable: `intern()` will refuse new bindings.
    pub is_versioned: bool,
    /// Metadata attached via `(ns ^{...} name ...)` or an `ns` attr-map.
    pub meta: Mutex<Option<Value>>,
}

impl Namespace {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            interns: Mutex::new(HashMap::new()),
            refers: Mutex::new(HashMap::new()),
            aliases: Mutex::new(HashMap::new()),
            source_file: Mutex::new(None),
            git_repo_root: Mutex::new(None),
            is_versioned: false,
            meta: Mutex::new(None),
        }
    }

    /// Create a versioned (immutable) namespace for `name@commit`.
    pub fn new_versioned(name: impl Into<Arc<str>>) -> Self {
        Self {
            is_versioned: true,
            ..Self::new(name)
        }
    }

    /// Record the source file path and its git repo root (if in a repo).
    pub fn set_source_location(&self, file: &str, repo_root: Option<&str>) {
        *self.source_file.lock().unwrap() = Some(Arc::from(file));
        *self.git_repo_root.lock().unwrap() = repo_root.map(Arc::from);
    }

    pub fn get_meta(&self) -> Option<Value> {
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Value) {
        *self.meta.lock().unwrap() = Some(m);
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
        {
            let meta = self.meta.lock().unwrap();
            if let Some(m) = meta.as_ref() {
                m.trace(visitor);
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
pub type NativeFnFunc = Arc<dyn Fn(&[Value]) -> crate::error::ValueResult<Value>>;

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
        func: impl Fn(&[Value]) -> crate::error::ValueResult<Value> + 'static,
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
    /// Optional primitive type hint per positional parameter (parallel to
    /// `params`).  `^long x` → `Some(TypeHint::Long)`; an un-hinted or
    /// non-primitive-tagged param → `None`.  Drives unboxed codegen.
    pub param_hints: Vec<Option<TypeHint>>,
    /// Primitive type hint on the rest param, if any (rarely useful, but parsed
    /// for symmetry).
    pub rest_hint: Option<TypeHint>,
}

impl CljxFnArity {
    /// Heap bytes owned by this arity, not counting the `CljxFnArity` struct itself.
    pub fn heap_size(&self) -> usize {
        // params Vec buffer (Arc<str> pointers; the str data is shared, skip it)
        self.params.capacity() * mem::size_of::<Arc<str>>()
        // body: the dominant consumer — Form AST trees stored inline
        + self.body.capacity() * mem::size_of::<Form>()
        + self.body.iter().map(|f| f.heap_size()).sum::<usize>()
        // destructure_params
        + self.destructure_params.capacity() * mem::size_of::<(usize, Form)>()
        + self.destructure_params.iter().map(|(_, f)| f.heap_size()).sum::<usize>()
        // destructure_rest
        + self.destructure_rest.as_ref()
            .map_or(0, |f| mem::size_of::<Form>() + f.heap_size())
        // param_hints (Copy elements, no nested heap)
        + self.param_hints.capacity() * mem::size_of::<Option<TypeHint>>()
    }
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
    /// True if this function carries `^:async` metadata. When an async runtime
    /// (`cljrs-async`) is registered, calling such a function spawns its body as
    /// a task and returns a `Value::Future` immediately instead of running it
    /// synchronously. Without a runtime it runs synchronously like any other fn.
    pub is_async: bool,
    /// Namespace in which this function was defined (for macro hygiene).
    pub defining_ns: Arc<str>,
    /// Back-pointer to the `GcPtr` that owns this `CljxFn`, set immediately
    /// after allocation so that a named anonymous function's self-reference
    /// (e.g. `(fn g [] g)`) returns the *identical* pointer to the caller,
    /// preserving pointer-equality semantics (`(= f (f))` → `true`).
    pub self_ptr: Option<GcPtr<CljxFn>>,
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
            is_async: false,
            defining_ns,
            self_ptr: None,
        }
    }
}

impl cljrs_gc::Trace for CljxFn {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
        for v in &self.closed_over_vals {
            v.trace(visitor);
        }
        if let Some(ref p) = self.self_ptr {
            visitor.visit(p);
        }
    }

    fn gc_size_extra(&self) -> usize {
        // Vec<CljxFnArity> buffer + each arity's inline-owned heap
        self.arities.capacity() * mem::size_of::<CljxFnArity>()
            + self
                .arities
                .iter()
                .map(CljxFnArity::heap_size)
                .sum::<usize>()
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

    fn gc_size_extra(&self) -> usize {
        // HashMap<usize, Value>: hashbrown open-addressing, ~1 control byte + entry per slot.
        self.captured_bindings.capacity() * (1 + mem::size_of::<usize>() + mem::size_of::<Value>())
    }
}

// ── Thunk / LazySeq ───────────────────────────────────────────────────────────

/// A deferred computation that produces a `Value` when forced.
pub trait Thunk: std::fmt::Debug + cljrs_gc::Trace {
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
        // GC builds: heap-promotion fallback (see `Var::bind`).
        let v = crate::publish::publish_value(v);
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
        // GC builds: heap-promotion fallback (see `Var::bind`).
        let v = crate::publish::publish_value(v);
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
        // GC builds: heap-promotion fallback — the promise may outlive (and be
        // read from outside) any region scope active at delivery time.
        let v = crate::publish::publish_value(v);
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
    /// The future's body threw. Holds the thrown Clojure value (a
    /// `Value::Error`) so `await`/`deref` can re-throw it with its
    /// `ex-data`/`ex-cause` intact, rather than a stringified message.
    Failed(Value),
    Cancelled,
}

/// A future value computed asynchronously on another thread.
pub struct CljxFuture {
    pub state: Mutex<FutureState>,
    pub cond: Condvar,
    /// Set once a consumer has read the settled result (via `await`/`deref`).
    /// Used to warn about a `Failed` future that is discarded without anyone
    /// ever observing its error (the fire-and-forget footgun).
    observed: std::sync::atomic::AtomicBool,
}

impl CljxFuture {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(FutureState::Running),
            cond: Condvar::new(),
            observed: std::sync::atomic::AtomicBool::new(false),
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

    /// Mark this future's result as observed. Call when a consumer reads the
    /// settled value (`await`/`deref`), so a later drop doesn't warn about an
    /// unobserved error.
    pub fn mark_observed(&self) {
        self.observed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl Drop for CljxFuture {
    fn drop(&mut self) {
        // Warn if a future failed but nobody ever observed the error — the
        // fire-and-forget case where a thrown error would otherwise vanish.
        // Tied to GC sweep timing: only fires once the future is unreachable,
        // so a not-yet-awaited (still reachable) failed future won't warn.
        //
        // SAFETY: this Drop can run during GC sweep. The thrown value held in
        // `Failed(v)` is itself a GC value whose backing box may be freed in
        // the *same* sweep, so we must NOT dereference it here (no `{v}`). We
        // only inspect the state discriminant, which is inline in our own
        // (still-valid) allocation.
        if !self.observed.load(std::sync::atomic::Ordering::Relaxed)
            && let Ok(state) = self.state.lock()
            && matches!(&*state, FutureState::Failed(_))
        {
            eprintln!(
                "[clojurust warning] a failed future was discarded without its error \
                 being observed (no await/deref); the thrown exception was lost"
            );
        }
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
            // Both Done and Failed hold a Value (the result or the thrown
            // error); trace either so the GC keeps it alive until observed.
            if let FutureState::Done(v) | FutureState::Failed(v) = &*state {
                v.trace(visitor);
            }
        }
    }
}

// ── Agent ─────────────────────────────────────────────────────────────────────

/// A Clojure agent — asynchronous state update queue (stub: not yet implemented).
pub struct Agent {
    /// Current state.
    pub state: Arc<Mutex<Value>>,
    /// Last error.
    pub error: Arc<Mutex<Option<Value>>>,
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod var_tests {
    use super::*;
    use crate::shared::SharedValue;

    #[test]
    fn bind_promotable_mirrors_shared_root() {
        let var = Var::new("user", "x");
        assert!(var.shared_root.load().is_none());
        var.bind(Value::Long(7));
        assert!(matches!(
            var.shared_root.load().as_ref().as_ref(),
            Some(SharedValue::Long(7))
        ));
        assert_eq!(var.deref(), Some(Value::Long(7)));
        assert_eq!(var.deref_shared(), Some(Value::Long(7)));
    }

    #[test]
    fn bind_nonpromotable_clears_shared_root() {
        let var = Var::new("user", "f");
        var.bind(Value::Long(1));
        assert!(var.shared_root.load().is_some());
        // Rebinding to a non-promotable value clears the mirror, but the
        // isolate-local fast path still holds it.
        let f = Value::NativeFunction(GcPtr::new(NativeFn::new("f", Arity::Fixed(0), |_| {
            Ok(Value::Nil)
        })));
        var.bind(f);
        assert!(var.shared_root.load().is_none());
        assert!(var.is_bound());
        assert_eq!(var.deref_shared(), None);
    }

    #[test]
    fn from_shared_root_seeds_local_slot() {
        let src = Var::new("user", "y");
        src.bind(Value::Long(99));
        let recv = Var::from_shared_root("user", "y", false, src.shared_root.clone());
        assert_eq!(recv.deref(), Some(Value::Long(99)));
        // Same underlying cell.
        src.bind(Value::Long(100));
        assert_eq!(recv.deref_shared(), Some(Value::Long(100)));
    }
}
