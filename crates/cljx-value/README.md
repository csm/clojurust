# cljx-value

Core runtime values and persistent collections for clojurust.

**Phase:** 3 (collections/Value) + 4 (CljxFn, Namespace) + 5 (LazySeq, CljxCons) + 6 (Protocol, ProtocolFn, MultiFn) — implemented.

---

## Purpose

Defines `Value`, the single enum that represents every Clojure runtime value,
plus all persistent (immutable, structurally shared) collection types.  The
`cljx-eval` crate will operate on `Value`s; `cljx-runtime` will build the
standard library on top of them.

---

## File layout

```
src/
  lib.rs                         — module declarations and re-exports
  error.rs                       — ValueError enum, ValueResult<T> alias
  hash.rs                        — ClojureHash trait, Murmur3 helpers, JVM-compatible hash_string
  keyword.rs                     — Keyword { namespace, name }
  symbol.rs                      — Symbol { namespace, name }
  types.rs                       — Var, Atom, Namespace, NativeFn, CljxFn, Thunk, LazySeq, CljxCons, Protocol, ProtocolFn, ProtocolMethod, MultiFn
  value.rs                       — Value enum, MapValue, pr_str, PartialEq, ClojureHash, std::hash::Hash
  collections/
    mod.rs                       — re-exports all collection types
    array_map.rs                 — PersistentArrayMap (≤8 entries, linear scan)
    hash_map.rs                  — PersistentHashMap (32-way HAMT)
    hash_set.rs                  — PersistentHashSet (backed by PersistentHashMap)
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
    Namespace(GcPtr<Namespace>),
    NativeFn(GcPtr<NativeFn>),
    CljxFn(GcPtr<CljxFn>),
    // Protocols & Multimethods (Phase 6)
    Protocol(GcPtr<Protocol>),
    ProtocolFn(GcPtr<ProtocolFn>),
    MultiFn(GcPtr<MultiFn>),
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
pub struct Symbol   { namespace: Option<Arc<str>>, name: Arc<str> }
pub struct Keyword  { namespace: Option<Arc<str>>, name: Arc<str> }
```

Both support `simple(name)`, `qualified(ns, name)`, `parse(str)`, and
`full_name() -> String`.

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
| `PersistentVector` | 32-way trie + tail buffer | `conj`, `nth`, `assoc_nth`, `pop`, `iter` |
| `PersistentArrayMap` | Flat key/value vec, ≤8 entries | `assoc` (returns `AssocResult`), `get`, `dissoc`, `iter` |
| `PersistentHashMap` | 32-way HAMT | `assoc`, `get`, `dissoc`, `merge`, `iter`, `keys`, `vals` |
| `PersistentHashSet` | Backed by `PersistentHashMap` | `conj`, `disj`, `contains`, `iter` |
| `PersistentQueue` | Front-list + rear-vector | `enqueue`, `dequeue`, `peek` |

`PersistentArrayMap::assoc` returns `AssocResult::Array(Self)` while under the
threshold, or `AssocResult::Promote(Vec<(Value, Value)>)` when the map is full.
`MapValue::assoc` handles the transparent promotion to `PersistentHashMap`.

All collections implement `PartialEq`, `Debug`, `Clone`, and `cljx_gc::Trace`.
`PersistentList`, `PersistentVector`, and `PersistentHashSet` implement
`std::iter::FromIterator<Value>`.

### `CljxFn` / `CljxFnArity` (Phase 4)

```rust
// Requires cljx-reader (for Vec<Form> body).
pub struct CljxFnArity {
    pub params: Vec<Arc<str>>,        // positional param names
    pub rest_param: Option<Arc<str>>, // name after & (if any)
    pub body: Vec<Form>,              // forms in this arity's body
}

pub struct CljxFn {
    pub name: Option<Arc<str>>,
    pub arities: Vec<CljxFnArity>,
    pub closed_over_names: Vec<Arc<str>>,
    pub closed_over_vals: Vec<Value>,
    pub is_macro: bool,
}
```

### `Namespace` (Phase 4)

```rust
pub struct Namespace {
    pub name: Arc<str>,
    pub interns: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,  // own vars
    pub refers: Mutex<HashMap<Arc<str>, GcPtr<Var>>>,   // imported names
    pub aliases: Mutex<HashMap<Arc<str>, Arc<str>>>,    // ns alias → ns name
}
```

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

`Thunk` implementations live in `cljx-eval` (e.g. `ClosureThunk`) so that
`cljx-value` stays free of evaluator dependencies while `LazySeq` can still
call back through the trait object.

### `Protocol` / `ProtocolFn` / `MultiFn` (Phase 6)

```rust
pub struct Protocol {
    pub name: Arc<str>,
    pub ns: Arc<str>,
    pub methods: Vec<ProtocolMethod>,
    /// type_tag → { method_name → impl fn }
    pub impls: Mutex<HashMap<Arc<str>, MethodMap>>,
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
    pub methods: Mutex<HashMap<String, Value>>,
    pub prefers: Mutex<HashMap<String, Vec<String>>>,
    pub default_dispatch: String,  // normally ":default"
}
```

### Dependencies

`cljx-value` depends on `cljx-reader` so that `CljxFnArity::body` can store
`Vec<Form>` (unevaluated source bodies for interpreter evaluation and closure
capture).
