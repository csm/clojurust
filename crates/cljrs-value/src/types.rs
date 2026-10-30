//! Stub types for Phase 4/7 that are referenced by the Value enum.

#![allow(unused)]

use crate::parking::Event;
use std::collections::HashMap;
use std::mem;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use cljrs_gc::GcPtr;
use cljrs_gc::mutator::TrackedMutex;
use cljrs_reader::Form;

use crate::TypeHint;
use crate::Value;

// ── Protocol ──────────────────────────────────────────────────────────────────

/// Inner map type for protocol implementations: method_name → impl fn.
pub type MethodMap = HashMap<Arc<str>, Value>;

#[cfg(feature = "no-gc")]
type ImplementationOwnership = HashMap<Arc<str>, HashMap<Arc<str>, crate::publish::ValueLease>>;

/// A Clojure protocol — an interface-like construct with named methods.
#[derive(Debug)]
pub struct Protocol {
    pub name: Arc<str>,
    pub ns: Arc<str>,
    pub methods: Vec<ProtocolMethod>,
    /// type_tag → { method_name → impl fn }
    pub impls: TrackedMutex<HashMap<Arc<str>, MethodMap>>,
    #[cfg(feature = "no-gc")]
    impl_ownership: std::cell::RefCell<ImplementationOwnership>,
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
            impls: TrackedMutex::new(HashMap::new()),
            #[cfg(feature = "no-gc")]
            impl_ownership: Default::default(),
            extend_via_metadata,
        }
    }
}

