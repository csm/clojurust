use std::cmp::Ordering;
use std::fmt;
use std::sync::{Arc, Mutex};

use crate::collections::{
    PersistentArrayMap, PersistentHashMap, PersistentHashSet, PersistentList, PersistentQueue,
    PersistentVector, SortedMap, SortedSet, TransientMap, TransientSet, TransientVector,
};
use crate::error::ExceptionInfo;
use crate::hash::{
    ClojureHash, hash_combine_ordered, hash_combine_unordered, hash_i64, hash_string, hash_u128,
};
use crate::keyword::Keyword;
use crate::regex::Matcher;
use crate::resource::ResourceHandle;
use crate::symbol::Symbol;
use crate::types::{
    Agent, Atom, BoundFn, CljxCons, CljxFn, CljxFuture, CljxPromise, Delay, LazySeq, MultiFn,
    Namespace, NativeFn, Protocol, ProtocolFn, Var, Volatile,
};
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use num_bigint::BigInt;
use num_traits::ToPrimitive;
use regex::Regex;

/// A GC-traced mutable array of Values (backs `object-array`).
#[derive(Debug)]
pub struct ObjectArray(pub Mutex<Vec<Value>>);

impl ObjectArray {
    pub fn new(v: Vec<Value>) -> Self {
        Self(Mutex::new(v))
    }
}

impl Trace for ObjectArray {
    fn trace(&self, visitor: &mut MarkVisitor) {
        {
            let guard = self.0.lock().unwrap();
            for v in guard.iter() {
                v.trace(visitor);
            }
        }
    }
}

/// The central runtime type: every Clojure value is a `Value`.
///
/// Small scalars (`Nil`, `Bool`, `Long`, `Double`, `Char`) are stored inline.
/// All heap-allocated types go behind `GcPtr` so that `clone()` is O(1).
#[derive(Clone, Debug)]
pub enum Value {
    // ── Scalars ───────────────────────────────────────────────────────────────
    Nil,
    Bool(bool),
    Long(i64),
    Double(f64),
    BigInt(GcPtr<BigInt>),
    BigDecimal(GcPtr<bigdecimal::BigDecimal>),
    Ratio(GcPtr<num_rational::Ratio<BigInt>>),
    Char(char),
    Str(GcPtr<String>),
    Uuid(u128),
    Pattern(GcPtr<Regex>),
    Matcher(GcPtr<Matcher>),

    // ── Identifiers ───────────────────────────────────────────────────────────
    Symbol(GcPtr<Symbol>),
    Keyword(GcPtr<Keyword>),

    // ── Collections ───────────────────────────────────────────────────────────
    List(GcPtr<PersistentList>),
    Vector(GcPtr<PersistentVector>),
    /// Small maps (≤8 entries) are stored as an ArrayMap; larger ones as a HashMap.
    /// Also contains sorted map.
    Map(MapValue),
    Set(SetValue),
    Queue(GcPtr<PersistentQueue>),

    // Transients
    TransientMap(GcPtr<TransientMap>),
    TransientSet(GcPtr<TransientSet>),
    TransientVector(GcPtr<TransientVector>),

    // Arrays
    IntArray(GcPtr<Mutex<Vec<i32>>>),
    LongArray(GcPtr<Mutex<Vec<i64>>>),
    ShortArray(GcPtr<Mutex<Vec<i16>>>),
    ByteArray(GcPtr<Mutex<Vec<i8>>>),
    FloatArray(GcPtr<Mutex<Vec<f32>>>),
    DoubleArray(GcPtr<Mutex<Vec<f64>>>),
    BooleanArray(GcPtr<Mutex<Vec<bool>>>),
    CharArray(GcPtr<Mutex<Vec<char>>>),
    ObjectArray(GcPtr<ObjectArray>),

    // ── Functions ─────────────────────────────────────────────────────────────
    NativeFunction(GcPtr<NativeFn>),
    Fn(GcPtr<CljxFn>),
    Macro(GcPtr<CljxFn>),
    BoundFn(GcPtr<BoundFn>),

    // ── Mutable state ─────────────────────────────────────────────────────────
    Var(GcPtr<Var>),
    Atom(GcPtr<Atom>),

    // ── Reduced (early termination sentinel for reduce/transduce) ───────────
    Reduced(Box<Value>),

    // ── Other ─────────────────────────────────────────────────────────────────
    Namespace(GcPtr<Namespace>),

    // ── Lazy sequences ────────────────────────────────────────────────────────
    /// A deferred sequence cell — realized at most once.
    LazySeq(GcPtr<LazySeq>),
    /// A realized cons cell whose tail may itself be lazy.
    Cons(GcPtr<CljxCons>),

    // ── Protocols & Multimethods ──────────────────────────────────────────────
    Protocol(GcPtr<Protocol>),
    ProtocolFn(GcPtr<ProtocolFn>),
    MultiFn(GcPtr<MultiFn>),

    // ── Concurrency primitives ────────────────────────────────────────────────
    Volatile(GcPtr<Volatile>),
    Delay(GcPtr<Delay>),
    Promise(GcPtr<CljxPromise>),
    Future(GcPtr<CljxFuture>),
    Agent(GcPtr<Agent>),

    // ── Records / reify instances ─────────────────────────────────────────────
    TypeInstance(GcPtr<TypeInstance>),

    // ── Native Rust objects (GcPtr-managed, for interop) ─────────────────────
    NativeObject(GcPtr<crate::native_object::NativeObjectBox>),

    // ── I/O resources (Arc-ref-counted, NOT GcPtr) ───────────────────────────
    Resource(ResourceHandle),

    // ── Metadata wrapper ─────────────────────────────────────────────────────
    /// A value with attached metadata. Transparent for equality, hashing, display.
    WithMeta(Box<Value>, Box<Value>),

    // Errors
    Error(GcPtr<ExceptionInfo>),
}

/// A map value: either a small array-map or a HAMT-based hash-map.
#[derive(Clone, Debug)]
pub enum MapValue {
    Array(GcPtr<PersistentArrayMap>),
    Hash(GcPtr<PersistentHashMap>),
    Sorted(GcPtr<SortedMap>),
}

impl MapValue {
    pub fn empty() -> Self {
        MapValue::Array(GcPtr::new(PersistentArrayMap::empty()))
    }

    /// Build a map from pre-evaluated key-value pairs.
    ///
    /// Chooses the optimal representation based on size: ArrayMap for small
    /// maps (≤8 entries), HashTrieMap for larger ones. This avoids N
    /// intermediate allocations that `empty() + assoc + assoc + ...` would
    /// create.
    pub fn from_pairs(pairs: Vec<(Value, Value)>) -> Self {
        use crate::collections::array_map::AssocResult;

        // Check for duplicates by building through assoc (last wins).
        match PersistentArrayMap::from_pairs(pairs) {
            AssocResult::Array(m) => MapValue::Array(GcPtr::new(m)),
            AssocResult::Promote(pairs) => {
                MapValue::Hash(GcPtr::new(PersistentHashMap::from_pairs(pairs)))
            }
        }
    }

