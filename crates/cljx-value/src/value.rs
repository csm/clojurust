use std::fmt;
use std::sync::Arc;

use num_bigint::BigInt;
use num_traits::ToPrimitive;

use cljx_gc::GcPtr;

use crate::collections::{
    PersistentArrayMap, PersistentHashMap, PersistentHashSet, PersistentList, PersistentQueue,
    PersistentVector,
};
use crate::hash::{
    ClojureHash, hash_combine_ordered, hash_combine_unordered, hash_i64, hash_string,
};
use crate::keyword::Keyword;
use crate::symbol::Symbol;
use crate::types::{
    Agent, Atom, CljxCons, CljxFn, CljxFuture, CljxPromise, Delay, LazySeq, MultiFn, Namespace,
    NativeFn, Protocol, ProtocolFn, Var, Volatile,
};

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

    // ── Identifiers ───────────────────────────────────────────────────────────
    Symbol(GcPtr<Symbol>),
    Keyword(GcPtr<Keyword>),

    // ── Collections ───────────────────────────────────────────────────────────
    List(GcPtr<PersistentList>),
    Vector(GcPtr<PersistentVector>),
    /// Small maps (≤8 entries) are stored as an ArrayMap; larger ones as a HashMap.
    Map(MapValue),
    Set(GcPtr<PersistentHashSet>),
    Queue(GcPtr<PersistentQueue>),

    // ── Functions ─────────────────────────────────────────────────────────────
    NativeFunction(GcPtr<NativeFn>),
    Fn(GcPtr<CljxFn>),
    Macro(GcPtr<CljxFn>),

    // ── Mutable state ─────────────────────────────────────────────────────────
    Var(GcPtr<Var>),
    Atom(GcPtr<Atom>),

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
}

/// A map value: either a small array-map or a HAMT-based hash-map.
#[derive(Clone, Debug)]
pub enum MapValue {
    Array(GcPtr<PersistentArrayMap>),
    Hash(GcPtr<PersistentHashMap>),
}

impl MapValue {
    pub fn empty() -> Self {
        MapValue::Array(GcPtr::new(PersistentArrayMap::empty()))
    }

    pub fn get(&self, key: &Value) -> Option<Value> {
        match self {
            MapValue::Array(m) => m.get().get(key).cloned(),
            MapValue::Hash(m) => m.get().get(key).cloned(),
        }
    }

    pub fn count(&self) -> usize {
        match self {
            MapValue::Array(m) => m.get().count(),
            MapValue::Hash(m) => m.get().count(),
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
        }
    }

    pub fn dissoc(&self, key: &Value) -> Self {
        match self {
            MapValue::Array(m) => MapValue::Array(GcPtr::new(m.get().dissoc(key))),
            MapValue::Hash(m) => MapValue::Hash(GcPtr::new(m.get().dissoc(key))),
        }
    }

    pub fn contains_key(&self, key: &Value) -> bool {
        match self {
            MapValue::Array(m) => m.get().contains_key(key),
            MapValue::Hash(m) => m.get().contains_key(key),
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
        }
    }
}

// ── Equality ──────────────────────────────────────────────────────────────────

