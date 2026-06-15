//! Structured-clone boundary between isolates (Phase B2).
//!
//! `serialize` converts a `Value` to a `Send + Sync` intermediate form;
//! `deserialize` allocates a fresh copy into the *current* isolate's GC heap.
//! Non-shareable values (mutable state, closures, native resources) produce a
//! [`CloneError`] so the compiler — not a runtime panic — enforces the boundary.
//!
//! The round-trip is:
//!
//! ```text
//! isolate A: Value → serialize → SerializedValue   (Send) ──► thread boundary
//! isolate B:                      SerializedValue → deserialize → Value
//! ```
//!
//! ## What is shareable
//!
//! - All scalar immediates: `Nil`, `Bool`, `Long`, `Double`, `Char`, `Uuid`
//! - Heap-allocated *data* values: `Str`, `BigInt`, `BigDecimal`, `Ratio`,
//!   `Pattern` (source only), `Symbol`, `Keyword`
//! - All persistent collections: `List`, `Vector`, `Map`, `Set`, `Queue`, `Cons`
//! - Primitive and object arrays (snapshot of current contents)
//! - `TypeInstance` records (fields cloned recursively)
//! - `Error` (message + data + cause chain, `Thrown` value cloned recursively)
//! - Lazy sequences: **realized** first; the realized value is then cloned
//! - `WithMeta`, `Reduced` wrappers (inner value + meta cloned recursively)
//!
//! ## Cross-isolate shared references (Phase B3)
//!
//! - `SharedAtom`, `ByteBlob`  — `Arc`-cloned, not deep-copied; both isolates
//!   share the same underlying cell/buffer.
//! - `Var`  — the var's *root* binding crosses through its shared cell
//!   (`Arc<ArcSwap<Option<SharedValue>>>`), so a var `def`'d in one isolate is
//!   observable by value from another, keyword/symbol identity preserved.  A
//!   var whose current root holds a non-promotable value (a closure / native
//!   resource) is **not** shareable and returns `CloneError` — such vars are
//!   explicitly isolate-local (option (b) of the ADR).
//!
//! ## What is *not* shareable (returns `CloneError`)
//!
//! - `Atom`, `Volatile`, `Promise`, `Future`, `Agent`  (mutable state)
//! - `Fn`, `BoundFn`, `Macro`, `NativeFunction`, `ProtocolFn`, `MultiFn`
//!   (closures capture isolate-local `GcPtr`s)
//! - `Namespace`, `Protocol`  (global singletons managed elsewhere)
//! - `Resource`, `NativeObject`  (isolate-bound OS handles / native objects)
//! - `TransientMap`, `TransientSet`, `TransientVector`  (isolate-local transients)
//! - `Delay` whose thunk has not yet been forced  (thunk is isolate-local)
//! - `Matcher`  (regex engine state tied to one execution context)

use std::sync::Arc;

use arc_swap::ArcSwap;
use num_bigint::BigInt;

use crate::collections::{PersistentHashSet, PersistentVector, SortedMap, SortedSet};
use crate::error::ValueError;
use crate::shared::{SharedAtom, SharedValue};
use crate::types::DelayState;
use crate::{Keyword, MapValue, PersistentList, PersistentQueue, SetValue, Symbol, Value};
use cljrs_gc::GcPtr;

// ── Error ────────────────────────────────────────────────────────────────────

/// Reason a value cannot cross an isolate boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CloneError {
    /// The value holds isolate-local state that cannot be serialized.
    NotShareable {
        /// Clojure type name (matches `Value::type_name()`).
        type_name: &'static str,
    },
    /// The channel's receiver side has been dropped.
    Disconnected,
}

impl std::fmt::Display for CloneError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CloneError::NotShareable { type_name } => {
                write!(
                    f,
                    "value of type `{type_name}` cannot cross an isolate boundary"
                )
            }
            CloneError::Disconnected => write!(f, "isolate channel is disconnected"),
        }
    }
}

impl std::error::Error for CloneError {}

fn not_shareable(type_name: &'static str) -> CloneError {
    CloneError::NotShareable { type_name }
}

// ── Wire form ─────────────────────────────────────────────────────────────────

/// Send + Sync intermediate form produced by `serialize` and consumed by
/// `deserialize`. All heap data is owned (no `GcPtr`), so it is safe to move
/// across thread boundaries.
#[derive(Clone, Debug)]
pub enum SerializedValue {
    // Scalars
    Nil,
    Bool(bool),
    Long(i64),
    Double(f64),
    BigInt(BigInt),
    BigDecimal(bigdecimal::BigDecimal),
    Ratio(num_rational::Ratio<BigInt>),
    Char(char),
    Str(String),
    Uuid(u128),
    /// Regex stored as source string; recompiled on deserialize.
    Pattern(String),