    /// Build a map from a flat evaluated entries vector `[k0, v0, k1, v1, ...]`.
    ///
    /// Similar to `from_pairs` but takes flat key-value entries. Handles
    /// duplicate keys (last wins via assoc). Avoids intermediate allocations.
    pub fn from_flat_entries(entries: Vec<Value>) -> Self {
        debug_assert!(entries.len().is_multiple_of(2));
        // We need to handle duplicate keys, so build through assoc.
        let pairs: Vec<(Value, Value)> = entries
            .chunks(2)
            .map(|chunk| (chunk[0].clone(), chunk[1].clone()))
            .collect();
        Self::from_pairs(pairs)
    }

    pub fn get(&self, key: &Value) -> Option<Value> {
        match self {
            MapValue::Array(m) => m.get().get(key).cloned(),
            MapValue::Hash(m) => m.get().get(key).cloned(),
            MapValue::Sorted(m) => m.get().get(key).cloned(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            MapValue::Array(m) => m.get().count(),
            MapValue::Hash(m) => m.get().count(),
            MapValue::Sorted(m) => m.get().count(),
        }
    }

    pub fn assoc(&self, k: Value, v: Value) -> Self {
        match self {
            MapValue::Array(m) => match m.get().assoc(k, v) {
                crate::collections::array_map::AssocResult::Array(new_m) => {
                    MapValue::Array(GcPtr::new(new_m))
                }
                crate::collections::array_map::AssocResult::Promote(pairs) => {
                    let hm = PersistentHashMap::from_pairs(pairs);
                    MapValue::Hash(GcPtr::new(hm))
                }
            },
            MapValue::Hash(m) => MapValue::Hash(GcPtr::new(m.get().assoc(k, v))),
            MapValue::Sorted(m) => MapValue::Sorted(GcPtr::new(m.get().assoc(k, v))),
        }
    }

    pub fn dissoc(&self, key: &Value) -> Self {
        match self {
            MapValue::Array(m) => MapValue::Array(GcPtr::new(m.get().dissoc(key))),
            MapValue::Hash(m) => MapValue::Hash(GcPtr::new(m.get().dissoc(key))),
            MapValue::Sorted(m) => MapValue::Sorted(GcPtr::new(m.get().dissoc(key))),
        }
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        match self {
            MapValue::Array(m) => m.get().contains_key(key),
            MapValue::Hash(m) => m.get().contains_key(key),
            MapValue::Sorted(m) => m.get().contains_key(key),
        }
    }

    /// Iterate over all `(key, value)` pairs.
    pub fn for_each<F: FnMut(&Value, &Value)>(&self, mut f: F) {
        match self {
            MapValue::Array(m) => {
                for (k, v) in m.get().iter() {
                    f(k, v);
                }
            }
            MapValue::Hash(m) => {
                for (k, v) in m.get().iter() {
                    f(k, v);
                }
            }
            MapValue::Sorted(m) => {
                for (k, v) in m.get().iter() {
                    f(k, v);
                }
            }
        }
    }

    /// Iterate over key/value pairs.
    pub fn iter(&self) -> Box<dyn Iterator<Item = (&Value, &Value)> + '_> {
        match self {
            MapValue::Array(m) => Box::new(m.get().iter()),
            MapValue::Hash(m) => Box::new(m.get().iter()),
            MapValue::Sorted(m) => Box::new(m.get().iter()),
        }
    }
}

/// A set value, either a hash set or a sorted set.
#[derive(Clone, Debug)]
pub enum SetValue {
    Hash(GcPtr<PersistentHashSet>),
    Sorted(GcPtr<SortedSet>),
}

impl SetValue {
    pub fn empty() -> Self {
        Self::Hash(GcPtr::new(PersistentHashSet::empty()))
    }

    pub fn count(&self) -> usize {
        match self {
            SetValue::Hash(m) => m.get().count(),
            SetValue::Sorted(m) => m.get().count(),
        }
    }

    pub fn is_empty(&self) -> bool {
        match self {
            SetValue::Hash(m) => m.get().is_empty(),
            SetValue::Sorted(m) => m.get().is_empty(),
        }
    }

    pub fn contains(&self, key: &Value) -> bool {
        match self {
            SetValue::Hash(m) => m.get().contains(key),
            SetValue::Sorted(m) => m.get().contains(key),
        }
    }

    pub fn conj(&self, value: Value) -> Self {
        match self {
            SetValue::Hash(m) => SetValue::Hash(GcPtr::new(m.get().conj(value))),
            SetValue::Sorted(m) => SetValue::Sorted(GcPtr::new(m.get().conj(value))),
        }
    }

    pub fn conj_mut(&mut self, value: Value) -> &mut Self {
        match self {
            SetValue::Hash(m) => {
                m.get_mut().conj_mut(value);
            }
            SetValue::Sorted(s) => {
                s.get_mut().conj_mut(value);
            }
        }
        self
    }

    pub fn disj(&self, value: &Value) -> Self {
        match self {
            SetValue::Hash(m) => SetValue::Hash(GcPtr::new(m.get().disj(value))),
            SetValue::Sorted(m) => SetValue::Sorted(GcPtr::new(m.get().disj(value))),
        }
    }

    pub fn iter(&self) -> Box<dyn Iterator<Item = &Value> + '_> {
        match self {
            SetValue::Hash(s) => Box::new(s.get().iter()),
            SetValue::Sorted(s) => Box::new(s.get().iter()),
        }
    }
}

impl cljrs_gc::Trace for SetValue {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
        match self {
            SetValue::Hash(s) => visitor.visit(s),
            SetValue::Sorted(s) => visitor.visit(s),
        }
    }
}

// ── Equality ──────────────────────────────────────────────────────────────────