impl Eq for Value {}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        // Identity shortcut: same GcPtr → equal without realizing.
        // Required for infinite lazy seqs: (let [r (range)] (= r r)) must not hang.
        if let (Value::LazySeq(a), Value::LazySeq(b)) = (self, other) && GcPtr::ptr_eq(a, b) {
            return true;
        }
        // Realize lazy sequences before comparing.
        if let Value::LazySeq(ls) = self {
            return ls.get().realize() == *other;
        }
        if let Value::LazySeq(ls) = other {
            return *self == ls.get().realize();
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
            (Value::Set(a), Value::Set(b)) => a.get() == b.get(),
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
            // Record equality: same tag and same fields.
            (Value::TypeInstance(a), Value::TypeInstance(b)) => {
                a.get().type_tag == b.get().type_tag && maps_equal(&a.get().fields, &b.get().fields)
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

fn seq_equal(a: &Value, b: &Value) -> bool {
    let a_items = value_to_seq_vec(a);
    let b_items = value_to_seq_vec(b);
    a_items.len() == b_items.len() && a_items.iter().zip(b_items.iter()).all(|(x, y)| x == y)
}

fn value_to_seq_vec(v: &Value) -> Vec<Value> {
    match v {
        Value::List(l) => l.get().iter().cloned().collect(),
        Value::Vector(v) => v.get().iter().cloned().collect(),
        Value::LazySeq(ls) => value_to_seq_vec(&ls.get().realize()),
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
            Value::Keyword(k) => hash_string(&k.get().to_string()),
            Value::Symbol(s) => hash_string(&s.get().to_string()),
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
                for k in s.get().iter() {
                    h = hash_combine_unordered(h, k.clojure_hash());
                }
                h
            }
            // For non-data types, use pointer identity.
            Value::Fn(f) => f.get() as *const _ as u32,
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

/// Print a value.  `readably = true` quotes strings and escapes chars.
pub fn pr_str(v: &Value, f: &mut fmt::Formatter<'_>, readably: bool) -> fmt::Result {
    match v {
        Value::Nil => write!(f, "nil"),
        Value::Bool(b) => write!(f, "{b}"),
        Value::Long(n) => write!(f, "{n}"),
        Value::Double(d) => {
            if d.is_infinite() {
                if *d > 0.0 {
                    write!(f, "##Inf")
                } else {
                    write!(f, "##-Inf")
                }
            } else if d.is_nan() {
                write!(f, "##NaN")
            } else if d.fract() == 0.0 && d.abs() < 1e15 {
                write!(f, "{d:.1}")
            } else {
                write!(f, "{d}")
            }
        }
        Value::BigInt(n) => write!(f, "{}N", n.get()),
        Value::BigDecimal(d) => write!(f, "{}M", d.get()),
        Value::Ratio(r) => write!(f, "{}", r.get()),
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
            for item in s.get().iter() {
                if !first {
                    write!(f, " ")?;
                }
                pr_str(item, f, readably)?;
                first = false;
            }
            write!(f, "}}")
        }
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
    }
}

// ── type_name helper ──────────────────────────────────────────────────────────

impl Value {
    /// A human-readable type name for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Long(_) => "long",
            Value::Double(_) => "double",
            Value::BigInt(_) => "bigint",
            Value::BigDecimal(_) => "bigdecimal",
            Value::Ratio(_) => "ratio",
            Value::Char(_) => "char",
            Value::Str(_) => "string",
            Value::Symbol(_) => "symbol",
            Value::Keyword(_) => "keyword",
            Value::List(_) => "list",
            Value::Vector(_) => "vector",
            Value::Map(_) => "map",
            Value::Set(_) => "set",
            Value::Queue(_) => "queue",
            Value::NativeFunction(_)
            | Value::Fn(_)
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

impl cljx_gc::Trace for Value {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        use cljx_gc::GcVisitor as _;
        match self {
            Value::Nil | Value::Bool(_) | Value::Long(_) | Value::Double(_) | Value::Char(_) => {}
            Value::BigInt(p) => visitor.visit(p),
            Value::BigDecimal(p) => visitor.visit(p),
            Value::Ratio(p) => visitor.visit(p),
            Value::Str(p) => visitor.visit(p),
            Value::Symbol(p) => visitor.visit(p),
            Value::Keyword(p) => visitor.visit(p),
            Value::List(p) => visitor.visit(p),
            Value::Vector(p) => visitor.visit(p),
            Value::Map(m) => m.trace(visitor),
            Value::Set(p) => visitor.visit(p),
            Value::Queue(p) => visitor.visit(p),
            Value::NativeFunction(p) => visitor.visit(p),
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
        }
    }
}

impl cljx_gc::Trace for MapValue {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
        use cljx_gc::GcVisitor as _;
        match self {
            MapValue::Array(p) => visitor.visit(p),
            MapValue::Hash(p) => visitor.visit(p),
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

impl cljx_gc::Trace for TypeInstance {
    fn trace(&self, visitor: &mut cljx_gc::MarkVisitor) {
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
