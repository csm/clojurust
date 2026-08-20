# cljrs-value

Core runtime values and persistent collections for clojurust.

**Phase:** 3 (collections/Value) + 4 (CljxFn, Namespace) + 5 (LazySeq, CljxCons) + 6 (Protocol, ProtocolFn, MultiFn) + 7 (Volatile, Delay, CljxPromise, CljxFuture, Agent) + 6-ext (TypeInstance for defrecord/reify) + B2 (structured-clone boundary) + B3 (shared static arena: intern tables, SharedValue, SharedAtom, ByteBlob) — implemented.

---

## Purpose

Defines `Value`, the single enum that represents every Clojure runtime value,
plus all persistent (immutable, structurally shared) collection types.  The
`cljrs-runtime` crate's `interp` and `tiered` modules operate on `Value`s; its
`builtins` module and `cljrs-stdlib` build the standard library on top of them.

---

## File layout

```
src/
  lib.rs                         — module declarations and re-exports
  clone.rs                       — SerializedValue (Send+Sync wire form), CloneError, serialize/deserialize for cross-isolate copy boundary (Phase B2); SerializedValue::byte_size for boundary metering; handles SharedAtom/ByteBlob/Var pass-through (B3 — Var shares its root cell, issue #171)
  error.rs                       — ValueError enum, ValueResult<T> alias
  hash.rs                        — ClojureHash trait, Murmur3 helpers, JVM-compatible hash_string
  intern.rs                      — (Phase B3) global keyword/symbol intern tables backed by StaticGcPtr; intern_keyword, intern_symbol
  jit_hooks.rs                   — (Phases 10.2/10.5) var-rebind hooks fired by Var::bind; set_var_rebind_hook registers multiple consumers (the compiler JIT stales superseded native code; cljrs-runtime invalidates cross-defn-specialized lowerings)
  keyword.rs                     — Keyword { namespace, name }
  publish.rs                     — (Phase 10.5, GC builds; identity stub under no-gc) heap-promotion publish barrier: publish_value(Value) -> Value scans for region-allocated boxes, deep-copies them to the GC heap (via clone.rs), or poisons the active regions when the value is opaque to the scan. Called by Var::bind, Atom::new/reset, Volatile::new/reset, CljxPromise::deliver, and cljrs-async channel puts
  shared.rs                      — (Phase B3) SharedValue enum, SharedAtom (Arc<ArcSwap<SharedValue>>), promote/demote; PromoteError. Var roots reuse SharedValue via Var::shared_root (issue #171)
  regex.rs                       — Pattern (engine-agnostic compiled regex behind Value::Pattern, selected by the regex-full / small-regex features), PatternError, Captures, Matcher (stateful re-find/re-matches driver), MatchPhase, MatchResult
  symbol.rs                      — Symbol { namespace, name }
  type_hint.rs                   — TypeHint enum (^long/^double/^longs/… primitive type tags) + from_tag/is_array/element
  native_object.rs               — NativeObject trait, NativeObjectBox wrapper, gc_native_object helper (Phase 9 interop)
  types.rs                       — Var, Atom, Namespace, NativeFn, CljxFn, Thunk, LazySeq, CljxCons, Protocol, ProtocolFn, ProtocolMethod, MultiFn, Volatile, Delay, CljxPromise, CljxFuture, Agent
  value.rs                       — Value enum (incl. SharedAtom, ByteBlob variants), MapValue, SetValue, TypeInstance, pr_str, PartialEq, ClojureHash, std::hash::Hash
  collections/
    mod.rs                       — re-exports all collection types
    array_map.rs                 — PersistentArrayMap (≤8 entries, linear scan)
    hash_map.rs                  — PersistentHashMap (32-way HAMT keyed by insertion-sequence index + a red-black tree ordering log, so iteration order matches insertion order deterministically)
    hash_set.rs                  — PersistentHashSet (same index+ordering-log technique as PersistentHashMap)
    list.rs                      — PersistentList (singly-linked cons list)
    queue.rs                     — PersistentQueue (front-list + rear-vector)
    vector.rs                    — PersistentVector (32-way trie + tail buffer)
    hamt/
      mod.rs                     — re-exports Node and bitmap helpers
      bitmap.rs                  — BITS, WIDTH, fragment, sparse_index, bit_for
      node.rs                    — Node<V> enum (Leaf, Branch, Collision); HAMT trie operations
```