    // Identifiers
    Symbol {
        namespace: Option<Arc<str>>,
        name: Arc<str>,
        version: Option<Arc<str>>,
    },
    Keyword {
        namespace: Option<Arc<str>>,
        name: Arc<str>,
    },

    // Collections
    List(Vec<SerializedValue>),
    Vector(Vec<SerializedValue>),
    ArrayMap(Vec<(SerializedValue, SerializedValue)>),
    HashMap(Vec<(SerializedValue, SerializedValue)>),
    SortedMap(Vec<(SerializedValue, SerializedValue)>),
    HashSet(Vec<SerializedValue>),
    SortedSet(Vec<SerializedValue>),
    Queue(Vec<SerializedValue>),
    Cons {
        head: Box<SerializedValue>,
        tail: Box<SerializedValue>,
    },

    // Records
    TypeInstance {
        type_tag: Arc<str>,
        fields: Vec<(SerializedValue, SerializedValue)>,
    },

    // Errors
    Error(Box<SerializedError>),

    // Primitive arrays (snapshot of current contents)
    BooleanArray(Vec<bool>),
    ByteArray(Vec<i8>),
    ShortArray(Vec<i16>),
    IntArray(Vec<i32>),
    LongArray(Vec<i64>),
    FloatArray(Vec<f32>),
    DoubleArray(Vec<f64>),
    CharArray(Vec<char>),
    ObjectArray(Vec<SerializedValue>),

    // Wrappers
    WithMeta {
        value: Box<SerializedValue>,
        meta: Box<SerializedValue>,
    },
    Reduced(Box<SerializedValue>),

    // Phase B3: cross-isolate shared references (Arc cloned, not deep-copied).
    /// `SharedAtom` is inherently cross-isolate; the `Arc` is simply cloned so
    /// both isolates share the same underlying `ArcSwap` cell.
    SharedAtom(Arc<SharedAtom>),
    /// `ByteBlob` is an immutable refcounted buffer; clone the `Arc`.
    ByteBlob(Arc<[u8]>),
    /// A var crosses by sharing its cross-isolate root cell (the `Arc` is
    /// cloned, so both isolates point at the same `ArcSwap`).  The receiving
    /// isolate rebuilds a `Var` whose local fast-path slot is the demoted
    /// current snapshot.  Only vars with a promotable (or unbound) root reach
    /// here — a non-promotable root is rejected at `serialize` time.
    Var {
        namespace: Arc<str>,
        name: Arc<str>,
        is_macro: bool,
        shared_root: Arc<ArcSwap<Option<SharedValue>>>,
    },
}

// Compile-time Send + Sync assertions.
const _: () = {
    const fn _assert_send<T: Send + Sync>() {}
    let _ = _assert_send::<SerializedValue>;
};

impl SerializedValue {
    /// Estimated heap bytes materialized by deep-copying this value into the
    /// receiving isolate's heap. This is an approximation for **telemetry**
    /// (the metered clone seam the isolate-boundary plan requires), not an
    /// exact allocation count: each node contributes a fixed per-node cost plus
    /// the size of any owned payload (string bytes, array elements, big-number
    /// magnitude). `Arc`-shared payloads (`SharedAtom`, `ByteBlob`) count as
    /// zero structural bytes because they cross by refcount, not by copy.
    pub fn byte_size(&self) -> usize {
        // Per-node base cost: one `SerializedValue` slot plus the GcPtr/box the
        // deserialized form allocates on the receiver side.
        const NODE: usize = std::mem::size_of::<SerializedValue>();

        let payload = match self {
            SerializedValue::Nil
            | SerializedValue::Bool(_)
            | SerializedValue::Long(_)
            | SerializedValue::Double(_)
            | SerializedValue::Char(_)
            | SerializedValue::Uuid(_) => 0,

            SerializedValue::BigInt(b) => (b.bits() as usize / 8) + 1,
            SerializedValue::Ratio(r) => {
                (r.numer().bits() as usize + r.denom().bits() as usize) / 8 + 2
            }
            SerializedValue::BigDecimal(_) => 16,

            SerializedValue::Str(s) => s.len(),
            SerializedValue::Pattern(s) => s.len(),

            SerializedValue::Symbol {
                namespace, name, ..
            } => namespace.as_ref().map_or(0, |n| n.len()) + name.len(),
            SerializedValue::Keyword { namespace, name } => {
                namespace.as_ref().map_or(0, |n| n.len()) + name.len()
            }

            SerializedValue::List(items)
            | SerializedValue::Vector(items)
            | SerializedValue::HashSet(items)
            | SerializedValue::SortedSet(items)
            | SerializedValue::Queue(items)
            | SerializedValue::ObjectArray(items) => {
                items.iter().map(SerializedValue::byte_size).sum()
            }

            SerializedValue::ArrayMap(pairs)
            | SerializedValue::HashMap(pairs)
            | SerializedValue::SortedMap(pairs) => pairs
                .iter()
                .map(|(k, v)| k.byte_size() + v.byte_size())
                .sum(),

            SerializedValue::TypeInstance { fields, .. } => fields
                .iter()
                .map(|(k, v)| k.byte_size() + v.byte_size())
                .sum(),

            SerializedValue::Cons { head, tail } => head.byte_size() + tail.byte_size(),

            SerializedValue::Error(e) => e.byte_size(),

            SerializedValue::BooleanArray(v) => v.len(),
            SerializedValue::ByteArray(v) => v.len(),
            SerializedValue::ShortArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::IntArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::LongArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::FloatArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::DoubleArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::CharArray(v) => std::mem::size_of_val(v.as_slice()),

            SerializedValue::WithMeta { value, meta } => value.byte_size() + meta.byte_size(),
            SerializedValue::Reduced(inner) => inner.byte_size(),

            // Arc-shared: crosses by refcount bump, no structural copy.
            SerializedValue::SharedAtom(_) | SerializedValue::ByteBlob(_) => 0,

            // The var's root cell is Arc-shared; only the ns/name strings copy.
            SerializedValue::Var {
                namespace, name, ..
            } => namespace.len() + name.len(),
        };

        NODE + payload
    }
}