impl Eq for Value {}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // Strip metadata — it is ignored for equality in Clojure.
        if let Value::WithMeta(inner, _) = self {
            return inner.as_ref() == other;
        }
        if let Value::WithMeta(inner, _) = other {
            return self == inner.as_ref();
        }
        // Unwrap Reduced for equality.
        if let Value::Reduced(inner) = self {
            return inner.as_ref() == other;
        }
        if let Value::Reduced(inner) = other {
            return self == inner.as_ref();
        }
        // Identity shortcut: same GcPtr → equal without realizing.
        // Required for infinite lazy seqs: (let [r (range)] (= r r)) must not hang.
        if let (Value::LazySeq(a), Value::LazySeq(b)) = (self, other)
            && GcPtr::ptr_eq(a, b)
        {
            return true;
        }
        // Realize lazy sequences before comparing.
        // A lazy-seq that realizes to nil is an empty sequence, which is equal
        // to both nil and any empty sequential collection (matching Clojure).
        if let Value::LazySeq(ls) = self {
            let realized = ls.get().realize();
            if realized == Value::Nil && other.is_sequential() {
                return value_to_seq_vec(other).is_empty();
            }
            return realized == *other;
        }
        if let Value::LazySeq(ls) = other {
            let realized = ls.get().realize();
            if realized == Value::Nil && self.is_sequential() {
                return value_to_seq_vec(self).is_empty();
            }
            return *self == realized;
        }
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            // Numeric cross-type equality.
            (Value::Long(a), Value::Long(b)) => a == b,
            (Value::Long(a), Value::BigInt(b)) => BigInt::from(*a) == *b.get(),
            (Value::BigInt(a), Value::Long(b)) => *a.get() == BigInt::from(*b),
            (Value::BigInt(a), Value::BigInt(b)) => a.get() == b.get(),
            (Value::Double(a), Value::Double(b)) => a == b, // NaN != NaN
            (Value::Long(a), Value::Double(b)) => b.fract() == 0.0 && b.to_i64() == Some(*a),
            (Value::Double(a), Value::Long(b)) => a.fract() == 0.0 && a.to_i64() == Some(*b),
            (Value::BigDecimal(a), Value::BigDecimal(b)) => a.get() == b.get(),
            (Value::Ratio(a), Value::Ratio(b)) => a.get() == b.get(),
            (Value::Char(a), Value::Char(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a.get() == b.get(),
            (Value::Symbol(a), Value::Symbol(b)) => a.get() == b.get(),
            (Value::Keyword(a), Value::Keyword(b)) => a.get() == b.get(),
            // Collection equality.
            (Value::List(a), Value::List(b)) => a.get() == b.get(),
            (Value::Vector(a), Value::Vector(b)) => a.get() == b.get(),
            (Value::Set(a), Value::Set(b)) => sets_equal(a, b),
            (Value::Queue(a), Value::Queue(b)) => a.get() == b.get(),
            (Value::Map(a), Value::Map(b)) => maps_equal(a, b),
            // Sequential cross-type equality: '(1 2) == [1 2].
            (Value::List(_), Value::Vector(_)) | (Value::Vector(_), Value::List(_)) => {
                seq_equal(self, other)
            }
            // Cons cells: compare element by element.
            (Value::Cons(_), _) | (_, Value::Cons(_)) => seq_equal(self, other),
            // Pointer equality for functions.
            (Value::Fn(a), Value::Fn(b)) => std::ptr::eq(a.get() as *const _, b.get() as *const _),
            (Value::Macro(a), Value::Macro(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::NativeFunction(a), Value::NativeFunction(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            // Pointer equality for protocol/multimethod objects.
            (Value::Protocol(a), Value::Protocol(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::ProtocolFn(a), Value::ProtocolFn(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::MultiFn(a), Value::MultiFn(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            // Pointer equality for concurrency primitives.
            (Value::Volatile(a), Value::Volatile(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Delay(a), Value::Delay(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Promise(a), Value::Promise(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Future(a), Value::Future(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Agent(a), Value::Agent(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Atom(a), Value::Atom(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Var(a), Value::Var(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            (Value::Namespace(a), Value::Namespace(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            // UUID equality: same u128 value.
            (Value::Uuid(a), Value::Uuid(b)) => a == b,
            // Regex pattern equality: compare source string (matches Clojure JVM behavior
            // where two patterns are equal iff their source strings are equal).
            (Value::Pattern(a), Value::Pattern(b)) => a.get().as_str() == b.get().as_str(),
            // NativeObject equality: pointer identity.
            (Value::NativeObject(a), Value::NativeObject(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            // Resource equality: pointer identity.
            (Value::Resource(a), Value::Resource(b)) => Arc::ptr_eq(&a.0, &b.0),
            // Record equality: same tag and same fields.
            (Value::TypeInstance(a), Value::TypeInstance(b)) => {
                a.get().type_tag == b.get().type_tag && maps_equal(&a.get().fields, &b.get().fields)
            }
            (Value::Error(a), Value::Error(b)) => {
                std::ptr::eq(a.get() as *const _, b.get() as *const _)
            }
            _ => false,
        }
    }
}

fn maps_equal(a: &MapValue, b: &MapValue) -> bool {
    if a.count() != b.count() {
        return false;
    }
    let mut equal = true;
    a.for_each(|k, v| {
        if equal {
            match b.get(k) {
                Some(bv) if &bv == v => {}
                _ => equal = false,
            }
        }
    });
    equal
}

fn sets_equal(a: &SetValue, b: &SetValue) -> bool {
    if a.count() != b.count() {
        return false;
    }
    for k in a.iter() {
        if !b.contains(k) {
            return false;
        }
    }
    true
}

fn seq_equal(a: &Value, b: &Value) -> bool {
    let a_items = value_to_seq_vec(a);
    let b_items = value_to_seq_vec(b);
    a_items.len() == b_items.len() && a_items.iter().zip(b_items.iter()).all(|(x, y)| x == y)
}

fn value_to_seq_vec(v: &Value) -> Vec<Value> {
    // Iteratively unwrap lazy seqs.
    let mut v = v.clone();
    while let Value::LazySeq(ls) = &v {
        v = ls.get().realize();
    }
    match &v {
        Value::List(l) => l.get().iter().cloned().collect(),
        Value::Vector(v) => v.get().iter().cloned().collect(),
        Value::LazySeq(_) => unreachable!("unwrapped above"),
        Value::Cons(c) => {
            let mut result = vec![c.get().head.clone()];
            let mut tail = c.get().tail.clone();
            loop {
                match tail {
                    Value::Nil => break,
                    Value::List(l) => {
                        result.extend(l.get().iter().cloned());
                        break;
                    }
                    Value::Cons(next_c) => {
                        result.push(next_c.get().head.clone());
                        tail = next_c.get().tail.clone();
                    }
                    Value::LazySeq(ls) => {
                        tail = ls.get().realize();
                    }
                    _ => break,
                }
            }
            result
        }
        _ => vec![],
    }
}

// ── Hashing ───────────────────────────────────────────────────────────────────

impl ClojureHash for Value {
    fn clojure_hash(&self) -> u32 {
        match self {
            Value::WithMeta(inner, _) => inner.clojure_hash(),
            Value::Reduced(inner) => inner.clojure_hash(),
            Value::Nil => 0,
            Value::Bool(b) => {
                if *b { 1231 } else { 1237 } // Java Boolean.hashCode
            }
            Value::Long(n) => hash_i64(*n),
            Value::Double(f) => {
                // Whole-number doubles hash like their Long equivalent.
                if f.fract() == 0.0
                    && f.is_finite()
                    && let Some(n) = num_traits::ToPrimitive::to_i64(f)
                {
                    return hash_i64(n);
                }
                hash_i64(f.to_bits() as i64)
            }
            Value::BigInt(n) => {
                // Hash like Long if it fits.
                if let Some(l) = n.get().to_i64() {
                    return hash_i64(l);
                }
                // Otherwise hash the decimal string (simplified).
                hash_string(&n.get().to_string())
            }
            Value::Char(c) => *c as u32,
            Value::Str(s) => hash_string(s.get()),
            Value::Pattern(r) => hash_string(r.get().as_str()),
            Value::Matcher(m) => hash_string(m.get().pattern.get().as_str()),
            Value::Keyword(k) => hash_string(&k.get().to_string()),
            Value::Symbol(s) => hash_string(&s.get().to_string()),
            Value::Uuid(u) => hash_u128(*u),
            Value::NativeObject(obj) => {
                let ptr = obj.get() as *const _ as usize;
                hash_i64(ptr as i64)
            }
            Value::Resource(r) => {
                let ptr = Arc::as_ptr(&r.0) as *const () as usize;
                hash_i64(ptr as i64)
            }
            Value::List(l) => {
                let mut h: u32 = 1;
                for v in l.get().iter() {
                    h = hash_combine_ordered(h, v.clojure_hash());
                }
                h
            }
            Value::Vector(v) => {
                let mut h: u32 = 1;
                for item in v.get().iter() {
                    h = hash_combine_ordered(h, item.clojure_hash());
                }
                h
            }
            Value::Map(m) => {
                let mut h: u32 = 0;
                m.for_each(|k, v| {
                    h = hash_combine_unordered(
                        h,
                        hash_combine_ordered(k.clojure_hash(), v.clojure_hash()),
                    );
                });
                h
            }
            Value::Set(s) => {
                let mut h: u32 = 0;
                for k in s.iter() {
                    h = hash_combine_unordered(h, k.clojure_hash());
                }
                h
            }
            Value::TransientMap(m) => m.get().clojure_hash(),
            Value::TransientSet(s) => s.get().clojure_hash(),
            Value::TransientVector(v) => v.get().clojure_hash(),

            // Arrays
            Value::BooleanArray(a) => {
                let mut h: u32 = 0;
                for b in a.get().lock().unwrap().iter() {
                    h = hash_combine_ordered(h, if *b { 1231 } else { 1237 })
                }
                h
            }
            Value::ByteArray(a) => {
                let mut h: u32 = 0;
                for b in a.get().lock().unwrap().iter() {
                    h = hash_combine_ordered(h, *b as u32)
                }
                h
            }
            Value::ShortArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    h = hash_combine_ordered(h, *item as u32)
                }
                h
            }
            Value::IntArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    h = hash_combine_ordered(h, *item as u32)
                }
                h
            }
            Value::CharArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    h = hash_combine_ordered(h, *item as u32)
                }
                h
            }
            Value::LongArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    let v = *item;
                    h = hash_combine_ordered(h, hash_i64(v));
                }
                h
            }
            Value::FloatArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    let f = *item;
                    h = hash_combine_ordered(
                        h,
                        if f.fract() == 0.0
                            && f.is_finite()
                            && let Some(n) = ToPrimitive::to_i64(item)
                        {
                            hash_i64(n)
                        } else {
                            hash_i64(f.to_bits() as i64)
                        },
                    )
                }
                h
            }
            Value::DoubleArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().lock().unwrap().iter() {
                    let f = *item;
                    h = hash_combine_ordered(
                        h,
                        if f.fract() == 0.0
                            && f.is_finite()
                            && let Some(n) = ToPrimitive::to_i64(item)
                        {
                            hash_i64(n)
                        } else {
                            hash_i64(f.to_bits() as i64)
                        },
                    )
                }
                h
            }
            Value::ObjectArray(a) => {
                let mut h: u32 = 0;
                for item in a.get().0.lock().unwrap().iter() {
                    h = hash_combine_ordered(h, item.clojure_hash())
                }
                h
            }

            // For non-data types, use pointer identity.
            Value::Fn(f) => f.get() as *const _ as u32,
            Value::BoundFn(f) => f.get() as *const _ as u32,
            Value::NativeFunction(f) => f.get() as *const _ as u32,
            Value::Var(v) => v.get() as *const _ as u32,
            Value::Atom(a) => a.get() as *const _ as u32,
            Value::Namespace(n) => n.get() as *const _ as u32,
            Value::Queue(q) => {
                let mut h: u32 = 1;
                for v in q.get().iter() {
                    h = hash_combine_ordered(h, v.clojure_hash());
                }
                h
            }
            Value::Macro(f) => f.get() as *const _ as u32,
            Value::BigDecimal(d) => hash_string(&d.get().to_string()),
            Value::Ratio(r) => hash_string(&r.get().to_string()),
            Value::LazySeq(ls) => ls.get().realize().clojure_hash(),
            Value::Protocol(p) => p.get() as *const _ as u32,
            Value::ProtocolFn(pf) => pf.get() as *const _ as u32,
            Value::MultiFn(mf) => mf.get() as *const _ as u32,
            Value::Cons(_) => {
                // Hash like an ordered sequence.
                let mut h: u32 = 1;
                for v in value_to_seq_vec(self) {
                    h = hash_combine_ordered(h, v.clojure_hash());
                }
                h
            }
            // Pointer identity for concurrency primitives.
            Value::Volatile(v) => v.get() as *const _ as u32,
            Value::Delay(d) => d.get() as *const _ as u32,
            Value::Promise(p) => p.get() as *const _ as u32,
            Value::Future(fu) => fu.get() as *const _ as u32,
            Value::Agent(a) => a.get() as *const _ as u32,
            // Record hash: combine type tag hash with fields hash.
            Value::TypeInstance(ti) => {
                let tag_hash = hash_string(&ti.get().type_tag);
                let mut fields_hash: u32 = 0;
                ti.get().fields.for_each(|k, v| {
                    fields_hash = hash_combine_unordered(
                        fields_hash,
                        hash_combine_ordered(k.clojure_hash(), v.clojure_hash()),
                    );
                });
                hash_combine_ordered(tag_hash, fields_hash)
            }
            Value::Error(e) => e.get().clojure_hash(),
        }
    }
}

// Implement std::hash::Hash by delegating to ClojureHash.
impl std::hash::Hash for Value {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.clojure_hash().hash(state);
    }
}

// ── Display / pr-str ──────────────────────────────────────────────────────────

impl fmt::Display for Value {
    /// Prints in `pr-str` style (readable): strings are quoted, chars use `\` notation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        pr_str(self, f, true)
    }
}

/// A wrapper for printing a Value non-readably (for `str`, `println`).
pub struct PrintValue<'a>(pub &'a Value);

impl fmt::Display for PrintValue<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        pr_str(self.0, f, false)
    }
}

/// Print a value.  `readably = true` quotes strings and escapes chars.
pub fn pr_str(v: &Value, f: &mut fmt::Formatter<'_>, readably: bool) -> fmt::Result {
    match v {
        Value::WithMeta(inner, _) => pr_str(inner, f, readably),
        Value::Reduced(inner) => {
            write!(f, "#reduced ")?;
            pr_str(inner, f, readably)
        }
        Value::Nil => write!(f, "nil"),
        Value::Bool(b) => write!(f, "{b}"),
        Value::Long(n) => write!(f, "{n}"),
        Value::Double(d) => {
            if d.is_infinite() {
                if readably {
                    if *d > 0.0 {
                        write!(f, "##Inf")
                    } else {
                        write!(f, "##-Inf")
                    }
                } else if *d > 0.0 {
                    write!(f, "Infinity")
                } else {
                    write!(f, "-Infinity")
                }
            } else if d.is_nan() {
                if readably {
                    write!(f, "##NaN")
                } else {
                    write!(f, "NaN")
                }
            } else if d.fract() == 0.0 && d.abs() < 1e15 {
                write!(f, "{d:.1}")
            } else {
                write!(f, "{d}")
            }
        }
        Value::BigInt(n) => {
            if readably {
                write!(f, "{}N", n.get())
            } else {
                write!(f, "{}", n.get())
            }
        }
        Value::BigDecimal(d) => {
            let dec = d.get();
            let s = format!("{}", dec);
            // bigdecimal Display omits trailing zeros for zero values (e.g. 0.0 → "0").
            // Preserve scale: if the value has fractional digits but displays without a dot, add them.
            if !s.contains('.') && dec.fractional_digit_count() > 0 {
                let zeros = "0".repeat(dec.fractional_digit_count() as usize);
                if readably {
                    write!(f, "{s}.{zeros}M")
                } else {
                    write!(f, "{s}.{zeros}")
                }
            } else if readably {
                write!(f, "{s}M")
            } else {
                write!(f, "{s}")
            }
        }
        Value::Ratio(r) => write!(f, "{}", r.get()),
        Value::Uuid(u) => {
            let uuid = uuid::Uuid::from_u128(*u);
            if readably {
                write!(f, "#uuid \"{}\"", uuid)
            } else {
                write!(f, "{}", uuid)
            }
        }
        Value::Char(c) => {
            if readably {
                match c {
                    '\n' => write!(f, "\\newline"),
                    '\t' => write!(f, "\\tab"),
                    ' ' => write!(f, "\\space"),
                    '\r' => write!(f, "\\return"),
                    c => write!(f, "\\{c}"),
                }
            } else {
                write!(f, "{c}")
            }
        }
        Value::Str(s) => {
            if readably {
                write!(f, "\"")?;
                for c in s.get().chars() {
                    match c {
                        '"' => write!(f, "\\\"")?,
                        '\\' => write!(f, "\\\\")?,
                        '\n' => write!(f, "\\n")?,
                        '\t' => write!(f, "\\t")?,
                        '\r' => write!(f, "\\r")?,
                        c => write!(f, "{c}")?,
                    }
                }
                write!(f, "\"")
            } else {
                write!(f, "{}", s.get())
            }
        }
        Value::Pattern(r) => {
            if readably {
                write!(f, "#\"")?;
                write!(f, "{}", r.get().as_str())?;
                write!(f, "\"")
            } else {
                write!(f, "#<{}>", r.get())
            }
        }
        Value::Matcher(_) => write!(f, "#<Matcher>"),
        Value::Symbol(s) => write!(f, "{}", s.get()),
        Value::Keyword(k) => write!(f, "{}", k.get()),
        Value::List(l) => {
            write!(f, "(")?;
            let mut first = true;
            for item in l.get().iter() {
                if !first {
                    write!(f, " ")?;
                }
                pr_str(item, f, readably)?;
                first = false;
            }
            write!(f, ")")
        }
        Value::Vector(v) => {
            write!(f, "[")?;
            let mut first = true;
            for item in v.get().iter() {
                if !first {
                    write!(f, " ")?;
                }
                pr_str(item, f, readably)?;
                first = false;
            }
            write!(f, "]")
        }
        Value::Map(m) => {
            write!(f, "{{")?;
            let mut first = true;
            m.for_each(|k, v| {
                // Ignore fmt errors inside the closure — limitations of fmt.
                if !first {
                    let _ = write!(f, ", ");
                }
                let _ = pr_str(k, f, readably);
                let _ = write!(f, " ");
                let _ = pr_str(v, f, readably);
                first = false;
            });
            write!(f, "}}")
        }
        Value::Set(s) => {
            write!(f, "#{{")?;
            let mut first = true;
            for item in s.iter() {
                if !first {
                    write!(f, " ")?;
                }
                pr_str(item, f, readably)?;
                first = false;
            }
            write!(f, "}}")
        }
        Value::BooleanArray(_)
        | Value::ByteArray(_)
        | Value::ShortArray(_)
        | Value::IntArray(_)
        | Value::LongArray(_)
        | Value::CharArray(_)
        | Value::FloatArray(_)
        | Value::DoubleArray(_)
        | Value::ObjectArray(_) => write!(f, "#[array]"),
        Value::Queue(q) => {
            // Printed as a list with a type tag.
            write!(f, "#queue (")?;
            let mut first = true;
            for item in q.get().iter() {
                if !first {
                    write!(f, " ")?;
                }
                pr_str(item, f, readably)?;
                first = false;
            }
            write!(f, ")")
        }
        Value::LazySeq(ls) => pr_str(&ls.get().realize(), f, readably),
        Value::Cons(c) => {
            write!(f, "(")?;
            pr_str(&c.get().head, f, readably)?;
            let mut tail = c.get().tail.clone();
            loop {
                match tail {
                    Value::Nil => break,
                    Value::List(l) => {
                        for item in l.get().iter() {
                            write!(f, " ")?;
                            pr_str(item, f, readably)?;
                        }
                        break;
                    }
                    Value::Cons(next_c) => {
                        write!(f, " ")?;
                        pr_str(&next_c.get().head, f, readably)?;
                        tail = next_c.get().tail.clone();
                    }
                    Value::LazySeq(ls) => {
                        tail = ls.get().realize();
                    }
                    other => {
                        write!(f, " . ")?;
                        pr_str(&other, f, readably)?;
                        break;
                    }
                }
            }
            write!(f, ")")
        }
        Value::NativeFunction(nf) => write!(f, "#<NativeFn {}>", nf.get().name),
        Value::BoundFn(_) => write!(f, "#<BoundFn>"),
        Value::Fn(fun) => match &fun.get().name {
            Some(n) => write!(f, "#<Fn {n}>"),
            None => write!(f, "#<Fn>"),
        },
        Value::Macro(m) => match &m.get().name {
            Some(n) => write!(f, "#<Macro {n}>"),
            None => write!(f, "#<Macro>"),
        },
        Value::Var(v) => write!(f, "#'{}/{}", v.get().namespace, v.get().name),
        Value::Atom(a) => write!(f, "#<Atom {}>", a.get().deref()),
        Value::Namespace(n) => write!(f, "#<Namespace {}>", n.get().name),
        Value::Protocol(p) => write!(f, "#<Protocol {}>", p.get().name),
        Value::ProtocolFn(pf) => {
            write!(
                f,
                "#<fn {}/{}>",
                pf.get().protocol.get().name,
                pf.get().method_name
            )
        }
        Value::MultiFn(mf) => write!(f, "#<MultiFn {}>", mf.get().name),
        Value::Volatile(_) => write!(f, "#<Volatile>"),
        Value::Delay(_) => write!(f, "#<Delay>"),
        Value::Promise(_) => write!(f, "#<Promise>"),
        Value::Future(_) => write!(f, "#<Future>"),
        Value::Agent(_) => write!(f, "#<Agent>"),
        Value::TypeInstance(ti) => {
            let ti = ti.get();
            write!(f, "#{}{{", ti.type_tag)?;
            let mut first = true;
            ti.fields.for_each(|k, v| {
                if !first {
                    let _ = write!(f, ", ");
                }
                let _ = pr_str(k, f, readably);
                let _ = write!(f, " ");
                let _ = pr_str(v, f, readably);
                first = false;
            });
            write!(f, "}}")
        }
        Value::NativeObject(obj) => {
            write!(f, "#<{} {:?}>", obj.get().type_tag(), obj.get().inner())
        }
        Value::Resource(r) => {
            if r.is_closed() {
                write!(f, "#<{} (closed)>", r.resource_type())
            } else {
                write!(f, "#<{}>", r.resource_type())
            }
        }
        Value::TransientMap(_) => write!(f, "#<TransientMap>"),
        Value::TransientSet(_) => write!(f, "#<TransientSet>"),
        Value::TransientVector(_) => write!(f, "#<TransientVector>"),
        Value::Error(e) => {
            write!(f, "#error ")?;
            let map = e.get().to_map().map_err(|_| fmt::Error {})?;
            pr_str(&map, f, readably)
        }
    }
}

// ── Metadata helpers ─────────────────────────────────────────────────────────

impl Value {
    /// Strip any `WithMeta` wrapper, returning the underlying value.
    pub fn unwrap_meta(&self) -> &Value {
        match self {
            Value::WithMeta(inner, _) => inner.unwrap_meta(),
            other => other,
        }
    }

    /// Return metadata if present, or `None`.
    pub fn get_meta(&self) -> Option<&Value> {
        match self {
            Value::WithMeta(_, meta) => Some(meta),
            _ => None,
        }
    }

    /// Return a new value with metadata attached.
    pub fn with_meta(self, meta: Value) -> Value {
        match self {
            Value::WithMeta(inner, _) => Value::WithMeta(inner, Box::new(meta)),
            other => Value::WithMeta(Box::new(other), Box::new(meta)),
        }
    }
}

// ── type_name helper ──────────────────────────────────────────────────────────

impl Value {
    /// A human-readable type name for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::WithMeta(inner, _) => inner.type_name(),
            Value::Reduced(_) => "reduced",
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Long(_) => "long",
            Value::Double(_) => "double",
            Value::BigInt(_) => "bigint",
            Value::BigDecimal(_) => "bigdecimal",
            Value::Ratio(_) => "ratio",
            Value::Char(_) => "char",
            Value::Str(_) => "string",
            Value::Pattern(_) => "pattern",
            Value::Matcher(_) => "matcher",
            Value::Symbol(_) => "symbol",
            Value::Keyword(_) => "keyword",
            Value::Uuid(_) => "uuid",
            Value::List(_) => "list",
            Value::Vector(_) => "vector",
            Value::Map(_) => "map",
            Value::Set(_) => "set",
            Value::Queue(_) => "queue",
            Value::NativeFunction(_)
            | Value::Fn(_)
            | Value::BoundFn(_)
            | Value::Macro(_)
            | Value::ProtocolFn(_)
            | Value::MultiFn(_) => "fn",
            Value::Var(_) => "var",
            Value::Atom(_) => "atom",
            Value::Namespace(_) => "namespace",
            Value::LazySeq(_) => "lazyseq",
            Value::Cons(_) => "cons",
            Value::Protocol(_) => "protocol",
            Value::Volatile(_) => "volatile",
            Value::Delay(_) => "delay",
            Value::Promise(_) => "promise",
            Value::Future(_) => "future",
            Value::Agent(_) => "agent",
            Value::TypeInstance(_) => "record",
            Value::NativeObject(_) => "native-object",
            Value::BooleanArray(_) => "boolean-array",
            Value::ByteArray(_) => "byte-array",
            Value::ShortArray(_) => "short-array",
            Value::IntArray(_) => "int-array",
            Value::LongArray(_) => "long-array",
            Value::FloatArray(_) => "float-array",
            Value::DoubleArray(_) => "double-array",
            Value::CharArray(_) => "char-array",
            Value::ObjectArray(_) => "object-array",
            Value::Resource(r) => r.resource_type(),
            Value::TransientMap(_) => "transient-map",
            Value::TransientSet(_) => "transient-set",
            Value::TransientVector(_) => "transient-vector",
            Value::Error(_) => "error",
        }
    }

    /// Convenience: wrap a `&str` in `Value::Str`.
    pub fn string(s: impl Into<String>) -> Self {
        Value::Str(GcPtr::new(s.into()))
    }

    /// Convenience: wrap a `Symbol`.
    pub fn symbol(s: Symbol) -> Self {
        Value::Symbol(GcPtr::new(s))
    }

    /// Convenience: wrap a `Keyword`.
    pub fn keyword(k: Keyword) -> Self {
        Value::Keyword(GcPtr::new(k))
    }

    /// True for sequential collections (list, vector, lazy seq, cons).
    pub fn is_sequential(&self) -> bool {
        matches!(
            self,
            Value::List(_) | Value::Vector(_) | Value::LazySeq(_) | Value::Cons(_)
        )
    }

    /// True for any collection.
    pub fn is_coll(&self) -> bool {
        self.unwrap_meta().is_coll_inner()
    }

    fn is_coll_inner(&self) -> bool {
        matches!(
            self,
            Value::List(_)
                | Value::Vector(_)
                | Value::Map(_)
                | Value::Set(_)
                | Value::Queue(_)
                | Value::LazySeq(_)
                | Value::Cons(_)
        )
    }
}