impl Protocol {
    /// Install an implementation and its replaceable ownership cache.
    pub fn set_implementation(&self, tag: Arc<str>, method: Arc<str>, value: Value) {
        #[cfg(feature = "no-gc")]
        let (value, lease) = crate::publish::prepare_for_owner(
            value,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        #[cfg(not(feature = "no-gc"))]
        let value = crate::publish::publish_value(value);
        self.impls
            .lock()
            .unwrap()
            .entry(tag.clone())
            .or_default()
            .insert(method.clone(), value);
        #[cfg(feature = "no-gc")]
        {
            let previous = self
                .impl_ownership
                .borrow_mut()
                .entry(tag)
                .or_default()
                .insert(method, lease);
            drop(previous);
        }
    }
    /// Borrow a cached implementation without allocating a composite key.
    pub fn protect_implementation(&self, tag: &str, method: &str) {
        #[cfg(feature = "no-gc")]
        if let Some(lease) = self
            .impl_ownership
            .borrow()
            .get(tag)
            .and_then(|methods| methods.get(method))
        {
            lease.protect_read();
        }
        #[cfg(not(feature = "no-gc"))]
        let _ = (tag, method);
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

impl Drop for Protocol {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}

impl cljrs_gc::Trace for Protocol {
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {
        // Mutable contents own their destination leases independently.
    }
    #[cfg(not(feature = "no-gc"))]
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
    pub methods: TrackedMutex<HashMap<String, Value>>,
    /// pr_str(dispatch-val) → the dispatch value itself, for `isa?` lookups
    pub dispatch_vals: TrackedMutex<HashMap<String, Value>>,
    /// recorded preferences: pr_str(preferred) → pr_str(over)
    pub prefers: TrackedMutex<HashMap<String, Vec<String>>>,
    /// pr_str(preference dispatch value) → original value, including values
    /// which do not currently have a method.
    pub preference_vals: TrackedMutex<HashMap<String, Value>>,
    /// normally ":default"
    pub default_dispatch: String,
    method_generation: std::sync::atomic::AtomicU64,
    method_cache: TrackedMutex<MultiFnMethodCache>,
    #[cfg(feature = "no-gc")]
    method_ownership: std::cell::RefCell<HashMap<String, crate::publish::ValueLease>>,
}

#[derive(Debug, Default)]
struct MultiFnMethodCache {
    hierarchy_generation: u64,
    method_generation: u64,
    methods: HashMap<String, String>,
}

impl MultiFn {
    pub fn new(name: Arc<str>, dispatch_fn: Value, default_dispatch: String) -> Self {
        Self {
            name,
            dispatch_fn,
            methods: TrackedMutex::new(HashMap::new()),
            dispatch_vals: TrackedMutex::new(HashMap::new()),
            prefers: TrackedMutex::new(HashMap::new()),
            preference_vals: TrackedMutex::new(HashMap::new()),
            default_dispatch,
            method_generation: std::sync::atomic::AtomicU64::new(0),
            method_cache: TrackedMutex::new(MultiFnMethodCache::default()),
            #[cfg(feature = "no-gc")]
            method_ownership: Default::default(),
        }
    }

    /// Read a method while borrowing its independently owned backing scopes.
    pub fn method(&self, key: &str) -> Option<Value> {
        let value = self.methods.lock().unwrap().get(key).cloned();
        #[cfg(feature = "no-gc")]
        if let Some(lease) = self.method_ownership.borrow().get(key) {
            lease.protect_read();
        }
        value
    }

    /// Replace one method and its cached backing scopes.
    pub fn set_method(&self, key: String, value: Value) {
        #[cfg(feature = "no-gc")]
        let (value, lease) = crate::publish::prepare_for_owner(
            value,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        #[cfg(not(feature = "no-gc"))]
        let value = crate::publish::publish_value(value);
        self.methods.lock().unwrap().insert(key.clone(), value);
        #[cfg(feature = "no-gc")]
        {
            let previous = self.method_ownership.borrow_mut().insert(key, lease);
            drop(previous);
        }
    }
    /// Remove a method, releasing its cache after existing readers finish.
    pub fn remove_method(&self, key: &str) -> bool {
        let removed = self.methods.lock().unwrap().remove(key).is_some();
        #[cfg(feature = "no-gc")]
        {
            let previous = self.method_ownership.borrow_mut().remove(key);
            drop(previous);
        }
        removed
    }

    /// Current generation of this multimethod's method and preference tables.
    pub fn method_generation(&self) -> u64 {
        self.method_generation
            .load(std::sync::atomic::Ordering::Acquire)
    }

    /// Invalidate this multimethod's cached inherited/default resolutions.
    pub fn bump_method_generation(&self) {
        self.method_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }

    /// Return a cached resolved method key when both source generations match.
    /// Observing either new generation lazily discards stale entries at once.
    pub fn cached_method(
        &self,
        dispatch_key: &str,
        hierarchy_generation: u64,
        method_generation: u64,
    ) -> Option<String> {
        let mut cache = self.method_cache.lock().unwrap();
        if cache.hierarchy_generation != hierarchy_generation
            || cache.method_generation != method_generation
        {
            cache.hierarchy_generation = hierarchy_generation;
            cache.method_generation = method_generation;
            cache.methods.clear();
        }
        cache.methods.get(dispatch_key).cloned()
    }

    /// Cache `dispatch_key -> method_key` under the source generations used to
    /// resolve it. A concurrent mutation makes the entry stale, so the next
    /// lookup discards it; the write itself never needs to be suppressed.
    pub fn cache_method(
        &self,
        dispatch_key: String,
        method_key: String,
        hierarchy_generation: u64,
        method_generation: u64,
    ) {
        let mut cache = self.method_cache.lock().unwrap();
        if cache.hierarchy_generation != hierarchy_generation
            || cache.method_generation != method_generation
        {
            cache.hierarchy_generation = hierarchy_generation;
            cache.method_generation = method_generation;
            cache.methods.clear();
        }
        cache.methods.insert(dispatch_key, method_key);
    }

    /// Number of entries in the current cache, exposed for regression tests.
    pub fn cached_method_count(&self) -> usize {
        self.method_cache.lock().unwrap().methods.len()
    }
}

impl Drop for MultiFn {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}

impl cljrs_gc::Trace for MultiFn {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let methods = self.methods.lock().unwrap().clone();
        for (key, value) in methods {
            self.set_method(key, value);
        }
        for (prefix, table) in [
            ("dispatch", &self.dispatch_vals),
            ("prefer", &self.preference_vals),
        ] {
            for (key, value) in table.lock().unwrap().iter() {
                let lease = cljrs_gc::region::RegionLease::for_value(value);
                cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                    self as *const _ as usize,
                    format!("{prefix}:{key}"),
                    &lease,
                );
            }
        }
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.dispatch_fn.trace(visitor);
    }
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.dispatch_fn.trace(visitor);
        {
            let methods = self.methods.lock().unwrap();
            for v in methods.values() {
                v.trace(visitor);
            }
        }
        {
            let dispatch_vals = self.dispatch_vals.lock().unwrap();
            for v in dispatch_vals.values() {
                v.trace(visitor);
            }
        }
        {
            let preference_vals = self.preference_vals.lock().unwrap();
            for v in preference_vals.values() {
                v.trace(visitor);
            }
        }
    }
}

#[cfg(feature = "no-gc")]
fn prepare_optional(
    value: Option<Value>,
    address: usize,
) -> (Option<Value>, crate::publish::ValueLease) {
    match value {
        Some(value) => {
            let (value, lease) = crate::publish::prepare_for_owner(
                value,
                &cljrs_gc::region::AllocationOwner::for_address(address),
            );
            (Some(value), lease)
        }
        None => (None, Default::default()),
    }
}
fn edit_watches(
    watches: &TrackedMutex<Vec<(Value, Value)>>,
    added: Option<(Value, Value)>,
    removed: Option<&Value>,
) {
    let mut watches = watches.lock().unwrap();
    if let Some((key, callback)) = added {
        if let Some(entry) = watches.iter_mut().find(|(k, _)| k == &key) {
            entry.1 = callback;
        } else {
            watches.push((key, callback));
        }
    }
    if let Some(key) = removed {
        watches.retain(|(k, _)| k != key);
    }
}

#[cfg(feature = "no-gc")]
fn refresh_watch_lease(
    watches: &TrackedMutex<Vec<(Value, Value)>>,
    cached: &std::cell::RefCell<crate::publish::ValueLease>,
    address: usize,
) {
    let values: Vec<Value> = watches
        .lock()
        .unwrap()
        .iter()
        .flat_map(|(key, callback)| [key.clone(), callback.clone()])
        .collect();
    let lease = crate::publish::ValueLease::for_values(
        &values,
        &cljrs_gc::region::AllocationOwner::for_address(address),
    );
    *cached.borrow_mut() = lease;
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
    pub value: TrackedMutex<Option<Value>>,
    #[cfg(feature = "no-gc")]
    value_lease: std::cell::RefCell<crate::publish::ValueLease>,
    /// Cross-isolate mirror of the root binding (Phase B3).  `None` when the
    /// var is unbound or its current root is not promotable.
    pub shared_root: Arc<arc_swap::ArcSwap<Option<crate::shared::SharedValue>>>,
    pub is_macro: bool,
    /// Metadata map (e.g. `{:dynamic true}`).
    pub meta: TrackedMutex<Option<Value>>,
    #[cfg(feature = "no-gc")]
    meta_lease: std::cell::RefCell<crate::publish::ValueLease>,
    #[cfg(feature = "no-gc")]
    watch_lease: std::cell::RefCell<crate::publish::ValueLease>,
    pub watches: TrackedMutex<Vec<(Value, Value)>>,
    binding_generation: std::sync::atomic::AtomicU64,
}

impl Var {
    pub fn new(namespace: impl Into<Arc<str>>, name: impl Into<Arc<str>>) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: TrackedMutex::new(None),
            #[cfg(feature = "no-gc")]
            value_lease: Default::default(),
            shared_root: Arc::new(arc_swap::ArcSwap::new(Arc::new(None))),
            is_macro: false,
            meta: TrackedMutex::new(None),
            #[cfg(feature = "no-gc")]
            meta_lease: Default::default(),
            #[cfg(feature = "no-gc")]
            watch_lease: Default::default(),
            watches: TrackedMutex::new(Vec::new()),
            binding_generation: std::sync::atomic::AtomicU64::new(0),
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
        #[cfg(feature = "no-gc")]
        let (local, lease) = match local {
            Some(value) => {
                let (value, lease) = crate::publish::prepare_definition(
                    value,
                    &cljrs_gc::region::AllocationOwner::current(),
                );
                (Some(value), lease)
            }
            None => (None, Default::default()),
        };
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value: TrackedMutex::new(local),
            #[cfg(feature = "no-gc")]
            value_lease: std::cell::RefCell::new(lease),
            shared_root,
            is_macro,
            meta: TrackedMutex::new(None),
            #[cfg(feature = "no-gc")]
            meta_lease: Default::default(),
            #[cfg(feature = "no-gc")]
            watch_lease: Default::default(),
            watches: TrackedMutex::new(Vec::new()),
            binding_generation: std::sync::atomic::AtomicU64::new(0),
        }
    }

    pub fn is_bound(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }

    pub fn snapshot(&self) -> Option<crate::publish::ValueSnapshot> {
        self.value
            .lock()
            .unwrap()
            .clone()
            .map(|value| crate::publish::ValueSnapshot {
                value,
                #[cfg(feature = "no-gc")]
                lease: self.value_lease.borrow().snapshot(),
            })
    }
    pub fn deref(&self) -> Option<Value> {
        let value = self.value.lock().unwrap().clone();
        #[cfg(feature = "no-gc")]
        self.value_lease.borrow().protect_read();
        value
    }

    /// Generation of the isolate-local root binding, incremented after each
    /// completed [`Var::bind`].
    pub fn binding_generation(&self) -> u64 {
        self.binding_generation
            .load(std::sync::atomic::Ordering::Acquire)
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
        #[cfg(not(feature = "no-gc"))]
        let v = crate::publish::publish_value(v);
        #[cfg(feature = "no-gc")]
        let previous_lease = self.value_lease.borrow().snapshot();
        #[cfg(feature = "no-gc")]
        let (v, lease) = crate::publish::prepare_definition(
            v,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        // Replace the binding, holding the lock only across the swap.  The
        // previous value (if any) is handed to the JIT rebind hook so it can
        // reclaim native code compiled for a now-superseded definition
        // (Phase 10.2 — code unloading).  `v.clone()` is O(1) for the only
        // values that carry compiled code (`Value::Fn`, a `GcPtr` clone).
        let prev = {
            let mut slot = self.value.lock().unwrap();
            slot.replace(v.clone())
        };
        #[cfg(feature = "no-gc")]
        {
            *self.value_lease.borrow_mut() = lease;
        }
        // Promote-on-`def` (Phase B3): mirror the new root into the
        // cross-isolate cell when it is promotable, else clear the cell so it
        // never advertises a stale or non-shareable root.  `def` is rare and
        // global by nature, so this write-path cost is acceptable; the read
        // path (and the JIT) never touch the shared cell.
        let shared = crate::shared::promote(&v).ok();
        self.shared_root.store(Arc::new(shared));
        self.binding_generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        if let Some(prev) = prev {
            crate::jit_hooks::notify_var_rebind(&prev, &v);
        }
        #[cfg(feature = "no-gc")]
        {
            drop(previous_lease);
        }
    }

    pub fn add_watch(&self, key: Value, callback: Value) {
        edit_watches(&self.watches, Some((key, callback)), None);
        #[cfg(feature = "no-gc")]
        self.refresh_watch_lease();
    }
    pub fn remove_watch(&self, key: &Value) {
        edit_watches(&self.watches, None, Some(key));
        #[cfg(feature = "no-gc")]
        self.refresh_watch_lease();
    }
    #[cfg(feature = "no-gc")]
    fn refresh_watch_lease(&self) {
        refresh_watch_lease(&self.watches, &self.watch_lease, self as *const _ as usize);
    }
    pub fn protect_watches(&self) {
        #[cfg(feature = "no-gc")]
        self.watch_lease.borrow().protect_read();
    }

    pub fn get_meta(&self) -> Option<Value> {
        #[cfg(feature = "no-gc")]
        self.meta_lease.borrow().protect_read();
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Value) {
        #[cfg(not(feature = "no-gc"))]
        let m = crate::publish::publish_for_owner(m, self as *const _ as usize);
        #[cfg(feature = "no-gc")]
        let (m, lease) = crate::publish::prepare_definition(
            m,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        *self.meta.lock().unwrap() = Some(m);
        #[cfg(feature = "no-gc")]
        {
            *self.meta_lease.borrow_mut() = lease;
        }
    }

    pub fn full_name(&self) -> String {
        format!("{}/{}", self.namespace, self.name)
    }
}

impl cljrs_gc::Trace for Var {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let owner = cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize);
        let lease = self.value_lease.borrow().reparent(&owner);
        let old = self.value_lease.replace(lease);
        drop(old);
    }

    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        #[cfg(not(feature = "no-gc"))]
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
    pub value: TrackedMutex<Value>,
    #[cfg(feature = "no-gc")]
    value_lease: std::cell::RefCell<crate::publish::ValueLease>,
    pub meta: TrackedMutex<Option<Value>>,
    #[cfg(feature = "no-gc")]
    meta_lease: std::cell::RefCell<crate::publish::ValueLease>,
    #[cfg(feature = "no-gc")]
    watch_lease: std::cell::RefCell<crate::publish::ValueLease>,
    pub validator: TrackedMutex<Option<Value>>,
    #[cfg(feature = "no-gc")]
    validator_lease: std::cell::RefCell<crate::publish::ValueLease>,
    pub watches: TrackedMutex<Vec<(Value, Value)>>,
}

