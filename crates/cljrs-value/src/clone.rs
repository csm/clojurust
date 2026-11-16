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
//! - Primitive and object arrays (snapshot of current contents).  An object
//!   array's elements are copied out under its lock and serialized after the
//!   lock is released, so a lazy element may read or write its own array.
//!   A value that contains itself (through an object array, lazy seq, delay
//!   or mutable record field) is rejected with [`CloneError::Cyclic`]; a
//!   repeated acyclic reference is copied once per occurrence, so aliasing
//!   is not preserved across the boundary.
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

use crate::collections::{
    PersistentHashSet, PersistentVector, Snapshot, SortedMap, SortedSet, ValueSnapshot,
};
use crate::error::ValueError;
use crate::shared::{ByteBlob, SharedAtom, SharedValue};
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
    /// Realizing a lazy value was interrupted by fiber cancellation; callers
    /// must surface it as `ValueError::Cancelled`, which stays uncatchable.
    Cancelled,
    /// Realizing a lazy value exhausted the gas budget; callers surface it
    /// as the uncatchable `ValueError::GasExhausted`.
    GasExhausted,
    /// The process memory governor refused the message's queue charge
    /// (strict mode).
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
    /// The value contains itself (through an object array, lazy seq, delay
    /// or mutable record field), so it has no finite copy.
    Cyclic {
        /// Clojure type name of the node that was re-entered.
        type_name: &'static str,
    },
    /// A byte-bounded channel has no room for the message.
    Full {
        /// Wire size of the refused message.
        bytes: usize,
        /// Bytes queued on the channel when the message was refused.
        queued: usize,
        /// The channel's byte limit.
        limit: usize,
    },
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
            CloneError::Cyclic { type_name } => write!(
                f,
                "value is cyclic (it contains itself through a `{type_name}`) and \
                 cannot cross an isolate boundary"
            ),
            CloneError::Disconnected => write!(f, "isolate channel is disconnected"),
            CloneError::Cancelled => write!(f, "execution cancelled"),
            CloneError::GasExhausted => write!(f, "gas exhausted"),
            CloneError::MemoryLimitExceeded(e) => e.fmt(f),
            CloneError::Full {
                bytes,
                queued,
                limit,
            } => write!(
                f,
                "isolate channel is full: a {bytes}-byte message does not fit \
                 ({queued} of {limit} bytes queued)"
            ),
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
    ByteBlob(Arc<ByteBlob>),
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

impl SerializedValue {
    /// Bytes this serialized message owns while it is queued: the root node
    /// plus every owned buffer (child vectors, strings, array contents).
    ///
    /// This is the message-queue charge.  It differs from [`Self::byte_size`],
    /// which estimates the receiver's materialized heap.  `Arc`-shared payloads
    /// (`SharedAtom`, `ByteBlob`, a var's root cell) count as zero: their
    /// owner objects already hold the applicable governor charge.
    pub fn wire_size(&self) -> usize {
        std::mem::size_of::<SerializedValue>() + self.owned_bytes()
    }