impl cljrs_gc::Trace for Value {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
        match self {
            Value::Reduced(inner) => inner.trace(visitor),
            Value::WithMeta(inner, meta) => {
                inner.trace(visitor);
                meta.trace(visitor);
            }
            Value::Nil
            | Value::Bool(_)
            | Value::Long(_)
            | Value::Double(_)
            | Value::Char(_)
            | Value::Uuid(_) => {}
            Value::BigInt(p) => visitor.visit(p),
            Value::BigDecimal(p) => visitor.visit(p),
            Value::Ratio(p) => visitor.visit(p),
            Value::Str(p) => visitor.visit(p),
            Value::Pattern(p) => visitor.visit(p),
            Value::Matcher(m) => visitor.visit(m),
            Value::Symbol(p) => visitor.visit(p),
            Value::Keyword(p) => visitor.visit(p),
            Value::List(p) => visitor.visit(p),
            Value::Vector(p) => visitor.visit(p),
            Value::Map(m) => m.trace(visitor),
            Value::Set(s) => s.trace(visitor),
            Value::Queue(p) => visitor.visit(p),
            Value::NativeFunction(p) => visitor.visit(p),
            Value::BoundFn(p) => visitor.visit(p),
            Value::Fn(p) | Value::Macro(p) => visitor.visit(p),
            Value::Var(p) => visitor.visit(p),
            Value::Atom(p) => visitor.visit(p),
            Value::Namespace(p) => visitor.visit(p),
            Value::LazySeq(p) => visitor.visit(p),
            Value::Cons(p) => visitor.visit(p),
            Value::Protocol(p) => visitor.visit(p),
            Value::ProtocolFn(p) => visitor.visit(p),
            Value::MultiFn(p) => visitor.visit(p),
            Value::Volatile(p) => visitor.visit(p),
            Value::Delay(p) => visitor.visit(p),
            Value::Promise(p) => visitor.visit(p),
            Value::Future(p) => visitor.visit(p),
            Value::Agent(p) => visitor.visit(p),
            Value::TypeInstance(p) => visitor.visit(p),
            Value::ObjectArray(p) => visitor.visit(p),
            Value::BooleanArray(_)
            | Value::ByteArray(_)
            | Value::ShortArray(_)
            | Value::IntArray(_)
            | Value::LongArray(_)
            | Value::FloatArray(_)
            | Value::DoubleArray(_)
            | Value::CharArray(_) => {}
            Value::NativeObject(p) => visitor.visit(p),
            // Resource is Arc-ref-counted, not GcPtr — nothing to trace.
            Value::Resource(_) => {}
            Value::TransientMap(m) => visitor.visit(m),
            Value::TransientVector(p) => visitor.visit(p),
            Value::TransientSet(m) => visitor.visit(m),
            Value::Error(e) => visitor.visit(e),
        }
    }
}