impl Atom {
    pub fn new(v: Value) -> Self {
        // Heap-promotion fallback (GC builds): an atom is program-lifetime
        // shared state, so its initial value must not be region-allocated.
        #[cfg(feature = "no-gc")]
        let (v, lease) =
            crate::publish::prepare_definition(v, &cljrs_gc::region::AllocationOwner::current());
        #[cfg(not(feature = "no-gc"))]
        let v = crate::publish::publish_value(v);
        Self {
            value: TrackedMutex::new(v),
            #[cfg(feature = "no-gc")]
            value_lease: std::cell::RefCell::new(lease),
            meta: TrackedMutex::new(None),
            #[cfg(feature = "no-gc")]
            meta_lease: Default::default(),
            #[cfg(feature = "no-gc")]
            watch_lease: Default::default(),
            validator: TrackedMutex::new(None),
            #[cfg(feature = "no-gc")]
            validator_lease: Default::default(),
            watches: TrackedMutex::new(Vec::new()),
        }
    }

    pub fn snapshot(&self) -> crate::publish::ValueSnapshot {
        crate::publish::ValueSnapshot {
            value: self.value.lock().unwrap().clone(),
            #[cfg(feature = "no-gc")]
            lease: self.value_lease.borrow().snapshot(),
        }
    }

