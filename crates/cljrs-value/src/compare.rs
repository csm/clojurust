//! The language-level comparison policy shared by `compare` and the sorted
//! collections.
//!
//! [`try_compare`] is the single source of truth for `clojure.core/compare`
//! and for the key order of `sorted-map` / `sorted-set`.  It is fallible:
//! values with no defined ordering (maps, sets, functions, atoms, …, or two
//! values of unrelated types) produce an error instead of an arbitrary answer,
//! so a sorted collection can never silently merge two distinct keys.
//!
//! Comparable values and their equivalence classes:
//!
//! | Values | Order |
//! |---|---|
//! | `nil` | before every other value |
//! | booleans | `false < true` |
//! | numbers (any mix of long, double, bigint, ratio, bigdecimal) | numeric value; `1` and `1.0` compare equal, as does `##NaN` with every number |
//! | strings, characters | code point order |
//! | keywords, symbols | namespace (absent first), then name |
//! | UUIDs | 128-bit value |
//! | vectors | element by element; a proper prefix sorts first |
//!
//! Anything else is an error.  [`Value`]'s Rust `Ord` impl is a separate,
//! infallible ordering for internal use only; it is not used for sorted keys.

use std::cmp::Ordering;

use bigdecimal::BigDecimal;
use num_bigint::BigInt;
use num_rational::Ratio;
use num_traits::ToPrimitive;

use crate::error::{ValueError, ValueResult};
use crate::value::Value;

/// Compare two values under the language comparison policy.
pub fn try_compare(a: &Value, b: &Value) -> ValueResult<Ordering> {
    // Recurses through nested vectors without reaching a safepoint.
    crate::realization::check_stack();
    let a = a.unwrap_meta();
    let b = b.unwrap_meta();
    match (a, b) {
        (Value::Nil, Value::Nil) => Ok(Ordering::Equal),
        (Value::Nil, _) => Ok(Ordering::Less),
        (_, Value::Nil) => Ok(Ordering::Greater),
        (Value::Bool(x), Value::Bool(y)) => Ok(x.cmp(y)),
        _ if is_number(a) && is_number(b) => num_compare(a, b),
        (Value::Str(x), Value::Str(y)) => Ok(x.get().cmp(y.get())),
        (Value::Char(x), Value::Char(y)) => Ok(x.cmp(y)),
        (Value::Keyword(x), Value::Keyword(y)) => Ok(cmp_ns_name(
            x.get().namespace.as_deref(),
            &x.get().name,
            y.get().namespace.as_deref(),
            &y.get().name,
        )),
        (Value::Symbol(x), Value::Symbol(y)) => Ok(cmp_ns_name(
            x.get().namespace.as_deref(),
            &x.get().name,
            y.get().namespace.as_deref(),
            &y.get().name,
        )),
        (Value::Uuid(x), Value::Uuid(y)) => Ok(x.get().0.cmp(&y.get().0)),
        (Value::Vector(x), Value::Vector(y)) => {
            let mut xi = x.get().iter();
            let mut yi = y.get().iter();
            loop {
                match (xi.next(), yi.next()) {
                    (None, None) => return Ok(Ordering::Equal),
                    (None, Some(_)) => return Ok(Ordering::Less),
                    (Some(_), None) => return Ok(Ordering::Greater),
                    (Some(p), Some(q)) => {
                        let c = try_compare(p, q)?;
                        if c != Ordering::Equal {
                            return Ok(c);
                        }
                    }
                }
            }
        }
        _ => Err(ValueError::Other(format!(
            "cannot compare {} to {}",
            a.type_name(),
            b.type_name()
        ))),
    }
}

fn is_number(v: &Value) -> bool {
    matches!(
        v,
        Value::Long(_)
            | Value::Double(_)
            | Value::BigInt(_)
            | Value::BigDecimal(_)
            | Value::Ratio(_)
    )
}

fn cmp_ns_name(ns_a: Option<&str>, name_a: &str, ns_b: Option<&str>, name_b: &str) -> Ordering {
    match (ns_a, ns_b) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(a), Some(b)) => a.cmp(b),
    }
    .then_with(|| name_a.cmp(name_b))
}