    /// Heap bytes owned below this node (excluding the node itself).
    fn owned_bytes(&self) -> usize {
        const NODE: usize = std::mem::size_of::<SerializedValue>();
        fn items(items: &[SerializedValue]) -> usize {
            items.iter().map(|v| NODE + v.owned_bytes()).sum()
        }
        fn pairs(pairs: &[(SerializedValue, SerializedValue)]) -> usize {
            pairs
                .iter()
                .map(|(k, v)| 2 * NODE + k.owned_bytes() + v.owned_bytes())
                .sum()
        }
        fn ident(namespace: &Option<Arc<str>>, name: &Arc<str>) -> usize {
            namespace.as_ref().map_or(0, |n| n.len()) + name.len()
        }
        match self {
            SerializedValue::Nil
            | SerializedValue::Bool(_)
            | SerializedValue::Long(_)
            | SerializedValue::Double(_)
            | SerializedValue::Char(_)
            | SerializedValue::Uuid(_) => 0,

            SerializedValue::BigInt(b) => b.bits().div_ceil(8) as usize,
            SerializedValue::Ratio(r) => {
                (r.numer().bits().div_ceil(8) + r.denom().bits().div_ceil(8)) as usize
            }
            // log2(10) / 8 ≈ 27 / 64 bytes per decimal digit.
            SerializedValue::BigDecimal(d) => (d.digits() as usize * 27).div_ceil(64),

            SerializedValue::Str(s) | SerializedValue::Pattern(s) => s.capacity(),

            SerializedValue::Symbol {
                namespace,
                name,
                version,
            } => ident(namespace, name) + version.as_ref().map_or(0, |v| v.len()),
            SerializedValue::Keyword { namespace, name } => ident(namespace, name),

            SerializedValue::List(v)
            | SerializedValue::Vector(v)
            | SerializedValue::HashSet(v)
            | SerializedValue::SortedSet(v)
            | SerializedValue::Queue(v)
            | SerializedValue::ObjectArray(v) => items(v),

            SerializedValue::ArrayMap(p)
            | SerializedValue::HashMap(p)
            | SerializedValue::SortedMap(p) => pairs(p),

            SerializedValue::TypeInstance { type_tag, fields } => type_tag.len() + pairs(fields),

            SerializedValue::Cons { head, tail } => {
                2 * NODE + head.owned_bytes() + tail.owned_bytes()
            }

            SerializedValue::Error(e) => e.wire_size(),

            SerializedValue::BooleanArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::ByteArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::ShortArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::IntArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::LongArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::FloatArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::DoubleArray(v) => std::mem::size_of_val(v.as_slice()),
            SerializedValue::CharArray(v) => std::mem::size_of_val(v.as_slice()),

            SerializedValue::WithMeta { value, meta } => {
                2 * NODE + value.owned_bytes() + meta.owned_bytes()
            }
            SerializedValue::Reduced(inner) => NODE + inner.owned_bytes(),

            SerializedValue::SharedAtom(_) | SerializedValue::ByteBlob(_) => 0,
            SerializedValue::Var {
                namespace, name, ..
            } => namespace.len() + name.len(),
        }
    }
}

impl SerializedError {
    /// Boxed size of this error plus everything it owns.
    fn wire_size(&self) -> usize {
        const NODE: usize = std::mem::size_of::<SerializedValue>();
        std::mem::size_of::<SerializedError>()
            + self.message.capacity()
            + self.data.as_ref().map_or(0, |pairs| {
                pairs
                    .iter()
                    .map(|(k, v)| 2 * NODE + k.owned_bytes() + v.owned_bytes())
                    .sum()
            })
            + self.cause.as_ref().map_or(0, |c| c.wire_size())
            + self.kind.owned_bytes()
    }

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
    GasExhausted,
    Cancelled,
    MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded),
    OutOfRange,
    TransientAlreadyPersisted,
    Parse,
    Thrown(Box<SerializedValue>),
}

impl SerializedErrorKind {
    /// Heap bytes this kind owns (strings and a thrown value).
    fn owned_bytes(&self) -> usize {
        match self {
            SerializedErrorKind::WrongType { got, .. } => got.capacity(),
            SerializedErrorKind::ArityError { name, expected, .. } => {
                name.capacity() + expected.capacity()
            }
            SerializedErrorKind::NotCallable { value } => value.capacity(),
            SerializedErrorKind::Other(msg) => msg.capacity(),
            SerializedErrorKind::Thrown(v) => v.wire_size(),
            SerializedErrorKind::IndexOutOfBounds { .. }
            | SerializedErrorKind::OddMap { .. }
            | SerializedErrorKind::Unsupported
            | SerializedErrorKind::Cancelled
            | SerializedErrorKind::GasExhausted
            | SerializedErrorKind::MemoryLimitExceeded(_)
            | SerializedErrorKind::OutOfRange
            | SerializedErrorKind::TransientAlreadyPersisted
            | SerializedErrorKind::Parse => 0,
        }
    }
}