    pub fn deref(&self) -> Value {
        let snapshot = self.snapshot();
        snapshot.protect_current();
        snapshot.value
    }

    pub fn reset(&self, v: Value) -> Value {
        #[cfg(not(feature = "no-gc"))]
        let v = crate::publish::publish_value(v);
        #[cfg(feature = "no-gc")]
        let (v, lease) = crate::publish::prepare_for_owner(
            v,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        #[cfg(feature = "no-gc")]
        lease.publish_return();
        *self.value.lock().unwrap() = v.clone();
        #[cfg(feature = "no-gc")]
        {
            *self.value_lease.borrow_mut() = lease;
        }
        v
    }

    /// Replace the value only if it still equals `expected`.
    /// Failed attempts do not publish or lease the candidate.
    pub fn compare_and_set(&self, expected: &Value, value: Value) -> bool {
        // The common vector update compares the exact snapshot just read.
        // Avoid walking its shared elements for each accumulator update.
        let matches_expected = |current: &Value| {
            matches!((current, expected), (Value::Vector(a), Value::Vector(b)) if GcPtr::ptr_eq(a, b))
                || current == expected
        };
        if !matches_expected(&self.value.lock().unwrap()) {
            return false;
        }
        #[cfg(not(feature = "no-gc"))]
        let value = crate::publish::publish_value(value);
        #[cfg(feature = "no-gc")]
        let (value, lease) = crate::publish::prepare_for_owner(
            value,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        let mut guard = self.value.lock().unwrap();
        if !matches_expected(&guard) {
            return false;
        }
        *guard = value;
        drop(guard);
        #[cfg(feature = "no-gc")]
        {
            *self.value_lease.borrow_mut() = lease;
        }
        true
    }

    pub fn add_watch(&self, key: Value, callback: Value) {
        edit_watches(&self.watches, Some((key, callback)), None);
        #[cfg(feature = "no-gc")]
        self.refresh_watch_lease();
    }
    pub fn remove_watch(&self, key: &Value) {
        edit_watches(&self.watches, None, Some(key));
        #[cfg(feature = "no-gc")]
        self.refresh_watch_lease();
    }
    #[cfg(feature = "no-gc")]
    fn refresh_watch_lease(&self) {
        refresh_watch_lease(&self.watches, &self.watch_lease, self as *const _ as usize);
    }
    pub fn protect_watches(&self) {
        #[cfg(feature = "no-gc")]
        self.watch_lease.borrow().protect_read();
    }

    pub fn get_meta(&self) -> Option<Value> {
        #[cfg(feature = "no-gc")]
        self.meta_lease.borrow().protect_read();
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Option<Value>) {
        #[cfg(feature = "no-gc")]
        let (m, lease) = prepare_optional(m, self as *const _ as usize);
        *self.meta.lock().unwrap() = m;
        #[cfg(feature = "no-gc")]
        {
            *self.meta_lease.borrow_mut() = lease;
        }
    }

    pub fn get_validator(&self) -> Option<Value> {
        #[cfg(feature = "no-gc")]
        self.validator_lease.borrow().protect_read();
        self.validator.lock().unwrap().clone()
    }

    pub fn set_validator(&self, vf: Option<Value>) {
        #[cfg(feature = "no-gc")]
        let (vf, lease) = prepare_optional(vf, self as *const _ as usize);
        *self.validator.lock().unwrap() = vf;
        #[cfg(feature = "no-gc")]
        {
            *self.validator_lease.borrow_mut() = lease;
        }
    }
}

impl cljrs_gc::Trace for Atom {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let owner = cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize);
        let lease = self.value_lease.borrow().reparent(&owner);
        let old = self.value_lease.replace(lease);
        drop(old);
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        #[cfg(not(feature = "no-gc"))]
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

/// Which `clojure.core` names a namespace auto-refers, as narrowed by an
/// `(:refer-clojure ...)` clause in `ns`.  An absent filter (the default)
/// refers every public core name.
#[derive(Debug, Clone, Default)]
pub struct ReferClojureFilter {
    /// `:only` — when set, no core name outside this set is referred.
    pub only: Option<std::collections::HashSet<Arc<str>>>,
    /// `:exclude` — core names that are never referred.
    pub exclude: std::collections::HashSet<Arc<str>>,
    /// `:rename` — core name → the local name it is referred under.  A renamed
    /// name is *not* also referred under its original name (matching
    /// `clojure.core/refer`).
    pub rename: HashMap<Arc<str>, Arc<str>>,
}

impl ReferClojureFilter {
    /// The local name `name` is referred under, or `None` when this filter
    /// drops it.
    pub fn local_name(&self, name: &Arc<str>) -> Option<Arc<str>> {
        if self.exclude.contains(name) {
            return None;
        }
        if let Some(only) = &self.only
            && !only.contains(name)
        {
            return None;
        }
        Some(
            self.rename
                .get(name)
                .cloned()
                .unwrap_or_else(|| name.clone()),
        )
    }
}

/// A Clojure namespace with intern table, refers, and aliases.
#[derive(Debug)]
pub struct Namespace {
    pub name: Arc<str>,
    /// Vars interned directly in this namespace.
    pub interns: TrackedMutex<HashMap<Arc<str>, GcPtr<Var>>>,
    /// Vars referred from other namespaces (e.g. clojure.core).
    pub refers: TrackedMutex<HashMap<Arc<str>, GcPtr<Var>>>,
    /// Namespace aliases: short-name → full namespace name.
    pub aliases: TrackedMutex<HashMap<Arc<str>, Arc<str>>>,
    /// Absolute path of the source file this namespace was loaded from,
    /// populated by the loader.  Used by the versioned resolver to locate the
    /// file for `git show`.
    pub source_file: TrackedMutex<Option<Arc<str>>>,
    /// Absolute path of the git repository root that contains `source_file`.
    pub git_repo_root: TrackedMutex<Option<Arc<str>>>,
    /// `true` for namespaces loaded from a specific commit (`name@hash`).
    /// Versioned namespaces are immutable: `intern()` will refuse new bindings.
    pub is_versioned: bool,
    /// Metadata attached via `(ns ^{...} name ...)` or an `ns` attr-map.
    pub meta: TrackedMutex<Option<Value>>,
    /// Narrowing applied to the automatic `clojure.core` refer, set by an
    /// `(:refer-clojure ...)` clause.  `None` refers all of core.
    pub refer_clojure_filter: TrackedMutex<Option<ReferClojureFilter>>,
}

impl Namespace {
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self {
            name: name.into(),
            interns: TrackedMutex::new(HashMap::new()),
            refers: TrackedMutex::new(HashMap::new()),
            aliases: TrackedMutex::new(HashMap::new()),
            source_file: TrackedMutex::new(None),
            git_repo_root: TrackedMutex::new(None),
            is_versioned: false,
            meta: TrackedMutex::new(None),
            refer_clojure_filter: TrackedMutex::new(None),
        }
    }

    /// Create a versioned (immutable) namespace for `name@commit`.
    pub fn new_versioned(name: impl Into<Arc<str>>) -> Self {
        let mut namespace = Self::new(name);
        namespace.is_versioned = true;
        namespace
    }

    /// Record the source file path and its git repo root (if in a repo).
    pub fn set_source_location(&self, file: &str, repo_root: Option<&str>) {
        *self.source_file.lock().unwrap() = Some(Arc::from(file));
        *self.git_repo_root.lock().unwrap() = repo_root.map(Arc::from);
    }

    pub fn get_meta(&self) -> Option<Value> {
        crate::publish::read_slot(self as *const _ as usize, 0);
        self.meta.lock().unwrap().clone()
    }

    pub fn set_meta(&self, m: Value) {
        let m = crate::publish::publish_for_owner(m, self as *const _ as usize);
        *self.meta.lock().unwrap() = Some(m);
    }
}

impl Drop for Namespace {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}

impl cljrs_gc::Trace for Namespace {
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {
        // Mutable contents own their destination leases independently.
    }
    #[cfg(not(feature = "no-gc"))]
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
    /// Managed values retained by the opaque callable closure.
    pub captures: Arc<[Value]>,
    /// Optional adapter that marshals owned inputs and offloads native work.
    pub blocking_func: Option<NativeFnFunc>,
}

impl NativeFn {
    /// Create from a bare function pointer (backwards-compatible).
    pub fn new(name: impl Into<Arc<str>>, arity: Arity, func: NativeFnPtr) -> Self {
        Self {
            name: name.into(),
            arity,
            blocking_func: None,
            captures: Default::default(),
            func: Arc::new(move |args| {
                crate::realization::catch(|| func(args))
                    .unwrap_or_else(|message| Err(crate::ValueError::Other(message)))
            }),
        }
    }