impl SerializedError {
    fn byte_size(&self) -> usize {
        std::mem::size_of::<SerializedError>()
            + self.message.len()
            + self.data.as_ref().map_or(0, |pairs| {
                pairs
                    .iter()
                    .map(|(k, v)| k.byte_size() + v.byte_size())
                    .sum()
            })
            + self.cause.as_ref().map_or(0, |c| c.byte_size())
    }
}

/// Serialized form of [`crate::error::ExceptionInfo`].
#[derive(Clone, Debug)]
pub struct SerializedError {
    pub kind: SerializedErrorKind,
    pub message: String,
    pub data: Option<Vec<(SerializedValue, SerializedValue)>>,
    pub cause: Option<Box<SerializedError>>,
}

/// Mirrors [`ValueError`] with `Value` replaced by `SerializedValue`.
#[derive(Clone, Debug)]
pub enum SerializedErrorKind {
    WrongType {
        expected: &'static str,
        got: String,
    },
    IndexOutOfBounds {
        idx: usize,
        count: usize,
    },
    ArityError {
        name: String,
        expected: String,
        got: usize,
    },
    NotCallable {
        value: String,
    },
    OddMap {
        count: usize,
    },
    Unsupported,
    Other(String),
    OutOfRange,
    TransientAlreadyPersisted,
    Parse,
    Thrown(Box<SerializedValue>),
}

// ── serialize ─────────────────────────────────────────────────────────────────