// ── serialize ─────────────────────────────────────────────────────────────────

/// The `ValueError` a caller surfaces for a resource-limit `CloneError`
/// (cancellation, gas, memory), so it stays uncatchable; `None` for ordinary
/// boundary errors.
pub fn resource_value_error(error: &CloneError) -> Option<ValueError> {
    match error {
        CloneError::Cancelled => Some(ValueError::Cancelled),
        CloneError::GasExhausted => Some(ValueError::GasExhausted),
        CloneError::MemoryLimitExceeded(fault) => Some(ValueError::MemoryLimitExceeded(*fault)),
        _ => None,
    }
}

/// Serialize a `Value` into a `Send + Sync` wire form suitable for crossing an
/// isolate boundary. Returns [`CloneError`] for non-shareable values.
pub fn serialize(v: &Value) -> Result<SerializedValue, CloneError> {
    serialize_value(&mut Traversal::default(), v)
}

/// Per-call traversal state for [`serialize`].
///
/// `active` holds the identities of the mutable or lazy nodes (object
/// arrays, lazy seqs, delays, records with mutable fields) on the path from
/// the root to the value being serialized.  Persistent collections are
/// immutable and cannot contain themselves, so any cycle passes through one
/// of these nodes: re-entering one that is still active is rejected with
/// [`CloneError::Cyclic`] instead of recursing without bound.  A node reached
/// again along a *different* path (a repeated, acyclic reference) is not a
/// cycle: each occurrence is copied, so aliasing is not preserved.
///
/// The state is threaded through the recursion rather than kept in a
/// thread-local, so a fiber that parks inside a lazy-seq realization never
/// sees another fiber's traversal.
#[derive(Default)]
struct Traversal {
    active: std::collections::HashSet<usize>,
}

impl Traversal {
    /// Run `f` with the node at `addr` on the active path, rejecting
    /// re-entry of a node that is already on it.
    fn within<T>(
        &mut self,
        addr: usize,
        type_name: &'static str,
        f: impl FnOnce(&mut Self) -> Result<T, CloneError>,
    ) -> Result<T, CloneError> {
        if !self.active.insert(addr) {
            return Err(CloneError::Cyclic { type_name });
        }
        let result = f(self);
        self.active.remove(&addr);
        result
    }
}