    /// Create a Rust callable with no managed captures. For closures that
    /// retain Values, use with_captures so the collector can visit them.
    pub fn with_closure(
        name: impl Into<Arc<str>>,
        arity: Arity,
        func: impl Fn(&[Value]) -> crate::error::ValueResult<Value> + 'static,
    ) -> Self {
        Self {
            name: name.into(),
            arity,
            blocking_func: None,
            captures: Default::default(),
            func: Arc::new(move |args| {
                crate::realization::catch(|| func(args))
                    .unwrap_or_else(|message| Err(crate::ValueError::Other(message)))
            }),
        }
    }

    /// Create a callable with explicit managed captures for precise tracing.
    /// The callback and tracer share one immutable capture slice.
    /// Callers must not replace the public `captures` handle after construction.
    pub fn with_captures(
        name: impl Into<Arc<str>>,
        arity: Arity,
        captures: Vec<Value>,
        func: impl Fn(&[Value], &[Value]) -> crate::ValueResult<Value> + 'static,
    ) -> Self {
        let captures: Arc<[Value]> = captures.into();
        let shared = captures.clone();
        let mut native = Self::with_closure(name, arity, move |args| func(&shared, args));
        native.captures = captures;
        native
    }

    /// Create a native function with a separate owned-work blocking adapter.
    /// The adapter must keep managed conversions on the caller's isolate.
    pub fn with_blocking(
        name: impl Into<Arc<str>>,
        arity: Arity,
        func: impl Fn(&[Value]) -> crate::ValueResult<Value> + 'static,
        blocking_func: impl Fn(&[Value]) -> crate::ValueResult<Value> + 'static,
    ) -> Self {
        let mut native = Self::with_closure(name, arity, func);
        native.blocking_func = Some(Arc::new(move |args| {
            crate::realization::catch(|| blocking_func(args))
                .unwrap_or_else(|message| Err(crate::ValueError::Other(message)))
        }));
        native
    }

    /// Call the blocking adapter. Reject unsupported functions before execution.
    pub fn call_blocking(&self, args: &[Value]) -> crate::ValueResult<Value> {
        let expected = match self.arity {
            Arity::Fixed(n) if args.len() != n => Some(n.to_string()),
            Arity::Variadic { min } if args.len() < min => Some(format!("{min}+")),
            _ => None,
        };
        if let Some(expected) = expected {
            return Err(crate::ValueError::ArityError {
                name: self.name.to_string(),
                expected,
                got: args.len(),
            });
        }
        let adapter = self.blocking_func.as_ref().ok_or_else(|| {
            crate::ValueError::Other(format!(
                "blocking cannot offload {}: this function has no owned-work adapter",
                self.name
            ))
        })?;
        adapter(args)
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
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        for value in self.captures.iter() {
            value.trace(visitor);
        }
    }
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

/// Identifies a computation by its isolate thread and optional fiber.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Realizer {
    thread: std::thread::ThreadId,
    fiber: Option<u64>,
}
impl Realizer {
    pub fn current() -> Self {
        Self {
            thread: std::thread::current().id(),
            fiber: crate::parking::current(),
        }
    }