/// Serialize a `Value` into a `Send + Sync` wire form suitable for crossing an
/// isolate boundary. Returns [`CloneError`] for non-shareable values.
pub fn serialize(v: &Value) -> Result<SerializedValue, CloneError> {
    match v {
        // ── Wrappers ──
        Value::WithMeta(inner, meta) => Ok(SerializedValue::WithMeta {
            value: Box::new(serialize(inner)?),
            meta: Box::new(serialize(meta)?),
        }),
        Value::Reduced(inner) => Ok(SerializedValue::Reduced(Box::new(serialize(inner)?))),

        // ── Scalars ──
        Value::Nil => Ok(SerializedValue::Nil),
        Value::Bool(b) => Ok(SerializedValue::Bool(*b)),
        Value::Long(n) => Ok(SerializedValue::Long(*n)),
        Value::Double(d) => Ok(SerializedValue::Double(*d)),
        Value::Char(c) => Ok(SerializedValue::Char(*c)),
        Value::Uuid(u) => Ok(SerializedValue::Uuid(*u)),

        Value::BigInt(p) => Ok(SerializedValue::BigInt(p.get().clone())),
        Value::BigDecimal(p) => Ok(SerializedValue::BigDecimal(p.get().clone())),
        Value::Ratio(p) => Ok(SerializedValue::Ratio(p.get().clone())),
        Value::Str(p) => Ok(SerializedValue::Str(p.get().clone())),
        Value::Pattern(p) => Ok(SerializedValue::Pattern(p.get().as_str().to_owned())),

        // ── Identifiers ──
        Value::Symbol(p) => {
            let s = p.get();
            Ok(SerializedValue::Symbol {
                namespace: s.namespace.clone(),
                name: s.name.clone(),
                version: s.version.clone(),
            })
        }
        Value::Keyword(p) => {
            let k = p.get();
            Ok(SerializedValue::Keyword {
                namespace: k.namespace.clone(),
                name: k.name.clone(),
            })
        }

        // ── Collections ──
        Value::List(p) => {
            let items: Result<Vec<_>, _> = p.get().iter().map(serialize).collect();
            Ok(SerializedValue::List(items?))
        }
        Value::Vector(p) => {
            let items: Result<Vec<_>, _> = p.get().iter().map(serialize).collect();
            Ok(SerializedValue::Vector(items?))
        }
        Value::Map(m) => serialize_map(m),
        Value::Set(s) => serialize_set(s),
        Value::Queue(p) => {
            let items: Result<Vec<_>, _> = p.get().iter().map(serialize).collect();
            Ok(SerializedValue::Queue(items?))
        }
        Value::Cons(p) => {
            let c = p.get();
            Ok(SerializedValue::Cons {
                head: Box::new(serialize(&c.head)?),
                tail: Box::new(serialize(&c.tail)?),
            })
        }

        // ── Records ──
        Value::TypeInstance(p) => {
            let ti = p.get();
            let fields = serialize_map_pairs(&ti.fields)?;
            Ok(SerializedValue::TypeInstance {
                type_tag: ti.type_tag.clone(),
                fields,
            })
        }

        // ── Errors ──
        Value::Error(p) => Ok(SerializedValue::Error(Box::new(serialize_error(p.get())?))),

        // ── Primitive arrays (snapshot) ──
        Value::BooleanArray(p) => Ok(SerializedValue::BooleanArray(
            p.get().lock().unwrap().clone(),
        )),
        Value::ByteArray(p) => Ok(SerializedValue::ByteArray(p.get().lock().unwrap().clone())),
        Value::ShortArray(p) => Ok(SerializedValue::ShortArray(p.get().lock().unwrap().clone())),
        Value::IntArray(p) => Ok(SerializedValue::IntArray(p.get().lock().unwrap().clone())),
        Value::LongArray(p) => Ok(SerializedValue::LongArray(p.get().lock().unwrap().clone())),
        Value::FloatArray(p) => Ok(SerializedValue::FloatArray(p.get().lock().unwrap().clone())),
        Value::DoubleArray(p) => Ok(SerializedValue::DoubleArray(
            p.get().lock().unwrap().clone(),
        )),
        Value::CharArray(p) => Ok(SerializedValue::CharArray(p.get().lock().unwrap().clone())),
        Value::ObjectArray(p) => {
            let guard = p.get().0.lock().unwrap();
            let items: Result<Vec<_>, _> = guard.iter().map(serialize).collect();
            Ok(SerializedValue::ObjectArray(items?))
        }

        // ── Lazy sequences: realize first ──
        Value::LazySeq(p) => serialize(&p.get().realize()),

        // ── Delay: force if already realized, else error ──
        Value::Delay(p) => {
            let state = p.get().state.lock().unwrap();
            if let DelayState::Forced(v) = &*state {
                serialize(v)
            } else {
                Err(not_shareable("delay"))
            }
        }

        // ── Non-shareable ──
        Value::Resource(_) => Err(not_shareable("resource")),
        Value::NativeObject(_) => Err(not_shareable("native-object")),
        Value::Matcher(_) => Err(not_shareable("matcher")),

        Value::Fn(_) | Value::Macro(_) => Err(not_shareable("fn")),
        Value::BoundFn(_) => Err(not_shareable("fn")),
        Value::NativeFunction(_) => Err(not_shareable("fn")),
        Value::ProtocolFn(_) => Err(not_shareable("fn")),
        Value::MultiFn(_) => Err(not_shareable("fn")),

        // ── Phase B3: vars cross by sharing their root cell ──
        Value::Var(p) => {
            let var = p.get();
            // The shared cell is kept in sync by `Var::bind` (promote-on-def).
            // It is empty when the var is unbound *or* its current root is not
            // promotable.  Distinguish the two: an unbound var may cross (it
            // arrives unbound), but a var bound to a non-promotable value
            // (closure / native resource) is explicitly isolate-local and is
            // rejected here — a non-silent boundary error, not a silent drop.
            let shared_empty = var.shared_root.load().is_none();
            if shared_empty && var.is_bound() {
                return Err(not_shareable("var"));
            }
            Ok(SerializedValue::Var {
                namespace: var.namespace.clone(),
                name: var.name.clone(),
                is_macro: var.is_macro,
                shared_root: var.shared_root.clone(),
            })
        }
        Value::Atom(_) => Err(not_shareable("atom")),
        Value::Volatile(_) => Err(not_shareable("volatile")),
        Value::Promise(_) => Err(not_shareable("promise")),
        Value::Future(_) => Err(not_shareable("future")),
        Value::Agent(_) => Err(not_shareable("agent")),

        Value::Namespace(_) => Err(not_shareable("namespace")),
        Value::Protocol(_) => Err(not_shareable("protocol")),

        Value::TransientMap(_) => Err(not_shareable("transient-map")),
        Value::TransientSet(_) => Err(not_shareable("transient-set")),
        Value::TransientVector(_) => Err(not_shareable("transient-vector")),

        // ── Phase B3: cross-isolate shared references (pass Arc through) ──
        Value::SharedAtom(a) => Ok(SerializedValue::SharedAtom(a.clone())),
        Value::ByteBlob(b) => Ok(SerializedValue::ByteBlob(b.clone())),
    }
}