impl cljrs_gc::Trace for MapValue {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        use cljrs_gc::GcVisitor as _;
        match self {
            MapValue::Array(p) => visitor.visit(p),
            MapValue::Hash(p) => visitor.visit(p),
            MapValue::Sorted(p) => visitor.visit(p),
        }
    }
}

// ── TypeInstance ──────────────────────────────────────────────────────────────

/// A record or reify instance.  `type_tag` identifies the concrete type;
/// `fields` holds the key/value pairs (keyword → value).
#[derive(Clone, Debug)]
pub struct TypeInstance {
    pub type_tag: Arc<str>,
    pub fields: MapValue,
}

impl cljrs_gc::Trace for TypeInstance {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.fields.trace(visitor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kw(s: &str) -> Value {
        Value::keyword(Keyword::simple(s))
    }
    fn sym(s: &str) -> Value {
        Value::symbol(Symbol::simple(s))
    }
    fn int(n: i64) -> Value {
        Value::Long(n)
    }
    fn s(v: &str) -> Value {
        Value::string(v)
    }

    // ── Equality ──────────────────────────────────────────────────────────────

    #[test]
    fn test_nil_eq() {
        assert_eq!(Value::Nil, Value::Nil);
        assert_ne!(Value::Nil, int(0));
    }

    #[test]
    fn test_numeric_cross_type() {
        let big1 = Value::BigInt(GcPtr::new(BigInt::from(1i64)));
        assert_eq!(int(1), big1.clone());
        assert_eq!(big1, int(1));
        // 1.0 == 1
        assert_eq!(Value::Double(1.0), int(1));
        assert_eq!(int(1), Value::Double(1.0));
        // 1.5 != 1
        assert_ne!(Value::Double(1.5), int(1));
    }

    #[test]
    fn test_nan_not_equal_to_itself() {
        let nan = Value::Double(f64::NAN);
        assert_ne!(nan, nan.clone());
    }

    #[test]
    fn test_string_equality() {
        assert_eq!(s("hello"), s("hello"));
        assert_ne!(s("hello"), s("world"));
    }

    #[test]
    fn test_list_vector_seq_equality() {
        let list = Value::List(GcPtr::new(PersistentList::from_iter([int(1), int(2)])));
        let vec = Value::Vector(GcPtr::new(PersistentVector::from_iter([int(1), int(2)])));
        // Clojure: (= '(1 2) [1 2]) => true
        assert_eq!(list, vec);
    }

    #[test]
    fn test_map_equality_order_independent() {
        let mut a = MapValue::empty();
        a = a.assoc(kw("a"), int(1));
        a = a.assoc(kw("b"), int(2));

        let mut b = MapValue::empty();
        b = b.assoc(kw("b"), int(2));
        b = b.assoc(kw("a"), int(1));

        assert_eq!(Value::Map(a), Value::Map(b));
    }

    // ── Hashing ───────────────────────────────────────────────────────────────

    #[test]
    fn test_hash_consistency() {
        let big1 = Value::BigInt(GcPtr::new(BigInt::from(1i64)));
        assert_eq!(int(1).clojure_hash(), big1.clojure_hash());
    }

    #[test]
    fn test_hash_whole_double() {
        // (= 1 1.0) → true, so (hash 1) == (hash 1.0)
        assert_eq!(int(1).clojure_hash(), Value::Double(1.0).clojure_hash());
    }

    // ── Display ───────────────────────────────────────────────────────────────

    #[test]
    fn test_pr_str_nil() {
        assert_eq!(Value::Nil.to_string(), "nil");
    }

    #[test]
    fn test_pr_str_string() {
        assert_eq!(s("hello").to_string(), "\"hello\"");
        assert_eq!(s("a\"b").to_string(), "\"a\\\"b\"");
    }

    #[test]
    fn test_pr_str_char() {
        assert_eq!(Value::Char('a').to_string(), "\\a");
        assert_eq!(Value::Char('\n').to_string(), "\\newline");
    }

    #[test]
    fn test_pr_str_list() {
        let l = Value::List(GcPtr::new(PersistentList::from_iter([int(1), int(2)])));
        assert_eq!(l.to_string(), "(1 2)");
    }

    #[test]
    fn test_pr_str_vector() {
        let v = Value::Vector(GcPtr::new(PersistentVector::from_iter([int(1), int(2)])));
        assert_eq!(v.to_string(), "[1 2]");
    }

    #[test]
    fn test_pr_str_double() {
        assert_eq!(Value::Double(1.0).to_string(), "1.0");
        assert_eq!(Value::Double(3.14).to_string(), "3.14");
        assert_eq!(Value::Double(f64::INFINITY).to_string(), "##Inf");
        assert_eq!(Value::Double(f64::NEG_INFINITY).to_string(), "##-Inf");
        assert_eq!(Value::Double(f64::NAN).to_string(), "##NaN");
    }
}

// Ord impl for Value

impl PartialOrd for Value {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Value {
    fn cmp(&self, other: &Self) -> Ordering {
        // Strip metadata for comparison.
        let a = self.unwrap_meta();
        let b = other.unwrap_meta();
        if !std::ptr::eq(a, self) || !std::ptr::eq(b, other) {
            return a.cmp(b);
        }
        // Same-type fast paths first, then cross-type numeric, then type-discriminant fallback.
        match (self, other) {
            // ── Nil ──
            (Value::Nil, Value::Nil) => Ordering::Equal,

            // ── Booleans ──
            (Value::Bool(a), Value::Bool(b)) => a.cmp(b),

            // ── Numerics (same type) ──
            (Value::Long(a), Value::Long(b)) => a.cmp(b),
            (Value::Double(a), Value::Double(b)) => total_cmp_f64(*a, *b),
            (Value::BigInt(a), Value::BigInt(b)) => a.get().cmp(b.get()),
            (Value::BigDecimal(a), Value::BigDecimal(b)) => {
                // BigDecimal doesn't impl Ord; compare via PartialOrd then string fallback.
                a.get()
                    .partial_cmp(b.get())
                    .unwrap_or_else(|| a.get().to_string().cmp(&b.get().to_string()))
            }
            (Value::Ratio(a), Value::Ratio(b)) => a.get().cmp(b.get()),

            // ── Cross-type numerics (promote to common type) ──
            (Value::Long(a), Value::Double(b)) => total_cmp_f64(*a as f64, *b),
            (Value::Double(a), Value::Long(b)) => total_cmp_f64(*a, *b as f64),
            (Value::Long(a), Value::BigInt(b)) => BigInt::from(*a).cmp(b.get()),
            (Value::BigInt(a), Value::Long(b)) => a.get().cmp(&BigInt::from(*b)),
            (Value::Long(a), Value::Ratio(b)) => {
                num_rational::Ratio::from(BigInt::from(*a)).cmp(b.get())
            }
            (Value::Ratio(a), Value::Long(b)) => {
                a.get().cmp(&num_rational::Ratio::from(BigInt::from(*b)))
            }
            (Value::BigInt(a), Value::Ratio(b)) => {
                num_rational::Ratio::from(a.get().clone()).cmp(b.get())
            }
            (Value::Ratio(a), Value::BigInt(b)) => {
                a.get().cmp(&num_rational::Ratio::from(b.get().clone()))
            }
            (Value::Double(a), Value::BigInt(b)) => {
                total_cmp_f64(*a, b.get().to_f64().unwrap_or(f64::MAX))
            }
            (Value::BigInt(a), Value::Double(b)) => {
                total_cmp_f64(a.get().to_f64().unwrap_or(f64::MAX), *b)
            }
            (Value::Double(a), Value::Ratio(b)) => {
                total_cmp_f64(*a, b.get().to_f64().unwrap_or(f64::MAX))
            }
            (Value::Ratio(a), Value::Double(b)) => {
                total_cmp_f64(a.get().to_f64().unwrap_or(f64::MAX), *b)
            }
            (Value::Long(a), Value::BigDecimal(b)) => {
                let ad = bigdecimal::BigDecimal::from(*a);
                ad.partial_cmp(b.get())
                    .unwrap_or_else(|| ad.to_string().cmp(&b.get().to_string()))
            }
            (Value::BigDecimal(a), Value::Long(b)) => {
                let bd = bigdecimal::BigDecimal::from(*b);
                a.get()
                    .partial_cmp(&bd)
                    .unwrap_or_else(|| a.get().to_string().cmp(&bd.to_string()))
            }
            (Value::Double(a), Value::BigDecimal(b)) => {
                match bigdecimal::BigDecimal::try_from(*a) {
                    Ok(ad) => ad
                        .partial_cmp(b.get())
                        .unwrap_or_else(|| ad.to_string().cmp(&b.get().to_string())),
                    Err(_) => {
                        // NaN or infinity
                        if a.is_nan() {
                            Ordering::Greater
                        } else if *a < 0.0 {
                            Ordering::Less
                        } else {
                            Ordering::Greater
                        }
                    }
                }
            }
            (Value::BigDecimal(a), Value::Double(b)) => {
                match bigdecimal::BigDecimal::try_from(*b) {
                    Ok(bd) => a
                        .get()
                        .partial_cmp(&bd)
                        .unwrap_or_else(|| a.get().to_string().cmp(&bd.to_string())),
                    Err(_) => {
                        if b.is_nan() {
                            Ordering::Less
                        } else if *b < 0.0 {
                            Ordering::Greater
                        } else {
                            Ordering::Less
                        }
                    }
                }
            }
            (Value::BigInt(a), Value::BigDecimal(b)) => {
                let ad = bigdecimal::BigDecimal::from(a.get().clone());
                ad.partial_cmp(b.get())
                    .unwrap_or_else(|| ad.to_string().cmp(&b.get().to_string()))
            }
            (Value::BigDecimal(a), Value::BigInt(b)) => {
                let bd = bigdecimal::BigDecimal::from(b.get().clone());
                a.get()
                    .partial_cmp(&bd)
                    .unwrap_or_else(|| a.get().to_string().cmp(&bd.to_string()))
            }
            (Value::Ratio(a), Value::BigDecimal(b)) => {
                let af = a.get().to_f64().unwrap_or(f64::MAX);
                match bigdecimal::BigDecimal::try_from(af) {
                    Ok(ad) => ad
                        .partial_cmp(b.get())
                        .unwrap_or_else(|| ad.to_string().cmp(&b.get().to_string())),
                    Err(_) => Ordering::Greater,
                }
            }
            (Value::BigDecimal(a), Value::Ratio(b)) => {
                let bf = b.get().to_f64().unwrap_or(f64::MAX);
                match bigdecimal::BigDecimal::try_from(bf) {
                    Ok(bd) => a
                        .get()
                        .partial_cmp(&bd)
                        .unwrap_or_else(|| a.get().to_string().cmp(&bd.to_string())),
                    Err(_) => Ordering::Less,
                }
            }

            // ── Characters ──
            (Value::Char(a), Value::Char(b)) => a.cmp(b),

            // ── Strings ──
            (Value::Str(a), Value::Str(b)) => a.get().cmp(b.get()),

            // ── Symbols ──
            (Value::Symbol(a), Value::Symbol(b)) => cmp_ns_name(
                &a.get().namespace,
                &a.get().name,
                &b.get().namespace,
                &b.get().name,
            ),

            // ── Keywords ──
            (Value::Keyword(a), Value::Keyword(b)) => cmp_ns_name(
                &a.get().namespace,
                &a.get().name,
                &b.get().namespace,
                &b.get().name,
            ),

            // ── Sequential collections: element-by-element ──
            (Value::Vector(a), Value::Vector(b)) => iter_cmp(a.get().iter(), b.get().iter()),
            (Value::List(a), Value::List(b)) => iter_cmp(a.get().iter(), b.get().iter()),

            // ── Sets: compare by size, then elements ──
            (Value::Set(a), Value::Set(b)) => a.count().cmp(&b.count()),

            // ── Maps: compare by size ──
            (Value::Map(a), Value::Map(b)) => a.count().cmp(&b.count()),

            // ── Different types: order by type discriminant for a consistent total order ──
            _ => type_discriminant(self).cmp(&type_discriminant(other)),
        }
    }
}

/// Compare two iterators of Values element-by-element.
fn iter_cmp<'a>(
    mut a: impl Iterator<Item = &'a Value>,
    mut b: impl Iterator<Item = &'a Value>,
) -> Ordering {
    loop {
        match (a.next(), b.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) => {
                let c = x.cmp(y);
                if c != Ordering::Equal {
                    return c;
                }
            }
        }
    }
}