    /// Reject a carrier wait that would prevent this owner's fiber resuming.
    /// A different fiber can park; a different thread can block normally.
    pub fn ensure_can_wait(self) -> Result<(), String> {
        if self.thread == std::thread::current().id()
            && self.fiber.is_some()
            && crate::parking::current().is_none()
        {
            Err("cannot block the isolate thread waiting for a parked fiber. Run this code inside a fiber.".into())
        } else {
            Ok(())
        }
    }
}

// Cancellation/panic returns the original thunk to Pending for another reader.
struct RealizationGuard<'a> {
    lazy: Option<&'a LazySeq>,
    delay: Option<&'a Delay>,
}
impl Drop for RealizationGuard<'_> {
    fn drop(&mut self) {
        if let Some(cell) = self.lazy {
            let mut state = cell.state.lock().unwrap();
            if let LazySeqState::Realizing { thunk, .. } = &*state {
                *state = LazySeqState::Pending(thunk.clone());
            }
            drop(state);
            cell.ready.notify_all();
        }
        if let Some(cell) = self.delay {
            let mut state = cell.state.lock().unwrap();
            if let DelayState::Realizing { thunk, .. } = &*state {
                *state = DelayState::Pending(thunk.clone());
            }
            drop(state);
            cell.ready.notify_all();
        }
    }
}

/// Internal state of a lazy sequence cell.
pub enum LazySeqState {
    /// Thunk not yet evaluated.
    Pending(Rc<dyn Thunk>),
    Realizing {
        owner: Realizer,
        thunk: Rc<dyn Thunk>,
    },
    /// Result cached after first force.
    Forced(Value),
    /// Thunk evaluation failed; error message is cached.
    Error(String),
}

/// A lazy sequence that caches its result and retries interrupted realizations.
pub struct LazySeq {
    ready: Event,
    pub state: TrackedMutex<LazySeqState>,
}

impl LazySeq {
    pub fn new(thunk: Box<dyn Thunk>) -> Self {
        Self {
            state: TrackedMutex::new(LazySeqState::Pending(Rc::from(thunk))),
            ready: Event::default(),
        }
    }

    /// Realize the sequence: force the thunk on first call, return cached value on subsequent calls.
    /// Cached thunk errors return nil and remain available through `error()`.
    /// Panics if a carrier wait would prevent the owning fiber resuming.
    /// Use `try_realize` when the caller can return an error.
    pub fn realize(&self) -> Value {
        match self.try_realize() {
            Ok(value) => value,
            Err(error) => {
                // Infallible trait callers cannot return a transient wait
                // error. Fail explicitly instead of silently losing data.
                assert!(self.error().is_some(), "{error}");
                Value::Nil
            }
        }
    }

    /// Realize a sequence with explicit errors, including unsafe carrier waits.
    /// A rejected wait does not change the thunk or its realization state.
    pub fn try_realize(&self) -> Result<Value, String> {
        let owner = Realizer::current();
        let thunk = loop {
            let listener = self.ready.listen();
            let mut state = self.state.lock().unwrap();
            match &*state {
                LazySeqState::Forced(v) => {
                    let value = v.clone();
                    drop(state);
                    crate::publish::read_slot(self as *const _ as usize, 0);
                    return Ok(value);
                }
                LazySeqState::Error(error) => return Err(error.clone()),
                LazySeqState::Realizing { owner: active, .. } => {
                    if *active == owner {
                        return Ok(Value::Nil);
                    }
                    active.ensure_can_wait()?;
                    drop(state);
                    listener.wait(None);
                    continue;
                }
                LazySeqState::Pending(_) => {}
            }
            let LazySeqState::Pending(thunk) =
                mem::replace(&mut *state, LazySeqState::Forced(Value::Nil))
            else {
                unreachable!()
            };
            *state = LazySeqState::Realizing {
                owner,
                thunk: thunk.clone(),
            };
            break thunk;
        };
        let mut guard = RealizationGuard {
            lazy: Some(self),
            delay: None,
        };
        let result = match thunk.force() {
            Ok(result) => {
                let result = crate::publish::publish_for_owner(result, self as *const _ as usize);
                *self.state.lock().unwrap() = LazySeqState::Forced(result.clone());
                Ok(result)
            }
            Err(msg) => {
                *self.state.lock().unwrap() = LazySeqState::Error(msg.clone());
                Err(msg)
            }
        };
        drop(thunk);
        #[cfg(feature = "no-gc")]
        cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
            self as *const _ as usize,
            usize::MAX,
            &Default::default(),
        );
        guard.lazy = None;
        if result.is_ok() {
            crate::publish::read_slot(self as *const _ as usize, 0);
        }
        self.ready.notify_all();
        result
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

impl Drop for LazySeq {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}
impl cljrs_gc::Trace for LazySeq {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let state = self.state.lock().unwrap();
        if let LazySeqState::Pending(thunk) = &*state {
            struct ThunkRef<'a>(&'a dyn Thunk);
            impl cljrs_gc::Trace for ThunkRef<'_> {
                fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
                    self.0.trace(visitor);
                }
            }
            let lease = cljrs_gc::region::RegionLease::for_value(&ThunkRef(thunk.as_ref()));
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                usize::MAX,
                &lease,
            );
        }
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        // Safe to lock unconditionally: realize() drops the lock before entering
        // eval (thunk.force()), so the lock is never held across a GC safepoint.
        {
            let state = self.state.lock().unwrap();
            match &*state {
                LazySeqState::Pending(thunk) => thunk.trace(visitor),
                LazySeqState::Realizing { thunk, .. } => thunk.trace(visitor),
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
    pub value: TrackedMutex<Value>,
    #[cfg(feature = "no-gc")]
    value_lease: std::cell::RefCell<crate::publish::ValueLease>,
}

impl Volatile {
    pub fn new(v: Value) -> Self {
        // GC builds: heap-promotion fallback (see `Var::bind`).
        #[cfg(feature = "no-gc")]
        let (v, lease) =
            crate::publish::prepare_for_owner(v, &cljrs_gc::region::AllocationOwner::current());
        #[cfg(not(feature = "no-gc"))]
        let v = crate::publish::publish_value(v);
        Self {
            value: TrackedMutex::new(v),
            #[cfg(feature = "no-gc")]
            value_lease: std::cell::RefCell::new(lease),
        }
    }

    pub fn snapshot(&self) -> crate::publish::ValueSnapshot {
        crate::publish::ValueSnapshot {
            value: self.value.lock().unwrap().clone(),
            #[cfg(feature = "no-gc")]
            lease: self.value_lease.borrow().snapshot(),
        }
    }

    pub fn deref(&self) -> Value {
        let snapshot = self.snapshot();
        snapshot.protect_current();
        snapshot.value
    }

    pub fn reset(&self, v: Value) -> Value {
        #[cfg(not(feature = "no-gc"))]
        let v = crate::publish::publish_value(v);
        #[cfg(feature = "no-gc")]
        let (v, lease) = crate::publish::prepare_for_owner(
            v,
            &cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize),
        );
        #[cfg(feature = "no-gc")]
        lease.publish_return();
        *self.value.lock().unwrap() = v.clone();
        #[cfg(feature = "no-gc")]
        {
            *self.value_lease.borrow_mut() = lease;
        }
        v
    }
}