fn serialize_map(m: &MapValue) -> Result<SerializedValue, CloneError> {
    let pairs = serialize_map_pairs(m)?;
    Ok(match m {
        MapValue::Array(_) => SerializedValue::ArrayMap(pairs),
        MapValue::Hash(_) => SerializedValue::HashMap(pairs),
        MapValue::Sorted(_) => SerializedValue::SortedMap(pairs),
    })
}

fn serialize_map_pairs(
    m: &MapValue,
) -> Result<Vec<(SerializedValue, SerializedValue)>, CloneError> {
    let mut pairs = Vec::with_capacity(m.count());
    let mut err: Option<CloneError> = None;
    m.for_each(|k, v| {
        if err.is_some() {
            return;
        }
        match (serialize(k), serialize(v)) {
            (Ok(sk), Ok(sv)) => pairs.push((sk, sv)),
            (Err(e), _) | (_, Err(e)) => err = Some(e),
        }
    });
    if let Some(e) = err { Err(e) } else { Ok(pairs) }
}

fn serialize_set(s: &SetValue) -> Result<SerializedValue, CloneError> {
    let items: Result<Vec<_>, _> = s.iter().map(serialize).collect();
    Ok(match s {
        SetValue::Hash(_) => SerializedValue::HashSet(items?),
        SetValue::Sorted(_) => SerializedValue::SortedSet(items?),
    })
}

fn serialize_error(e: &crate::error::ExceptionInfo) -> Result<SerializedError, CloneError> {
    let kind = match &e.error {
        ValueError::WrongType { expected, got } => SerializedErrorKind::WrongType {
            expected,
            got: got.clone(),
        },
        ValueError::IndexOutOfBounds { idx, count } => SerializedErrorKind::IndexOutOfBounds {
            idx: *idx,
            count: *count,
        },
        ValueError::ArityError {
            name,
            expected,
            got,
        } => SerializedErrorKind::ArityError {
            name: name.clone(),
            expected: expected.clone(),
            got: *got,
        },
        ValueError::NotCallable { value } => SerializedErrorKind::NotCallable {
            value: value.clone(),
        },
        ValueError::OddMap { count } => SerializedErrorKind::OddMap { count: *count },
        ValueError::Unsupported => SerializedErrorKind::Unsupported,
        ValueError::Other(s) => SerializedErrorKind::Other(s.clone()),
        ValueError::OutOfRange => SerializedErrorKind::OutOfRange,
        ValueError::TransientAlreadyPersisted => SerializedErrorKind::TransientAlreadyPersisted,
        ValueError::Parse => SerializedErrorKind::Parse,
        ValueError::Thrown(v) => SerializedErrorKind::Thrown(Box::new(serialize(v)?)),
    };

    let data = e.data.as_ref().map(serialize_map_pairs).transpose()?;

    let cause = e
        .cause
        .as_ref()
        .map(|c| serialize_error(c.get()).map(Box::new))
        .transpose()?;

    Ok(SerializedError {
        kind,
        message: e.message.clone(),
        data,
        cause,
    })
}

// ── deserialize ───────────────────────────────────────────────────────────────