/// Total ordering for f64: NaN sorts after everything else, otherwise use IEEE total_order.
fn total_cmp_f64(a: f64, b: f64) -> Ordering {
    a.total_cmp(&b)
}

/// Compare namespace-qualified names: namespace first (None < Some), then name.
fn cmp_ns_name(
    ns_a: &Option<Arc<str>>,
    name_a: &Arc<str>,
    ns_b: &Option<Arc<str>>,
    name_b: &Arc<str>,
) -> Ordering {
    match (ns_a, ns_b) {
        (None, None) => name_a.cmp(name_b),
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b).then_with(|| name_a.cmp(name_b)),
    }
}

/// Assign a stable integer to each Value variant for cross-type ordering.
fn type_discriminant(v: &Value) -> u8 {
    match v {
        Value::WithMeta(inner, _) => type_discriminant(inner),
        Value::Reduced(inner) => type_discriminant(inner),
        Value::Nil => 0,
        Value::Bool(_) => 1,
        Value::Long(_)
        | Value::Double(_)
        | Value::BigInt(_)
        | Value::BigDecimal(_)
        | Value::Ratio(_) => 2,
        Value::Char(_) => 3,
        Value::Str(_) => 4,
        Value::Symbol(_) => 5,
        Value::Keyword(_) => 6,
        Value::List(_) => 7,
        Value::Vector(_) => 8,
        Value::Map(_) => 9,
        Value::Set(_) => 10,
        Value::Queue(_) => 11,
        Value::LazySeq(_) => 12,
        Value::Cons(_) => 13,
        Value::NativeFunction(_) => 14,
        Value::BoundFn(_) => 14,
        Value::Fn(_) => 15,
        Value::Macro(_) => 16,
        Value::Var(_) => 17,
        Value::Atom(_) => 18,
        Value::Namespace(_) => 19,
        Value::Protocol(_) => 20,
        Value::ProtocolFn(_) => 21,
        Value::MultiFn(_) => 22,
        Value::Volatile(_) => 23,
        Value::Delay(_) => 24,
        Value::Promise(_) => 25,
        Value::Future(_) => 26,
        Value::Agent(_) => 27,
        Value::TypeInstance(_) => 28,
        Value::BooleanArray(_) => 29,
        Value::ByteArray(_) => 30,
        Value::ShortArray(_) => 31,
        Value::IntArray(_) => 32,
        Value::LongArray(_) => 33,
        Value::CharArray(_) => 34,
        Value::FloatArray(_) => 35,
        Value::DoubleArray(_) => 36,
        Value::ObjectArray(_) => 37,
        Value::Uuid(_) => 38,
        Value::NativeObject(_) => 43,
        Value::Resource(_) => 39,
        Value::TransientMap(_) => 40,
        Value::TransientSet(_) => 41,
        Value::TransientVector(_) => 42,
        Value::Pattern(_) => 43,
        Value::Matcher(_) => 44,
        Value::Error(_) => 45,
    }
}