impl std::fmt::Debug for Volatile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Volatile")
    }
}

impl cljrs_gc::Trace for Volatile {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let owner = cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize);
        let lease = self.value_lease.borrow().reparent(&owner);
        let old = self.value_lease.replace(lease);
        drop(old);
    }

    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        #[cfg(not(feature = "no-gc"))]
        {
            let value = self.value.lock().unwrap();
            value.trace(visitor);
        }
    }
}

// ── Delay ─────────────────────────────────────────────────────────────────────

/// Internal state of a delay cell.
pub enum DelayState {
    Error(String),
    Pending(Rc<dyn Thunk>),
    Realizing {
        owner: Realizer,
        thunk: Rc<dyn Thunk>,
    },
    Forced(Value),
}

/// A lazy computation with a cached result and retry after interruption.
pub struct Delay {
    ready: Event,
    pub state: TrackedMutex<DelayState>,
}

impl Delay {
    pub fn new(thunk: Box<dyn Thunk>) -> Self {
        Self {
            state: TrackedMutex::new(DelayState::Pending(Rc::from(thunk))),
            ready: Event::default(),
        }
    }

    /// Force the delay and cache the result.
    /// Returns the value on success, or an error message on failure.
    pub fn force(&self) -> Result<Value, String> {
        let owner = Realizer::current();
        let thunk = loop {
            let listener = self.ready.listen();
            let mut state = self.state.lock().unwrap();
            match &*state {
                DelayState::Forced(v) => {
                    let value = v.clone();
                    drop(state);
                    crate::publish::read_slot(self as *const _ as usize, 0);
                    return Ok(value);
                }
                DelayState::Error(e) => return Err(e.clone()),
                DelayState::Realizing { owner: active, .. } => {
                    if *active == owner {
                        return Err("recursive delay realization".into());
                    }
                    active.ensure_can_wait()?;
                    drop(state);
                    listener.wait(None);
                    continue;
                }
                DelayState::Pending(_) => {}
            }
            let DelayState::Pending(thunk) =
                mem::replace(&mut *state, DelayState::Forced(Value::Nil))
            else {
                unreachable!()
            };
            *state = DelayState::Realizing {
                owner,
                thunk: thunk.clone(),
            };
            break thunk;
        };
        let mut guard = RealizationGuard {
            lazy: None,
            delay: Some(self),
        };
        let result = thunk
            .force()
            .map(|value| crate::publish::publish_for_owner(value, self as *const _ as usize));
        *self.state.lock().unwrap() = match &result {
            Ok(value) => DelayState::Forced(value.clone()),
            Err(error) => DelayState::Error(error.clone()),
        };
        drop(thunk);
        #[cfg(feature = "no-gc")]
        cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
            self as *const _ as usize,
            usize::MAX,
            &Default::default(),
        );
        guard.delay = None;
        if result.is_ok() {
            crate::publish::read_slot(self as *const _ as usize, 0);
        }
        self.ready.notify_all();
        result
    }

    /// True if the delay has already been forced.
    pub fn is_realized(&self) -> bool {
        matches!(
            &*self.state.lock().unwrap(),
            DelayState::Forced(_) | DelayState::Error(_)
        )
    }
}

impl std::fmt::Debug for Delay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Delay")
    }
}

impl Drop for Delay {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
    }
}
impl cljrs_gc::Trace for Delay {
    #[cfg(feature = "no-gc")]
    fn init_ownership(&self) {
        let state = self.state.lock().unwrap();
        if let DelayState::Pending(thunk) = &*state {
            struct ThunkRef<'a>(&'a dyn Thunk);
            impl cljrs_gc::Trace for ThunkRef<'_> {
                fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
                    self.0.trace(visitor);
                }
            }
            let lease = cljrs_gc::region::RegionLease::for_value(&ThunkRef(thunk.as_ref()));
            cljrs_gc::region::AllocationOwner::for_address(self as *const _ as usize).set_slot(
                self as *const _ as usize,
                usize::MAX,
                &lease,
            );
        }
    }
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        // Safe to lock unconditionally: force() drops the lock before entering
        // eval (thunk.force()), so the lock is never held across a GC safepoint.
        {
            let state = self.state.lock().unwrap();
            match &*state {
                DelayState::Pending(thunk) => thunk.trace(visitor),
                DelayState::Realizing { thunk, .. } => thunk.trace(visitor),
                DelayState::Error(_) => {}
                DelayState::Forced(v) => v.trace(visitor),
            }
        }
    }
}

// ── CljxPromise ───────────────────────────────────────────────────────────────

/// A one-shot rendezvous (promise).
pub struct CljxPromise {
    pub ready: Event,
    pub value: Mutex<Option<Value>>,
    pub cond: Condvar,
}

impl CljxPromise {
    pub fn new() -> Self {
        Self {
            value: Mutex::new(None),
            cond: Condvar::new(),
            ready: Event::default(),
        }
    }

    /// Deliver a value (no-op if already delivered).
    pub fn deliver(&self, v: Value) {
        if self.value.lock().unwrap().is_some() {
            return;
        }
        // GC builds: heap-promotion fallback — the promise may outlive (and be
        // read from outside) any region scope active at delivery time.
        let v = crate::publish::publish_for_owner(v, self as *const _ as usize);
        let mut guard = self.value.lock().unwrap();
        if guard.is_none() {
            *guard = Some(v);
            self.cond.notify_all();
            drop(guard);
            self.ready.notify_all();
        }
    }