/// Deserialize a wire form into a fresh `Value` allocated in the *current*
/// isolate's GC heap. Infallible: all non-shareable values are rejected at
/// `serialize` time, so nothing in `SerializedValue` requires runtime checks.
pub fn deserialize(sv: SerializedValue) -> Value {
    match sv {
        SerializedValue::WithMeta { value, meta } => {
            Value::WithMeta(Box::new(deserialize(*value)), Box::new(deserialize(*meta)))
        }
        SerializedValue::Reduced(inner) => Value::Reduced(Box::new(deserialize(*inner))),

        SerializedValue::Nil => Value::Nil,
        SerializedValue::Bool(b) => Value::Bool(b),
        SerializedValue::Long(n) => Value::Long(n),
        SerializedValue::Double(d) => Value::Double(d),
        SerializedValue::Char(c) => Value::Char(c),
        SerializedValue::Uuid(u) => Value::Uuid(u),

        SerializedValue::BigInt(n) => Value::BigInt(GcPtr::new(n)),
        SerializedValue::BigDecimal(d) => Value::BigDecimal(GcPtr::new(d)),
        SerializedValue::Ratio(r) => Value::Ratio(GcPtr::new(r)),
        SerializedValue::Str(s) => Value::Str(GcPtr::new(s)),
        SerializedValue::Pattern(src) => Value::Pattern(GcPtr::new(
            regex::Regex::new(&src).expect("pattern was valid at serialize time"),
        )),

        SerializedValue::Symbol {
            namespace,
            name,
            version,
        } => Value::Symbol(GcPtr::new(Symbol {
            namespace,
            name,
            version,
        })),
        SerializedValue::Keyword { namespace, name } => {
            Value::Keyword(GcPtr::new(Keyword { namespace, name }))
        }

        SerializedValue::List(items) => Value::List(GcPtr::new(PersistentList::from_iter(
            items.into_iter().map(deserialize),
        ))),
        SerializedValue::Vector(items) => Value::Vector(GcPtr::new(PersistentVector::from_iter(
            items.into_iter().map(deserialize),
        ))),
        SerializedValue::ArrayMap(pairs) => Value::Map(MapValue::from_pairs(
            pairs
                .into_iter()
                .map(|(k, v)| (deserialize(k), deserialize(v)))
                .collect(),
        )),
        SerializedValue::HashMap(pairs) => Value::Map(MapValue::from_pairs(
            pairs
                .into_iter()
                .map(|(k, v)| (deserialize(k), deserialize(v)))
                .collect(),
        )),
        SerializedValue::SortedMap(pairs) => {
            // Rebuild as a sorted map through the standard sorted-map path.
            let items: Vec<(Value, Value)> = pairs
                .into_iter()
                .map(|(k, v)| (deserialize(k), deserialize(v)))
                .collect();
            let sm = SortedMap::from_pairs(items);
            Value::Map(MapValue::Sorted(GcPtr::new(sm)))
        }
        SerializedValue::HashSet(items) => {
            let mut hs = PersistentHashSet::empty();
            for item in items.into_iter().map(deserialize) {
                hs = hs.conj(item);
            }
            Value::Set(SetValue::Hash(GcPtr::new(hs)))
        }
        SerializedValue::SortedSet(items) => {
            let mut ss = SortedSet::empty();
            for item in items.into_iter().map(deserialize) {
                ss = ss.conj(item);
            }
            Value::Set(SetValue::Sorted(GcPtr::new(ss)))
        }
        SerializedValue::Queue(items) => {
            let mut q = PersistentQueue::empty();
            for item in items.into_iter().map(deserialize) {
                q = q.conj(item);
            }
            Value::Queue(GcPtr::new(q))
        }
        SerializedValue::Cons { head, tail } => {
            use crate::types::CljxCons;
            Value::Cons(GcPtr::new(CljxCons {
                head: deserialize(*head),
                tail: deserialize(*tail),
            }))
        }

        SerializedValue::TypeInstance { type_tag, fields } => {
            use crate::value::TypeInstance;
            let pairs: Vec<(Value, Value)> = fields
                .into_iter()
                .map(|(k, v)| (deserialize(k), deserialize(v)))
                .collect();
            Value::TypeInstance(GcPtr::new(TypeInstance {
                type_tag,
                fields: MapValue::from_pairs(pairs),
            }))
        }

        SerializedValue::Error(se) => Value::Error(GcPtr::new(deserialize_error(*se))),

        SerializedValue::BooleanArray(v) => {
            Value::BooleanArray(GcPtr::new(std::sync::Mutex::new(v)))
        }
        SerializedValue::ByteArray(v) => Value::ByteArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::ShortArray(v) => Value::ShortArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::IntArray(v) => Value::IntArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::LongArray(v) => Value::LongArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::FloatArray(v) => Value::FloatArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::DoubleArray(v) => Value::DoubleArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::CharArray(v) => Value::CharArray(GcPtr::new(std::sync::Mutex::new(v))),
        SerializedValue::ObjectArray(items) => {
            use crate::value::ObjectArray;
            Value::ObjectArray(GcPtr::new(ObjectArray::new(
                items.into_iter().map(deserialize).collect(),
            )))
        }

        // Phase B3: cross-isolate shared references — clone the Arc.
        SerializedValue::SharedAtom(a) => Value::SharedAtom(a),
        SerializedValue::ByteBlob(b) => Value::ByteBlob(b),
        SerializedValue::Var {
            namespace,
            name,
            is_macro,
            shared_root,
        } => Value::Var(GcPtr::new(crate::types::Var::from_shared_root(
            namespace,
            name,
            is_macro,
            shared_root,
        ))),
    }
}