---

## Public API

### `Value`

```rust
pub enum Value {
    // Scalars
    Nil,
    Bool(bool),
    Long(i64),
    Double(f64),
    BigInt(GcPtr<num_bigint::BigInt>),
    BigDecimal(GcPtr<bigdecimal::BigDecimal>),
    Ratio(GcPtr<num_rational::Ratio<num_bigint::BigInt>>),
    Char(char),
    Str(GcPtr<String>),
    // Regexes (engine selected by the regex-full / small-regex features)
    Pattern(GcPtr<Pattern>),
    Matcher(GcPtr<Matcher>),
    // Identifiers
    Symbol(GcPtr<Symbol>),
    Keyword(GcPtr<Keyword>),
    // Collections
    List(GcPtr<PersistentList>),
    Vector(GcPtr<PersistentVector>),
    Map(MapValue),
    Set(GcPtr<PersistentHashSet>),
    Queue(GcPtr<PersistentQueue>),
    // Lazy sequences (Phase 5)
    LazySeq(GcPtr<LazySeq>),   // deferred sequence; forced at most once
    Cons(GcPtr<CljxCons>),     // cons cell with lazy-capable tail
    // Runtime objects
    Var(GcPtr<Var>),
    Atom(GcPtr<Atom>),
    SharedAtom(Arc<SharedAtom>),       // cross-isolate mutable ref (Phase B3)
    ByteBlob(Arc<[u8]>),               // refcounted immutable byte buffer (Phase B3)
    Namespace(GcPtr<Namespace>),
    NativeFn(GcPtr<NativeFn>),
    CljxFn(GcPtr<CljxFn>),
    // Protocols & Multimethods (Phase 6)
    Protocol(GcPtr<Protocol>),
    ProtocolFn(GcPtr<ProtocolFn>),
    MultiFn(GcPtr<MultiFn>),
    // Concurrency primitives (Phase 7)
    Volatile(GcPtr<Volatile>),
    Delay(GcPtr<Delay>),
    Promise(GcPtr<CljxPromise>),
    Future(GcPtr<CljxFuture>),
    Agent(GcPtr<Agent>),

    // Records / reify (Phase 6-ext)
    TypeInstance(GcPtr<TypeInstance>),
}

pub enum MapValue {
    Array(GcPtr<PersistentArrayMap>),
    Hash(GcPtr<PersistentHashMap>),
}
```

`PartialEq` implements cross-type numeric equality (`(= 1 1N)`, `(= 1 1.0)`)
and sequential collection equality between `List` and `Vector`.

`Display` / `pr_str` produce Clojure-readable output.

### `Symbol` / `Keyword`

```rust
pub struct Symbol   { namespace: Option<Arc<str>>, name: Arc<str>, version: Option<Arc<str>> }
pub struct Keyword  { namespace: Option<Arc<str>>, name: Arc<str> }
```

Both support `simple(name)`, `qualified(ns, name)`, `parse(str)`, and
`full_name() -> String`. `Symbol` additionally carries an optional git-commit
`version` (the `@<hash>` suffix) with `versioned_name() -> String`. Free
helpers used by all execution tiers to detect versioned names:
`symbol::is_commit_hash(s) -> bool` and
`symbol::split_version(name) -> (&str, Option<&str>)`.

### `regex` — `Pattern` / `Captures` / `Matcher`