    /// Block until a value is available, then return it.
    pub fn deref_blocking(&self) -> Value {
        self.wait_value(None).unwrap()
    }

    pub fn wait_value(&self, timeout: Option<Duration>) -> Option<Value> {
        let deadline = timeout.map(|d| Instant::now() + d);
        loop {
            let listener = self.ready.listen();
            let value = self.value.lock().unwrap().clone();
            if let Some(value) = value {
                crate::publish::read_slot(self as *const _ as usize, 0);
                return Some(value);
            }
            if !listener.wait(deadline.map(|d| d.saturating_duration_since(Instant::now()))) {
                return self.value.lock().unwrap().clone();
            }
        }
    }

    /// True if already delivered.
    pub fn is_realized(&self) -> bool {
        self.value.lock().unwrap().is_some()
    }
}

impl Drop for CljxPromise {
    fn drop(&mut self) {
        crate::publish::clear_slots(self as *const _ as usize);
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
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
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
    /// The future exhausted the evaluation budget. Kept distinct from a
    /// thrown value so user-level catch clauses cannot intercept it.
    GasExhausted,
    /// The future's body failed with a managed-memory limit fault (strict
    /// governor mode). Kept distinct for the same reason as `GasExhausted`.
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
    Cancelled,
}

/// The message carried by the error raised when reading a cancelled future.
pub const FUTURE_CANCELLED_MSG: &str = "future was cancelled";

/// A future value computed asynchronously on another thread.
pub struct CljxFuture {
    pub ready: Event,
    pub state: Mutex<FutureState>,
    pub cond: Condvar,
    executor_thread: Option<std::thread::ThreadId>,
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
            ready: Event::default(),
            executor_thread: None,
            observed: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// A future whose producer needs this thread's local executor.
    pub fn new_local() -> Self {
        let mut future = Self::new();
        future.executor_thread = Some(std::thread::current().id());
        future
    }

    /// Wait for settlement, parking only when running on a fiber.
    pub fn wait_value(&self, timeout: Option<Duration>) -> crate::ValueResult<Option<Value>> {
        let deadline = timeout.map(|d| Instant::now() + d);
        loop {
            let listener = self.ready.listen();
            if let Some(result) = self.settled_value() {
                return result.map(Some);
            }
            if deadline.is_some_and(|deadline| deadline <= Instant::now()) {
                return self.settled_value().transpose();
            }
            if crate::parking::current().is_none()
                && self.executor_thread == Some(std::thread::current().id())
            {
                return Err(crate::ValueError::Other(
                    "cannot wait for an isolate future outside a fiber; use await in async code or fiber::run_root for synchronous evaluation".into(),
                ));
            }
            if !listener.wait(deadline.map(|d| d.saturating_duration_since(Instant::now()))) {
                return self.settled_value().transpose();
            }
        }
    }

    pub fn settled_value(&self) -> Option<crate::ValueResult<Value>> {
        use crate::ValueError;
        let state = self.state.lock().unwrap();
        let value = match &*state {
            FutureState::Running => return None,
            FutureState::Done(v) => Ok(v.clone()),
            FutureState::Failed(v) => Err(ValueError::Thrown(v.clone())),
            FutureState::GasExhausted => Err(ValueError::GasExhausted),
            FutureState::MemoryLimitExceeded(f) => Err(ValueError::MemoryLimitExceeded(*f)),
            FutureState::Cancelled => {
                drop(state);
                return Some(Err(ValueError::Thrown(Self::cancelled_error())));
            }
        };
        self.mark_observed();
        drop(state);
        #[cfg(feature = "no-gc")]
        match &value {
            Ok(value) | Err(crate::ValueError::Thrown(value)) => {
                cljrs_gc::region::RegionLease::for_value(value).protect_read()
            }
            _ => {}
        }
        Some(value)
    }

    /// True if done, failed, or cancelled (not still running).
    pub fn is_done(&self) -> bool {
        !matches!(&*self.state.lock().unwrap(), FutureState::Running)
    }

    /// True if explicitly cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(&*self.state.lock().unwrap(), FutureState::Cancelled)
    }

    /// Mark a still-running future cancelled, waking anyone waiting on it.
    ///
    /// Returns false when it had already settled. The task itself is not
    /// interrupted; what is cancelled is the result.
    pub fn cancel(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        if matches!(&*state, FutureState::Running) {
            *state = FutureState::Cancelled;
            self.cond.notify_all();
            drop(state);
            self.ready.notify_all();
            true
        } else {
            false
        }
    }

    /// The catchable value every reader of a cancelled future raises.
    ///
    /// One shape for every path — the sync tree-walker's `await`/`deref`, the
    /// async tree-walker, and the compiled state machine all hand user code
    /// the same `ex-info`-like error, so a `catch` behaves the same whether or
    /// not the enclosing fn happened to be async-JIT-compiled.
    pub fn cancelled_error() -> Value {
        Value::Error(GcPtr::new(crate::ExceptionInfo::new(
            crate::ValueError::Other(FUTURE_CANCELLED_MSG.to_string()),
            FUTURE_CANCELLED_MSG.to_string(),
            None,
            None,
        )))
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
            && matches!(
                &*state,
                FutureState::Failed(_)
                    | FutureState::GasExhausted
                    | FutureState::MemoryLimitExceeded(_)
            )
        {
            eprintln!(
                "[clojurust warning] a failed future was discarded without its error \
                 being observed (no await/deref); the failure was lost"
            );
        }
        crate::publish::clear_slots(self as *const _ as usize);
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
    #[cfg(feature = "no-gc")]
    fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    #[cfg(not(feature = "no-gc"))]
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
    pub watches: TrackedMutex<Vec<(Value, Value)>>,
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
    fn native_captures_share_one_immutable_allocation() {
        let native = NativeFn::with_captures(
            "capture",
            Arity::Fixed(0),
            vec![Value::Long(7)],
            |captures, _| Ok(captures[0].clone()),
        );
        assert_eq!(Arc::strong_count(&native.captures), 2);
        assert_eq!((native.func)(&[]).unwrap(), Value::Long(7));
    }

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