fn deserialize_error(se: SerializedError) -> crate::error::ExceptionInfo {
    let error = match se.kind {
        SerializedErrorKind::WrongType { expected, got } => ValueError::WrongType { expected, got },
        SerializedErrorKind::IndexOutOfBounds { idx, count } => {
            ValueError::IndexOutOfBounds { idx, count }
        }
        SerializedErrorKind::ArityError {
            name,
            expected,
            got,
        } => ValueError::ArityError {
            name,
            expected,
            got,
        },
        SerializedErrorKind::NotCallable { value } => ValueError::NotCallable { value },
        SerializedErrorKind::OddMap { count } => ValueError::OddMap { count },
        SerializedErrorKind::Unsupported => ValueError::Unsupported,
        SerializedErrorKind::Other(s) => ValueError::Other(s),
        SerializedErrorKind::OutOfRange => ValueError::OutOfRange,
        SerializedErrorKind::TransientAlreadyPersisted => ValueError::TransientAlreadyPersisted,
        SerializedErrorKind::Parse => ValueError::Parse,
        SerializedErrorKind::Thrown(sv) => ValueError::Thrown(deserialize(*sv)),
    };

    let data = se.data.map(|pairs| {
        MapValue::from_pairs(
            pairs
                .into_iter()
                .map(|(k, v)| (deserialize(k), deserialize(v)))
                .collect(),
        )
    });

    let cause = se.cause.map(|c| GcPtr::new(deserialize_error(*c)));

    crate::error::ExceptionInfo::new(error, se.message, data, cause)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(v: &Value) -> Value {
        deserialize(serialize(v).expect("serialize"))
    }

    #[test]
    fn scalars_roundtrip() {
        assert_eq!(roundtrip(&Value::Nil), Value::Nil);
        assert_eq!(roundtrip(&Value::Bool(true)), Value::Bool(true));
        assert_eq!(roundtrip(&Value::Long(42)), Value::Long(42));
        assert_eq!(roundtrip(&Value::Char('x')), Value::Char('x'));
        assert_eq!(roundtrip(&Value::Uuid(12345)), Value::Uuid(12345));
    }

    #[test]
    fn string_roundtrip() {
        let v = Value::string("hello, world");
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn keyword_roundtrip() {
        let v = Value::keyword(Keyword::simple("foo"));
        assert_eq!(roundtrip(&v), v);
        let v2 = Value::keyword(Keyword::qualified("clojure.core", "map"));
        assert_eq!(roundtrip(&v2), v2);
    }

    #[test]
    fn symbol_roundtrip() {
        let v = Value::symbol(Symbol::simple("my-fn"));
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn list_roundtrip() {
        let v = Value::List(GcPtr::new(PersistentList::from_iter([
            Value::Long(1),
            Value::Long(2),
            Value::Long(3),
        ])));
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn vector_roundtrip() {
        let v = Value::Vector(GcPtr::new(PersistentVector::from_iter([
            Value::string("a"),
            Value::Bool(false),
            Value::Nil,
        ])));
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn nested_map_roundtrip() {
        let inner = Value::Vector(GcPtr::new(PersistentVector::from_iter([Value::Long(1)])));
        let v = MapValue::from_pairs(vec![(Value::keyword(Keyword::simple("k")), inner)]);
        let v = Value::Map(v);
        assert_eq!(roundtrip(&v), v);
    }

    #[test]
    fn resource_not_shareable() {
        use crate::resource::ResourceHandle;
        use std::any::Any;
        use std::sync::Arc;
        #[derive(Debug)]
        struct FakeResource;
        impl crate::resource::Resource for FakeResource {
            fn resource_type(&self) -> &'static str {
                "fake"
            }
            fn close(&self) -> crate::error::ValueResult<()> {
                Ok(())
            }
            fn is_closed(&self) -> bool {
                false
            }
            fn as_any(&self) -> &dyn Any {
                self
            }
        }
        let r = Value::Resource(ResourceHandle(Arc::new(FakeResource)));
        assert!(matches!(
            serialize(&r),
            Err(CloneError::NotShareable {
                type_name: "resource"
            })
        ));
    }

    #[test]
    fn atom_not_shareable() {
        use crate::types::Atom;
        let a = Value::Atom(GcPtr::new(Atom::new(Value::Nil)));
        assert!(matches!(
            serialize(&a),
            Err(CloneError::NotShareable { type_name: "atom" })
        ));
    }

    #[test]
    fn fn_not_shareable() {
        use crate::types::{Arity, NativeFn};
        let nf = Value::NativeFunction(GcPtr::new(NativeFn::new("test", Arity::Fixed(0), |_| {
            Ok(Value::Nil)
        })));
        assert!(matches!(
            serialize(&nf),
            Err(CloneError::NotShareable { type_name: "fn" })
        ));
    }

    #[test]
    fn lazy_seq_realized_roundtrip() {
        use crate::types::{LazySeq, Thunk};
        struct DoneThunk;
        impl std::fmt::Debug for DoneThunk {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "DoneThunk")
            }
        }
        impl cljrs_gc::Trace for DoneThunk {
            fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
        }
        impl Thunk for DoneThunk {
            fn force(&self) -> Result<Value, String> {
                Ok(Value::Long(99))
            }
        }
        let ls = LazySeq::new(Box::new(DoneThunk));
        let _ = ls.realize(); // force it
        let v = Value::LazySeq(GcPtr::new(ls));
        assert_eq!(roundtrip(&v), Value::Long(99));
    }

    #[test]
    fn with_meta_roundtrip() {
        let v = Value::Long(7).with_meta(Value::Map(MapValue::empty()));
        let rt = roundtrip(&v);
        // WithMeta strips for equality, so unwrap and check inner
        assert_eq!(rt, Value::Long(7));
    }

    #[test]
    fn reduced_roundtrip() {
        let v = Value::Reduced(Box::new(Value::Long(55)));
        assert_eq!(roundtrip(&v), Value::Reduced(Box::new(Value::Long(55))));
    }

    #[test]
    fn byte_size_counts_string_payload() {
        let small = serialize(&Value::string("hi")).unwrap();
        let large = serialize(&Value::string("x".repeat(1000))).unwrap();
        // Same variant, so the difference is the string payload (~998 bytes).
        assert!(large.byte_size() > small.byte_size() + 900);
    }

    #[test]
    fn byte_size_grows_with_collection() {
        let small = serialize(&Value::Vector(GcPtr::new(PersistentVector::from_iter([
            Value::Long(1),
        ]))))
        .unwrap();
        let large = serialize(&Value::Vector(GcPtr::new(PersistentVector::from_iter(
            (0..100).map(Value::Long),
        ))))
        .unwrap();
        assert!(large.byte_size() > small.byte_size());
    }

    #[test]
    fn var_with_promotable_root_roundtrips() {
        use crate::types::Var;
        // A var def'd with a promotable value crosses by value.
        let var = Var::new("user", "answer");
        var.bind(Value::Long(42));
        let v = Value::Var(GcPtr::new(var));

        let crossed = roundtrip(&v);
        if let Value::Var(p) = crossed {
            let got = p.get();
            assert_eq!(got.namespace.as_ref(), "user");
            assert_eq!(got.name.as_ref(), "answer");
            // Observable by value on the receiving side.
            assert_eq!(got.deref(), Some(Value::Long(42)));
        } else {
            panic!("expected a Var on the receiving side");
        }
    }

    #[test]
    fn var_keyword_root_preserves_identity() {
        use crate::types::Var;
        let var = Var::new("user", "k");
        var.bind(Value::keyword(Keyword::qualified("ns", "kw")));
        let v = Value::Var(GcPtr::new(var));

        let crossed = roundtrip(&v);
        let Value::Var(p) = crossed else {
            panic!("expected Var")
        };
        // Keyword identity preserved through the intern table on demote.
        assert_eq!(
            p.get().deref(),
            Some(Value::keyword(Keyword::qualified("ns", "kw")))
        );
    }

    #[test]
    fn var_shares_root_cell_across_boundary() {
        use crate::types::Var;
        // Both isolates point at the *same* shared root cell, so a write on the
        // sending side is observable through the receiver's shared view.
        let var = GcPtr::new(Var::new("user", "shared"));
        var.get().bind(Value::Long(1));
        let v = Value::Var(var.clone());

        let Value::Var(recv) = roundtrip(&v) else {
            panic!("expected Var")
        };
        assert_eq!(recv.get().deref_shared(), Some(Value::Long(1)));

        // Sender re-defs through the same cell; receiver observes via the cell.
        var.get().bind(Value::Long(2));
        assert_eq!(recv.get().deref_shared(), Some(Value::Long(2)));
    }

    #[test]
    fn var_with_nonpromotable_root_is_not_shareable() {
        use crate::types::{Arity, NativeFn, Var};
        // A var bound to a closure / native fn is explicitly isolate-local.
        let var = Var::new("user", "f");
        var.bind(Value::NativeFunction(GcPtr::new(NativeFn::new(
            "f",
            Arity::Fixed(0),
            |_| Ok(Value::Nil),
        ))));
        let v = Value::Var(GcPtr::new(var));
        assert!(matches!(
            serialize(&v),
            Err(CloneError::NotShareable { type_name: "var" })
        ));
    }

    #[test]
    fn unbound_var_crosses_as_unbound() {
        use crate::types::Var;
        let v = Value::Var(GcPtr::new(Var::new("user", "later")));
        let Value::Var(p) = roundtrip(&v) else {
            panic!("expected Var")
        };
        assert!(!p.get().is_bound());
    }

    #[test]
    fn byte_size_scalar_is_node_sized() {
        // A scalar has no owned payload, so it costs exactly one node.
        let node = std::mem::size_of::<SerializedValue>();
        assert_eq!(serialize(&Value::Long(7)).unwrap().byte_size(), node);
        assert_eq!(serialize(&Value::Nil).unwrap().byte_size(), node);
    }
}