`Value::Pattern` holds a `GcPtr<Pattern>`, never an engine type directly.
`Pattern` is the only place in the workspace that names a regex engine, so the
engine is a build-time choice (see [Features](#features)):

```rust
pub struct Pattern(/* regex::Regex or regex_lite::Regex */);

impl Pattern {
    pub fn new(pattern: &str) -> Result<Pattern, PatternError>;
    pub fn as_str(&self) -> &str;
    pub fn captures<'h>(&self, haystack: &'h str) -> Option<Captures<'h>>;
    /// Whole-haystack match (Java `Matcher.matches()`), via a `\A(?:…)\z` twin
    /// compiled once per Pattern — neither engine has an anchored search, and
    /// span-filtering an unanchored one loses to leftmost-first (`a|ab`).
    pub fn captures_full<'h>(&self, haystack: &'h str) -> Option<Captures<'h>>;
    pub fn captures_at<'h>(&self, haystack: &'h str, start: usize) -> Option<Captures<'h>>;
    pub fn replace<'h>(&self, haystack: &'h str, replacement: &str) -> Cow<'h, str>;
    pub fn replace_all<'h>(&self, haystack: &'h str, replacement: &str) -> Cow<'h, str>;
    pub fn split<'h>(&self, haystack: &'h str) -> impl Iterator<Item = &'h str>;
    pub fn splitn<'h>(&self, haystack: &'h str, limit: usize) -> impl Iterator<Item = &'h str>;
}

impl Clone for Pattern {}
impl fmt::Display for Pattern {}   // the pattern source
impl Trace for Pattern {}          // leaf: no GcPtr fields

/// Engine-independent compile failure; carries the engine's message.
pub struct PatternError(/* private */);
impl fmt::Display for PatternError {}
impl std::error::Error for PatternError {}

/// A successful match, borrowing the haystack.
pub struct Captures<'h>(/* private */);

impl<'h> Captures<'h> {
    pub fn full(&self) -> &'h str;        // group 0
    pub fn start(&self) -> usize;         // byte offset where group 0 begins
    pub fn end(&self) -> usize;           // byte offset past group 0
    pub fn group_count(&self) -> usize;   // groups incl. group 0; ≥ 1
    pub fn groups(&self) -> impl Iterator<Item = Option<&'h str>> + '_;
}
```

`Matcher` is the stateful driver behind `re-find`/`re-matches`/`re-seq`: it
holds `pattern: GcPtr<Pattern>` plus a haystack and walks matches left to
right, one per `next()` call. With `match_all` set (`re-matches`) the search is
anchored to the whole haystack — Java's `Matcher.matches()` semantics — and
yields at most one match: the next `next()` is `Complete` either way.

```rust
/// Matching's payload is the byte offset the next search resumes from: the end
/// of the match, bumped one character past a zero-width one (Java `find`), so
/// `#"a*"` terminates instead of re-finding the empty match forever.
pub enum MatchPhase { New, Matching(usize), Complete }

pub struct Matcher { pub pattern: GcPtr<Pattern>, /* haystack, state, match_all */ }

impl Matcher {
    pub fn new(pattern: Pattern, source: String, match_all: bool) -> Matcher;
    /// As `new`, sharing an allocated pattern so its anchored form is compiled once.
    pub fn from_ptr(pattern: GcPtr<Pattern>, source: String, match_all: bool) -> Matcher;
    pub fn next(&self) -> MatchPhase;            // advance; Complete once exhausted
    pub fn capture(&self) -> Option<MatchResult>; // last match, owned
    pub fn phase(&self) -> MatchPhase;
}

impl Clone for Matcher {}
impl Trace for Matcher {}

/// Owned form of a match — outlives the haystack borrow.
pub struct MatchResult { pub full: String, pub groups: Vec<Option<String>> }

impl MatchResult {
    pub fn new(cap: &Captures<'_>) -> MatchResult;
    /// Str for a group-less match, Vector of groups otherwise (Clojure's
    /// re-find return shape).
    pub fn to_value(&self) -> Value;
}
```

### Phase B3 — Shared static arena

#### Intern tables (`intern` module)

```rust
pub fn intern_keyword(namespace: Option<&str>, name: &str)
    -> StaticGcPtr<Keyword>;
pub fn intern_symbol(namespace: Option<&str>, name: &str, version: Option<&str>)
    -> StaticGcPtr<Symbol>;
```

Global `OnceLock<Mutex<HashMap<…>>>` tables.  First call allocates the
`Keyword`/`Symbol` into program-lifetime memory via `static_alloc`; subsequent
calls return a clone of the same `StaticGcPtr` (pointer-stable identity across
all isolates).

#### `SharedValue` and `SharedAtom` (`shared` module)

```rust
pub enum SharedValue {
    Nil, Bool(bool), Long(i64), Double(f64), Char(char), Uuid(u128),
    Str(Arc<str>),
    Keyword(StaticGcPtr<Keyword>),
    Symbol(StaticGcPtr<Symbol>),
    ByteBlob(Arc<[u8]>),              // BEAM off-heap-binary trick
}

pub struct SharedAtom {
    pub cell: Arc<ArcSwap<SharedValue>>,
    pub meta: Mutex<Option<SharedValue>>,
}

impl SharedAtom {
    pub fn new(val: SharedValue) -> Self
    pub fn deref_val(&self) -> Arc<SharedValue>     // atomic load
    pub fn reset(&self, val: SharedValue) -> Arc<SharedValue>
    pub fn swap<F>(&self, f: F) -> Arc<SharedValue> // CAS-retry (closure form)
    pub fn compare_and_set(&self, current: &Arc<SharedValue>, new: SharedValue) -> bool
}

pub fn promote(value: &Value)   -> Result<SharedValue, PromoteError>;
pub fn demote (sv:    &SharedValue) -> Value;
```

`promote` converts an isolate-local `Value` to `SharedValue` (fails for
closures, resources, atoms, …).  `demote` converts back into a fresh
isolate-local `Value`.  `compare_and_set` is the single lock-free CAS that
backs the Clojure-level `compare-and-set!` and the `swap!` retry loop (callers
that must run interpreter code between load and store use it instead of the
closure-based `swap`).

#### Var roots — two-tier, promote-on-`def` (issue #171)

A var's *root* binding uses the **same** cross-isolate mechanism as
`shared-atom`.  `Var` carries two slots:

```rust
pub struct Var {
    pub value: Mutex<Option<Value>>,                          // isolate-local fast path
    pub shared_root: Arc<ArcSwap<Option<SharedValue>>>,       // cross-isolate mirror (B3)
    // …namespace, name, is_macro, meta, watches
}

impl Var {
    pub fn deref(&self) -> Option<Value>          // reads the local fast path
    pub fn deref_shared(&self) -> Option<Value>   // demotes the shared cell
    pub fn bind(&self, v: Value)                  // promote-on-def: updates both slots
    pub fn from_shared_root(ns, name, is_macro, shared_root) -> Self  // receiver side
}
```

- **Reads stay local.** Every var deref, the IR tier, and the JIT/AOT `rt_*`
  ABI read `value` — promotion never touches it, so inline caches and
  pointer-identity assumptions in compiled code remain valid (no JIT
  regression).
- **`bind` promotes-on-write.** `def` / `alter-var-root` / `set!` all funnel
  through `Var::bind`, which mirrors the new root into `shared_root` when it is
  promotable, and clears it to `None` otherwise.  `def` is rare, so this
  write-path cost is acceptable.
- **Crossing isolates.** `clone::serialize` passes the `shared_root` `Arc`
  through (both isolates share the same cell); the receiver rebuilds the var
  with `from_shared_root`, seeding its local slot from the demoted snapshot.
  A var bound to a **non-promotable** root (closure / native resource) is
  *explicitly isolate-local* (ADR option (b)): `serialize` returns
  `CloneError::NotShareable { type_name: "var" }` — a non-silent boundary
  error.  Var-root watches stay isolate-local (the shared cell carries no
  watch callbacks), matching `shared-atom`.

### `ClojureHash`

```rust
pub trait ClojureHash { fn clojure_hash(&self) -> u32; }
```

Implemented for `Value` using Murmur3 + JVM `String.hashCode` semantics.
Whole-number doubles hash like their `Long` equivalent.

### Collections

| Type | Description | Key operations |
|---|---|---|
| `PersistentList` | Singly-linked cons list | `cons`, `first`, `rest`, `count` (O(1)) |
| `PersistentVector` | 32-way trie + tail buffer | `conj`, `nth`, `assoc_nth`, `pop`, `iter`, `map_entry`, `is_map_entry` |
| `PersistentArrayMap` | Flat key/value vec, ≤8 entries | `assoc` (returns `AssocResult`), `get`, `dissoc`, `iter` |
| `PersistentHashMap` | 32-way HAMT index (key→seq) + red-black ordering log (seq→(key,val)); iterates in insertion order | `assoc`, `get`, `dissoc`, `merge`, `iter`, `keys`, `vals` |
| `PersistentHashSet` | Same index+ordering-log technique; iterates in insertion order | `conj`, `disj`, `contains`, `iter` |
| `PersistentQueue` | Front-list + rear-vector | `enqueue`, `dequeue`, `peek` |

`PersistentArrayMap::assoc` returns `AssocResult::Array(Self)` while under the
threshold, or `AssocResult::Promote(Vec<(Value, Value)>)` when the map is full.
`MapValue::assoc` handles the transparent promotion to `PersistentHashMap`.

All collections implement `PartialEq`, `Debug`, `Clone`, and `cljrs_gc::Trace`.
`PersistentList`, `PersistentVector`, and `PersistentHashSet` implement
`std::iter::FromIterator<Value>`.

A `PersistentVector` may be tagged as a **map entry** — the `[key val]` pairs
produced by seq'ing a map, `find`, or the `map-entry` builtin.
`PersistentVector::map_entry(key, val)` builds one; `is_map_entry()` reads the
tag (there is also a `Value::map_entry(k, v)` / `Value::is_map_entry()`
convenience pair). The tag is invisible to equality, hashing, and printing —
`(= (first {:a 1}) [:a 1])` still holds — and it exists only so `map-entry?`
can distinguish real entries from plain 2-element vectors. As in Clojure, any
derived vector (`conj`, `assoc_nth`, `pop`, `from_iter`, ...) is a plain
vector again.

All collection Trace impls also override `gc_size_extra` to report the heap
bytes owned by each collection beyond the GcBox struct.  Approximations used:

| Type | Formula |
|------|---------|
| `PersistentArrayMap` | `16 + capacity × size_of::<Value>()` |
| `PersistentHashMap` | `n × (88 + 2×size_of::<Value>())` |
| `PersistentHashSet` | `n × (88 + size_of::<Value>())` |
| `PersistentVector` | `n × (24 + size_of::<Value>())` |
| `SortedMap` | `n × (40 + 2×size_of::<Value>())` |
| `TransientMap/Set` | same as HashMap/Set (locked at alloc) |
| `TransientVector` | same as Vector (locked at alloc) |
| `ObjectArray` | `capacity × size_of::<Value>()` |
| Primitive arrays | `capacity × size_of::<T>()` |
| `BoundFn` | `capacity × (1 + size_of::<usize>() + size_of::<Value>())` |
| `ExceptionInfo` | `message.capacity()` |

The 40-byte per-entry overhead for HAMT/RBTree is: 16 bytes `Arc` ref-counts +
16 bytes `EntryWithHash`/left-right pointers + 8 bytes tree-node sharing.  The
24-byte overhead for trie vector elements is: 16 bytes `Arc` overhead + 8 bytes
thin pointer in the leaf-node Vec.

### `CljxFn` / `CljxFnArity` (Phase 4)

```rust
// Requires cljrs-reader (for Vec<Form> body).
pub struct CljxFnArity {
    pub params: Vec<Arc<str>>,        // positional param names
    pub rest_param: Option<Arc<str>>, // name after & (if any)
    pub body: Vec<Form>,              // forms in this arity's body
    pub param_hints: Vec<Option<TypeHint>>, // primitive type hint per param (^long, …)
    pub rest_hint: Option<TypeHint>,         // hint on the rest param (rarely used)
    // (also: destructure_params, destructure_rest, ir_arity_id)
}

pub struct CljxFn {
    pub name: Option<Arc<str>>,
    pub arities: Vec<CljxFnArity>,
    pub closed_over_names: Vec<Arc<str>>,
    pub closed_over_vals: Vec<Value>,
    pub is_macro: bool,
    pub is_async: bool,       // ^:async — dispatched via the async runtime when one is registered
    pub defining_ns: Arc<str>,
    pub self_ptr: Option<GcPtr<CljxFn>>, // back-pointer for named-fn pointer identity (issue #194)
}
```

`is_async` is set by the interpreter when a `fn`/`defn` carries `^:async` (or an
`{:async true}` attr-map). `CljxFn::new` defaults it to `false`;
`cljrs_runtime::env`'s `dispatch_if_async` checks it at call time.

`self_ptr` is set immediately after `GcPtr::new(cljrs_fn)` in `eval_fn` (for
named anonymous functions) so that the self-reference returned from the function
body is the *same* `GcPtr` as the outer binding, preserving pointer-equality
semantics (`(= f (f))` → `true`).  `CljxFn::new` defaults it to `None`.

### Var-rebind hooks (`jit_hooks`, Phases 10.2/10.5)

```rust
/// Register a rebind hook. Multiple hooks may be registered; each is called
/// with (old_value, new_value) in registration order.
pub fn set_var_rebind_hook(f: impl Fn(&Value, &Value) + Send + Sync + 'static);
```

`Var::bind` invokes every registered hook (via `notify_var_rebind`) whenever
it overwrites an existing binding.  Two consumers exist: the JIT stales and
reclaims native code compiled for the superseded definition (10.2), and
`cljrs_runtime::tiered`'s defn registry invalidates lowerings of *other* functions that
specialized against it (10.5).  When no hook is registered the cost is a
single atomic flag load.

### Heap-promotion publish barrier (`publish`, Phase 10.5 — GC builds)

```rust
/// Prepare a value for publication into a program-lifetime cell (or another
/// thread): returns the value to store — the original when no region-
/// allocated box is reachable, or a heap deep-copy when one is.  Values
/// opaque to the scan (closures, unrealized lazy seqs, native objects)
/// poison the thread's active regions instead
/// (cljrs_gc::region::poison_active_regions), retiring them at scope close.
/// One thread-local depth check when no region is open.
pub fn publish_value(v: Value) -> Value;
```

The runtime safety net for bump regions coexisting with the tracing GC:
correctness never depends on escape analysis being perfect.  Invoked by
`Var::bind`, `Atom::new`/`Atom::reset`, `Volatile::new`/`Volatile::reset`,
`CljxPromise::deliver`, and `cljrs-async`'s channel puts.  Under `no-gc` the
module is an identity stub (that build keeps its `StaticCtxGuard` discipline).

### `Namespace` (Phase 4)

```rust
pub struct Namespace {
    pub name: Arc<str>,
    pub interns: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,        // own vars
    pub refers: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,         // imported names
    pub aliases: Mutex<HashMap<Arc<str>, Arc<str>>>,          // ns alias → ns name
    pub source_file: Mutex<Option<Arc<str>>>,                 // path the ns was loaded from
    pub git_repo_root: Mutex<Option<Arc<str>>>,               // repo root of source_file, if any
    pub is_versioned: bool,                                   // true for `name@commit` namespaces
    pub meta: Mutex<Option<Value>>,                           // from `(ns ^{...} name ...)` / attr-map
    pub refer_clojure_filter: Mutex<Option<ReferClojureFilter>>, // from `(:refer-clojure ...)`
}

impl Namespace {
    pub fn new(name: impl Into<Arc<str>>) -> Self;
    pub fn new_versioned(name: impl Into<Arc<str>>) -> Self;
    pub fn set_source_location(&self, file: &str, repo_root: Option<&str>);
    pub fn get_meta(&self) -> Option<Value>;
    pub fn set_meta(&self, m: Value);
}
```

### `ReferClojureFilter` (Phase 4)

How much of `clojure.core` a namespace auto-refers, as narrowed by an
`(:refer-clojure ...)` clause in `ns`.  `None` on the namespace means the
default: every public core name is referred.  Applied by
`GlobalEnv::refer_core`, and only there — an explicit refer (`refer_all` /
`refer_named`) bypasses it.

```rust
#[derive(Debug, Clone, Default)]
pub struct ReferClojureFilter {
    pub only: Option<HashSet<Arc<str>>>,     // `:only` — nothing outside this set is referred
    pub exclude: HashSet<Arc<str>>,          // `:exclude` — never referred
    pub rename: HashMap<Arc<str>, Arc<str>>, // `:rename` — core name → local name
}

impl ReferClojureFilter {
    /// The local name `name` is referred under, or `None` when dropped.
    pub fn local_name(&self, name: &Arc<str>) -> Option<Arc<str>>;
}
```

A renamed name is *not* also referred under its original name, matching
`clojure.core/refer`.  `local_name` is a pure lookup: validating the filter
against what core actually defines, and rejecting two names that collide on
one local name, happens once in `GlobalEnv::set_refer_clojure_filter`.

### `Thunk` / `LazySeq` / `CljxCons` (Phase 5)

```rust
pub trait Thunk: Send + Sync + std::fmt::Debug {
    fn force(&self) -> Value;
}

pub struct LazySeq {
    pub state: Mutex<LazySeqState>,  // Pending(Box<dyn Thunk>) | Forced(Value)
}
impl LazySeq {
    pub fn new(thunk: Box<dyn Thunk>) -> Self
    pub fn realize(&self) -> Value   // forces once, caches result
}

pub struct CljxCons {
    pub head: Value,
    pub tail: Value,   // may be LazySeq, Cons, List, or Nil
}
```

`Thunk` implementations live in `cljrs-runtime` (e.g. `ClosureThunk`) so that
`cljrs-value` stays free of evaluator dependencies while `LazySeq` can still
call back through the trait object.

### `TypeInstance` (Phase 6-ext — defrecord/reify)

```rust
pub struct TypeInstance {
    pub type_tag: Arc<str>,  // record name (defrecord) or gensym (reify)
    pub fields: MapValue,    // keyword → value
}
```

Used by `defrecord` (named type_tag, generates `->Name`/`map->Name` constructors) and
`reify` (gensym'd type_tag, no constructors).  Supports keyword field access `(:field rec)`,
`get`, `assoc` (returns new TypeInstance), and `count`.

### `Volatile` / `Delay` / `CljxPromise` / `CljxFuture` / `Agent` (Phase 7)

```rust
pub struct Volatile { pub value: Mutex<Value> }

pub struct Delay { pub state: Mutex<DelayState> }  // Pending(Box<dyn Thunk>) | Forced(Value)

pub struct CljxPromise {
    pub value: Mutex<Option<Value>>,
    pub cond: Condvar,
}

pub struct CljxFuture {
    pub state: Mutex<FutureState>,  // Running | Done(Value) | Failed(Value) | GasExhausted | Cancelled
    pub cond: Condvar,
}

pub struct Agent {
    pub state: Arc<Mutex<Value>>,
    pub error: Arc<Mutex<Option<String>>>,
    pub sender: Mutex<SyncSender<AgentMsg>>,
}
pub type AgentFn = Box<dyn FnOnce(Value) -> Result<Value, String> + Send>;
```

### `Protocol` / `ProtocolFn` / `MultiFn` (Phase 6)

```rust
pub struct Protocol {
    pub name: Arc<str>,
    pub ns: Arc<str>,
    pub methods: Vec<ProtocolMethod>,
    /// type_tag → { method_name → impl fn }
    pub impls: Mutex<HashMap<Arc<str>, MethodMap>>,
    /// Set by `(defprotocol Name :extend-via-metadata true ...)`; see dispatch
    /// note below.
    pub extend_via_metadata: bool,
}

pub struct ProtocolMethod {
    pub name: Arc<str>,
    pub min_arity: usize,
    pub variadic: bool,
}

pub struct ProtocolFn {
    pub protocol: GcPtr<Protocol>,
    pub method_name: Arc<str>,
    pub min_arity: usize,
    pub variadic: bool,
}

pub struct MultiFn {
    pub name: Arc<str>,
    pub dispatch_fn: Value,
    /// pr_str(dispatch-val) → implementation fn
    pub methods: Mutex<HashMap<String, Value>>,
    /// pr_str(dispatch-val) → the dispatch value itself; `apply_value` scans it
    /// with `isa?` when no method matches the key exactly (hierarchy dispatch)
    pub dispatch_vals: Mutex<HashMap<String, Value>>,
    /// pr_str(preferred) → pr_str(over), from `prefer-method`
    pub prefers: Mutex<HashMap<String, Vec<String>>>,
    pub default_dispatch: String,  // normally ":default"
}

/// Phase 10.6 — protocol-dispatch inline-cache invalidation.
/// `bump_protocol_generation()` must follow every mutation of any
/// `Protocol::impls` map (extend-type / extend-protocol / inline impls);
/// `rt_call_ic` (cljrs-compiler) tags each cached dispatch with the
/// generation observed at fill time and re-resolves on mismatch.
pub fn protocol_generation() -> u64;
pub fn bump_protocol_generation();
```

When `Protocol::extend_via_metadata` is set, `apply_value`'s `ProtocolFn` arm
(`cljrs-runtime/src/env/apply.rs`) checks the dispatch value's metadata for an entry
keyed by the exact `ProtocolFn` before falling back to the `impls` type-tag
lookup — see that crate's README for the dispatch order.

### `clone` — isolate copy boundary (Phase B2)

```rust
/// A Send + Sync intermediate representation for cross-isolate transfer.
/// All heap data is owned (no GcPtr); safe to move across thread boundaries.
pub enum SerializedValue { Nil, Bool(bool), Long(i64), /* … */ }

impl SerializedValue {
    /// Estimated heap bytes a deep copy of this value materializes on the
    /// receiver. Telemetry approximation for the metered boundary seam
    /// (`docs/isolate-boundary-plan.md`); Arc-shared payloads count as zero.
    pub fn byte_size(&self) -> usize;
}

/// Reason a value cannot cross an isolate boundary.
pub enum CloneError {
    NotShareable { type_name: &'static str },
    Disconnected,
}

/// Convert a Value to SerializedValue.  Returns CloneError for mutable state,
/// closures, native resources, and other non-shareable types.
pub fn serialize(v: &Value) -> Result<SerializedValue, CloneError>;

/// Allocate a fresh Value in the *current* GC heap from a SerializedValue.
/// Infallible — non-shareable types are rejected at serialize time.
pub fn deserialize(sv: SerializedValue) -> Value;
```

Shareable types: all scalars, strings, BigInt/BigDecimal/Ratio, Symbol/Keyword,
all persistent collections, TypeInstance records, Error chains, primitive and
object arrays, lazy sequences (realized first), WithMeta/Reduced wrappers.

Cross-isolate shared references (Arc passed through, not deep-copied): SharedAtom,
ByteBlob, and Var — a var crosses by sharing its `shared_root` cell, so a value
`def`'d in one isolate is observable by value in another (issue #171). A var
whose current root is non-promotable (closure/resource) is the exception and
returns `CloneError`.

Non-shareable (returns `CloneError`): Atom, Volatile, Promise, Future, Agent
(mutable state); Fn, BoundFn, NativeFn, Macro, ProtocolFn, MultiFn (closures
with isolate-local captures); Namespace, Protocol (global singletons); Resource,
NativeObject (isolate-bound handles); TransientMap/Set/Vector; unforced Delay;
Matcher; Var whose root holds a non-promotable value.

### Dependencies

`cljrs-value` depends on `cljrs-reader` so that `CljxFnArity::body` can store
`Vec<Form>` (unevaluated source bodies for interpreter evaluation and closure
capture).

---

## Features

| Feature | Default | Effect |
|---|---|---|
| `regex-full` | yes | `Value::Pattern` uses the `regex` crate: linear-time matching, full Unicode character classes. |
| `small-regex` | no | `Value::Pattern` uses `regex-lite` instead — roughly a tenth of the code size; `regex-automata`, `regex-syntax` and `aho-corasick` drop out of the build. |
| `no-gc` | no | Forwards `cljrs-gc/no-gc` (region allocation, no tracing GC). |

Exactly one regex engine is compiled in, and `regex-full` wins when both
features are enabled. Cargo features are additive, so that ordering is
deliberate: a build where one dependent asks for `small-regex` and another takes
the default gets the more capable engine rather than a silent semantic
downgrade.

Measured on a stripped release build of the interpreter plus
`clojure.core`/`clojure.string` with `deps` off (`cljrs-stdlib` with
`default-features = false`), swapping the engine takes `.text` from 3.68 MB to
2.89 MB and the binary from 5.77 MB to 4.10 MB.

`small-regex` is a **behaviour** change as well as a size one — `regex-lite` has
no Unicode character classes (`\w`, `\d`, `[[:alpha:]]` and friends are ASCII
only) and is materially slower on pathological patterns. `re-find`/`re-matches`/
`re-seq` over short strings, which is what most Clojure code does with regexes,
are unaffected in behaviour.

Because `regex-full` wins ties, selecting the small engine means turning default
features off on every workspace crate you depend on **directly**. Cargo unions
features across all edges to a package, so an edge left at its defaults re-enables
`regex-full` for the whole graph — and a second, direct dependency declaration
cannot switch it off again. Every workspace crate therefore takes its own internal
dependencies with default features off and re-exports both features, so one
`default-features = false` at your edge is enough:

```toml
[dependencies]
# interpreter + clojure.core/clojure.string, no git dependency support
cljrs-stdlib = { version = "0.1", default-features = false, features = ["small-regex"] }
```

One crate left at its defaults anywhere in the graph puts `regex` back — the same
unification caveat that applies to `cljrs-runtime`'s `deps` feature. And `deps`
itself has to be off for the swap to pay: `cljrs-project/vcs` pulls `regex` in
through `pgp`, so an embedding that resolves git-hosted dependencies links the
full engine no matter what this feature says. Every crate that re-exports the regex pair also
re-exports `deps` for that reason.

The one crate where the swap cannot pay is the CLI: `cljrs` depends on
`cljrs-project` with `vcs`/`vcs-net`/`ssh` unconditionally, so `pgp` — and with it
`regex` — is always in that binary.