/// Numeric comparison after promotion to the widest operand category
/// (Double > BigDecimal > Ratio/BigInt/Long), matching `<`, `>` and friends.
fn num_compare(a: &Value, b: &Value) -> ValueResult<Ordering> {
    let has = |f: fn(&Value) -> bool| f(a) || f(b);
    if has(|v| matches!(v, Value::Double(_))) {
        let x = as_f64(a);
        let y = as_f64(b);
        Ok(x.partial_cmp(&y).unwrap_or(Ordering::Equal))
    } else if has(|v| matches!(v, Value::BigDecimal(_))) {
        Ok(as_bigdecimal(a).cmp(&as_bigdecimal(b)))
    } else {
        Ok(as_ratio(a).cmp(&as_ratio(b)))
    }
}

fn as_f64(v: &Value) -> f64 {
    match v {
        Value::Long(n) => *n as f64,
        Value::Double(f) => *f,
        Value::BigInt(n) => n.get().to_f64().unwrap_or(f64::INFINITY),
        Value::BigDecimal(d) => d.get().to_f64().unwrap_or(f64::INFINITY),
        Value::Ratio(r) => r.get().to_f64().unwrap_or(f64::NAN),
        _ => unreachable!("as_f64 on non-number"),
    }
}

fn as_bigdecimal(v: &Value) -> BigDecimal {
    match v {
        Value::Long(n) => BigDecimal::from(*n),
        Value::BigInt(n) => BigDecimal::from(n.get().clone()),
        Value::BigDecimal(d) => d.get().clone(),
        Value::Ratio(r) => {
            BigDecimal::from(r.get().numer().clone()) / BigDecimal::from(r.get().denom().clone())
        }
        _ => unreachable!("as_bigdecimal on non-number or double"),
    }
}

fn as_ratio(v: &Value) -> Ratio<BigInt> {
    match v {
        Value::Long(n) => Ratio::from(BigInt::from(*n)),
        Value::BigInt(n) => Ratio::from(n.get().clone()),
        Value::Ratio(r) => r.get().clone(),
        _ => unreachable!("as_ratio on non-rational"),
    }
}

// ── Sorted-collection keys ────────────────────────────────────────────────────

thread_local! {
    /// The first comparison error raised while a [`checked`] operation runs.
    static PENDING: std::cell::RefCell<Option<ValueError>> =
        const { std::cell::RefCell::new(None) };
}

/// A sorted-collection key, ordered by [`try_compare`].
///
/// The underlying red-black tree needs an infallible `Ord`, so a failed
/// comparison is recorded in a thread-local slot and reported as `Less`
/// (never `Equal`, so it can't alias two keys).  Every tree operation runs
/// inside [`checked`], which surfaces the recorded error; an update whose
/// comparisons failed is discarded, leaving the original collection intact.
#[derive(Debug, Clone)]
#[repr(transparent)]
pub(crate) struct SortKey(pub(crate) Value);

impl SortKey {
    /// View a `&Value` as a `&SortKey` for lookups, without cloning.
    pub(crate) fn from_ref(v: &Value) -> &SortKey {
        // SAFETY: `SortKey` is `#[repr(transparent)]` over `Value`.
        unsafe { &*(v as *const Value as *const SortKey) }
    }
}

impl Ord for SortKey {
    fn cmp(&self, other: &Self) -> Ordering {
        match try_compare(&self.0, &other.0) {
            Ok(o) => o,
            Err(e) => {
                PENDING.with(|p| {
                    let mut p = p.borrow_mut();
                    if p.is_none() {
                        *p = Some(e);
                    }
                });
                Ordering::Less
            }
        }
    }
}

impl PartialOrd for SortKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for SortKey {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}

impl Eq for SortKey {}

/// Run a tree operation, returning the first comparison error it raised.
pub(crate) fn checked<T>(f: impl FnOnce() -> T) -> ValueResult<T> {
    let outer = PENDING.with(|p| p.borrow_mut().take());
    let out = f();
    let err = PENDING.with(|p| std::mem::replace(&mut *p.borrow_mut(), outer));
    match err {
        None => Ok(out),
        Some(e) => Err(e),
    }
}