fn serialize_value(cx: &mut Traversal, v: &Value) -> Result<SerializedValue, CloneError> {
    match v {
        // ── Wrappers ──
        Value::WithMeta(inner, meta) => Ok(SerializedValue::WithMeta {
            value: Box::new(serialize_value(cx, inner)?),
            meta: Box::new(serialize_value(cx, meta)?),
        }),
        Value::Reduced(inner) => Ok(SerializedValue::Reduced(Box::new(serialize_value(
            cx, inner,
        )?))),

        // ── Scalars ──
        Value::Nil => Ok(SerializedValue::Nil),
        Value::Bool(b) => Ok(SerializedValue::Bool(*b)),
        Value::Long(n) => Ok(SerializedValue::Long(*n)),
        Value::Double(d) => Ok(SerializedValue::Double(*d)),
        Value::Char(c) => Ok(SerializedValue::Char(*c)),
        Value::Uuid(u) => Ok(SerializedValue::Uuid(u.get().0)),

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
            let items: Result<Vec<_>, _> = p.get().iter().map(|x| serialize_value(cx, x)).collect();
            Ok(SerializedValue::List(items?))
        }
        Value::Vector(p) => {
            let items: Result<Vec<_>, _> = p.get().iter().map(|x| serialize_value(cx, x)).collect();
            Ok(SerializedValue::Vector(items?))
        }
        Value::Map(m) => serialize_map(cx, m),
        Value::Set(s) => serialize_set(cx, s),
        Value::Queue(p) => {
            let items: Result<Vec<_>, _> = p.get().iter().map(|x| serialize_value(cx, x)).collect();
            Ok(SerializedValue::Queue(items?))
        }
        Value::Cons(p) => {
            let c = p.get();
            Ok(SerializedValue::Cons {
                head: Box::new(serialize_value(cx, &c.head)?),
                tail: Box::new(serialize_value(cx, &c.tail)?),
            })
        }

        // ── Records ──
        Value::TypeInstance(p) => {
            let ti = p.get();
            // Fold any mutable-field slots into the serialized field map: the
            // snapshot preserves their VALUES. Mutability itself is runtime
            // state and is not restored (deserialize sets `mutable: None`).
            let fields = match ti.mutable.as_ref().map(|a| a.get().deref()) {
                Some(Value::Map(mm)) => {
                    let mut merged = ti.fields.clone();
                    for (k, v) in mm.iter() {
                        merged = merged.assoc(k.clone(), v.clone());
                    }
                    // A mutable slot can be reset while a lazy field value is
                    // realized, so root the merged copy; and a slot can hold
                    // the record itself, so track it on the active path.
                    let merged = Snapshot::new(Value::Map(merged));
                    cx.within(ti as *const _ as usize, "record", |cx| match merged.get() {
                        Value::Map(m) => serialize_map_pairs(cx, m),
                        _ => unreachable!("merged record fields are a map"),
                    })?
                }
                _ => serialize_map_pairs(cx, &ti.fields)?,
            };
            Ok(SerializedValue::TypeInstance {
                type_tag: ti.type_tag.clone(),
                fields,
            })
        }

        // ── Errors ──
        Value::Error(p) => Ok(SerializedValue::Error(Box::new(serialize_error(
            cx,
            p.get(),
        )?))),

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
            let addr = p.get() as *const _ as usize;
            cx.within(addr, "object-array", |cx| {
                // Copy the elements out under the lock and release it before
                // recursing: serializing an element can realize a lazy seq,
                // whose thunk may read or write this same array (and a
                // self-containing array would re-lock it directly).  The
                // copy is a traced snapshot root, so elements stay alive even
                // if the thunk overwrites their slots and a collection runs.
                // The wire form is the contents at the moment of the copy.
                let elements = p.get().0.lock().unwrap().clone();
                let snapshot = Snapshot::new(ValueSnapshot(elements));
                let items: Result<Vec<_>, _> = snapshot
                    .get()
                    .0
                    .iter()
                    .map(|x| serialize_value(cx, x))
                    .collect();
                Ok(SerializedValue::ObjectArray(items?))
            })
        }

        // ── Lazy sequences: realize first ──
        Value::LazySeq(p) => cx.within(p.get() as *const _ as usize, "lazy-seq", |cx| {
            let realized = p.get().try_realize().map_err(|error| match error {
                // Resource errors and cancellation keep their identity, so they
                // are not turned into a catchable "not shareable" error.
                ValueError::Cancelled => CloneError::Cancelled,
                ValueError::GasExhausted => CloneError::GasExhausted,
                ValueError::MemoryLimitExceeded(fault) => CloneError::MemoryLimitExceeded(fault),
                _ => not_shareable("lazy seq whose realization failed"),
            })?;
            serialize_value(cx, &realized)
        }),

        // ── Delay: force if already realized, else error ──
        Value::Delay(p) => {
            // Copy the forced value out and release the state lock before
            // recursing, for the same re-entry reason as object arrays.  A
            // forced value never changes, so the delay keeps it alive.
            let forced = match &*p.get().state.lock().unwrap() {
                DelayState::Forced(v) => Some(v.clone()),
                _ => None,
            };
            match forced {
                Some(v) => cx.within(p.get() as *const _ as usize, "delay", |cx| {
                    serialize_value(cx, &v)
                }),
                None => Err(not_shareable("delay")),
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

fn serialize_map(cx: &mut Traversal, m: &MapValue) -> Result<SerializedValue, CloneError> {
    let pairs = serialize_map_pairs(cx, m)?;
    Ok(match m {
        MapValue::Array(_) => SerializedValue::ArrayMap(pairs),
        MapValue::Hash(_) => SerializedValue::HashMap(pairs),
        MapValue::Sorted(_) => SerializedValue::SortedMap(pairs),
    })
}

fn serialize_map_pairs(
    cx: &mut Traversal,
    m: &MapValue,
) -> Result<Vec<(SerializedValue, SerializedValue)>, CloneError> {
    let mut pairs = Vec::with_capacity(m.count());
    let mut err: Option<CloneError> = None;
    m.for_each(|k, v| {
        if err.is_some() {
            return;
        }
        match (serialize_value(cx, k), serialize_value(cx, v)) {
            (Ok(sk), Ok(sv)) => pairs.push((sk, sv)),
            (Err(e), _) | (_, Err(e)) => err = Some(e),
        }
    });
    if let Some(e) = err { Err(e) } else { Ok(pairs) }
}

fn serialize_set(cx: &mut Traversal, s: &SetValue) -> Result<SerializedValue, CloneError> {
    let items: Result<Vec<_>, _> = s.iter().map(|x| serialize_value(cx, x)).collect();
    Ok(match s {
        SetValue::Hash(_) => SerializedValue::HashSet(items?),
        SetValue::Sorted(_) => SerializedValue::SortedSet(items?),
    })
}

fn serialize_error(
    cx: &mut Traversal,
    e: &crate::error::ExceptionInfo,
) -> Result<SerializedError, CloneError> {
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
        ValueError::GasExhausted => SerializedErrorKind::GasExhausted,
        ValueError::Cancelled => SerializedErrorKind::Cancelled,
        ValueError::MemoryLimitExceeded(e) => SerializedErrorKind::MemoryLimitExceeded(*e),
        ValueError::OutOfRange => SerializedErrorKind::OutOfRange,
        ValueError::TransientAlreadyPersisted => SerializedErrorKind::TransientAlreadyPersisted,
        ValueError::Parse => SerializedErrorKind::Parse,
        ValueError::Thrown(v) => SerializedErrorKind::Thrown(Box::new(serialize_value(cx, v)?)),
    };

    let data = e
        .data
        .as_ref()
        .map(|m| serialize_map_pairs(cx, m))
        .transpose()?;

    let cause = e
        .cause
        .as_ref()
        .map(|c| serialize_error(cx, c.get()).map(Box::new))
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
        SerializedValue::WithMeta { value, meta } => Value::WithMeta(
            std::sync::Arc::new(deserialize(*value)),
            std::sync::Arc::new(deserialize(*meta)),
        ),
        SerializedValue::Reduced(inner) => Value::Reduced(std::sync::Arc::new(deserialize(*inner))),

        SerializedValue::Nil => Value::Nil,
        SerializedValue::Bool(b) => Value::Bool(b),
        SerializedValue::Long(n) => Value::Long(n),
        SerializedValue::Double(d) => Value::Double(d),
        SerializedValue::Char(c) => Value::Char(c),
        SerializedValue::Uuid(u) => Value::uuid(u),

        SerializedValue::BigInt(n) => Value::BigInt(GcPtr::new(n)),
        SerializedValue::BigDecimal(d) => Value::BigDecimal(GcPtr::new(d)),
        SerializedValue::Ratio(r) => Value::Ratio(GcPtr::new(r)),
        SerializedValue::Str(s) => Value::Str(GcPtr::new(s)),
        SerializedValue::Pattern(src) => Value::Pattern(GcPtr::new(
            crate::regex::Pattern::new(&src).expect("pattern was valid at serialize time"),
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
                mutable: None,
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
        SerializedErrorKind::GasExhausted => ValueError::GasExhausted,
        SerializedErrorKind::Cancelled => ValueError::Cancelled,
        SerializedErrorKind::MemoryLimitExceeded(e) => ValueError::MemoryLimitExceeded(e),
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
        assert_eq!(roundtrip(&Value::uuid(12345)), Value::uuid(12345));
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
            fn force(&self) -> crate::ValueResult<Value> {
                Ok(Value::Long(99))
            }
        }
        let ls = LazySeq::new(Box::new(DoneThunk));
        let _ = ls.realize(); // force it
        let v = Value::LazySeq(GcPtr::new(ls));
        assert_eq!(roundtrip(&v), Value::Long(99));
    }

    fn object_array(items: Vec<Value>) -> GcPtr<crate::value::ObjectArray> {
        GcPtr::new(crate::value::ObjectArray::new(items))
    }

    fn array_items(sv: &SerializedValue) -> &[SerializedValue] {
        match sv {
            SerializedValue::ObjectArray(items) => items,
            other => panic!("expected an object array, got {other:?}"),
        }
    }

    /// A lazy element whose thunk reads and writes the array being
    /// serialized must not find the array locked (issue #517).  `try_lock`
    /// keeps a regression from hanging the test suite.
    #[test]
    fn object_array_lazy_element_can_reenter_its_array() {
        use crate::types::{LazySeq, Thunk};
        #[derive(Debug)]
        struct ReenterThunk(GcPtr<crate::value::ObjectArray>);
        impl cljrs_gc::Trace for ReenterThunk {
            fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
                cljrs_gc::GcVisitor::visit(visitor, &self.0);
            }
        }
        impl Thunk for ReenterThunk {
            fn force(&self) -> crate::ValueResult<Value> {
                let mut items = self
                    .0
                    .get()
                    .0
                    .try_lock()
                    .expect("array lock must not be held while realizing an element");
                let read = items[1].clone();
                items[1] = Value::Long(-1);
                Ok(read)
            }
        }
        let arr = object_array(vec![Value::Nil, Value::Long(2), Value::Long(3)]);
        let lazy = Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(ReenterThunk(
            arr.clone(),
        )))));
        arr.get().0.lock().unwrap()[0] = lazy;

        let sv = serialize(&Value::ObjectArray(arr.clone())).expect("serialize");
        // The wire form is the snapshot taken before realization; the thunk's
        // write lands in the source array only.
        assert!(matches!(
            array_items(&sv),
            [
                SerializedValue::Long(2),
                SerializedValue::Long(2),
                SerializedValue::Long(3)
            ]
        ));
        assert_eq!(arr.get().0.lock().unwrap()[1], Value::Long(-1));
    }

    #[test]
    fn object_array_direct_cycle_is_rejected() {
        let arr = object_array(vec![Value::Long(1), Value::Nil]);
        arr.get().0.lock().unwrap()[1] = Value::ObjectArray(arr.clone());
        assert!(matches!(
            serialize(&Value::ObjectArray(arr.clone())),
            Err(CloneError::Cyclic {
                type_name: "object-array"
            })
        ));
        // Break the cycle so the test does not leak under refcounting.
        arr.get().0.lock().unwrap()[1] = Value::Nil;
    }

    #[test]
    fn object_array_indirect_cycle_is_rejected() {
        let a = object_array(vec![Value::Nil]);
        let b = object_array(vec![Value::Nil]);
        let through_vector = Value::Vector(GcPtr::new(PersistentVector::from_iter([
            Value::ObjectArray(b.clone()),
        ])));
        a.get().0.lock().unwrap()[0] = through_vector;
        b.get().0.lock().unwrap()[0] = Value::ObjectArray(a.clone());
        assert!(matches!(
            serialize(&Value::ObjectArray(a.clone())),
            Err(CloneError::Cyclic { .. })
        ));
        b.get().0.lock().unwrap()[0] = Value::Nil;
    }

    /// The same child reached twice, without a cycle, is copied twice.
    #[test]
    fn object_array_repeated_child_is_not_a_cycle() {
        let child = object_array(vec![Value::Long(7)]);
        let parent = object_array(vec![
            Value::ObjectArray(child.clone()),
            Value::ObjectArray(child.clone()),
        ]);
        let sv = serialize(&Value::ObjectArray(parent)).expect("repeated child serializes");
        let items = array_items(&sv);
        assert_eq!(items.len(), 2);
        for item in items {
            assert!(matches!(array_items(item), [SerializedValue::Long(7)]));
        }
        let Value::ObjectArray(copy) = deserialize(sv) else {
            panic!("expected an object array");
        };
        let copy = copy.get().0.lock().unwrap();
        let (Value::ObjectArray(first), Value::ObjectArray(second)) = (&copy[0], &copy[1]) else {
            panic!("expected nested object arrays");
        };
        // Aliasing is not preserved: each occurrence is its own copy.
        assert!(!GcPtr::ptr_eq(first, second));
        assert!(!GcPtr::ptr_eq(first, &child));
    }

    #[test]
    fn nested_acyclic_object_arrays_roundtrip() {
        let inner = object_array(vec![Value::Long(1), Value::string("x")]);
        let outer = object_array(vec![Value::ObjectArray(inner), Value::Long(2)]);
        let Value::ObjectArray(copy) = roundtrip(&Value::ObjectArray(outer)) else {
            panic!("expected an object array");
        };
        let copy = copy.get().0.lock().unwrap();
        assert_eq!(copy[1], Value::Long(2));
        let Value::ObjectArray(inner) = &copy[0] else {
            panic!("expected a nested object array");
        };
        assert_eq!(
            *inner.get().0.lock().unwrap(),
            vec![Value::Long(1), Value::string("x")]
        );
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
        let v = Value::Reduced(std::sync::Arc::new(Value::Long(55)));
        assert_eq!(
            roundtrip(&v),
            Value::Reduced(std::sync::Arc::new(Value::Long(55)))
        );
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
    fn wire_size_counts_owned_buffers() {
        const NODE: usize = std::mem::size_of::<SerializedValue>();
        assert_eq!(serialize(&Value::Long(1)).unwrap().wire_size(), NODE);
        let s = serialize(&Value::string("x".repeat(100))).unwrap();
        assert!(s.wire_size() >= NODE + 100);
        let v = serialize(&Value::Vector(GcPtr::new(PersistentVector::from_iter([
            Value::Long(1),
            Value::Long(2),
        ]))))
        .unwrap();
        assert_eq!(v.wire_size(), 3 * NODE, "root plus two child slots");
    }

    #[test]
    fn wire_size_counts_error_kind_payloads() {
        let big = "x".repeat(1 << 20);
        let thrown = SerializedValue::Error(Box::new(SerializedError {
            kind: SerializedErrorKind::Thrown(Box::new(SerializedValue::Str(big.clone()))),
            message: String::new(),
            data: None,
            cause: None,
        }));
        assert!(thrown.wire_size() >= 1 << 20);
        let other = SerializedValue::Error(Box::new(SerializedError {
            kind: SerializedErrorKind::Other(big),
            message: String::new(),
            data: None,
            cause: None,
        }));
        assert!(other.wire_size() >= 1 << 20);
    }

    #[test]
    fn wire_size_excludes_shared_payloads() {
        const NODE: usize = std::mem::size_of::<SerializedValue>();
        let blob = Value::ByteBlob(Arc::new(ByteBlob::new(vec![0u8; 1 << 16])));
        assert_eq!(serialize(&blob).unwrap().wire_size(), NODE);
    }

    #[test]
    fn byte_size_scalar_is_node_sized() {
        // A scalar has no owned payload, so it costs exactly one node.
        let node = std::mem::size_of::<SerializedValue>();
        assert_eq!(serialize(&Value::Long(7)).unwrap().byte_size(), node);
        assert_eq!(serialize(&Value::Nil).unwrap().byte_size(), node);
    }
}
