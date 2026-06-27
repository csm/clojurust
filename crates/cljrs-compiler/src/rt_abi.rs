#![allow(clippy::not_unsafe_ptr_arg_deref)]
//! C-ABI runtime bridge for AOT-compiled clojurust code.
//!
//! Every function here is `extern "C"` with `#[unsafe(no_mangle)]` so the compiled
//! object code can call them by symbol name.  They wrap the existing
//! interpreter/value logic.
//!
//! All Clojure values are passed as `*const Value` (opaque pointers).
//! The compiled code treats them as machine-word-sized tokens; all
//! actual manipulation happens in these bridge functions.

use cljrs_gc::GcPtr;
use cljrs_value::keyword::Keyword;
use cljrs_value::value::{MapValue, PrintValue, SetValue};
use cljrs_value::{
    CljxCons, PersistentHashSet, PersistentList, PersistentVector, Symbol, TypeInstance, Value,
};

use std::sync::{Arc, OnceLock};

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Convert a raw pointer back to a `&Value`.
///
/// # Safety
/// `ptr` must be a valid pointer to a live `Value` (either GC-heap, region, or
/// static).
#[inline]
unsafe fn val_ref<'a>(ptr: *const Value) -> &'a Value {
    unsafe { &*ptr }
}

/// Helper: collect stack-spilled arguments into a Vec.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[inline]
unsafe fn collect_args(elems: *const *const Value, n: i64) -> Vec<Value> {
    let mut args = Vec::with_capacity(n as usize);
    for i in 0..n as usize {
        let ptr = unsafe { *elems.add(i) };
        args.push(unsafe { val_ref(ptr) }.clone());
    }
    args
}

/// Box a `Value` on the GC heap and return a raw pointer.
///
/// The returned pointer is valid until the next GC collection that doesn't
/// mark it.
#[inline]
fn box_val(v: Value) -> *const Value {
    // We allocate a single-element wrapper on the GC heap.
    // The GcPtr keeps the Value alive; we leak the pointer.
    let ptr = GcPtr::new(v);
    // GcPtr<Value> is NonNull<GcBox<Value>>; .get() returns &Value inside.
    ptr.get() as *const Value
}

/// Like `box_val` but allocates in the active region when one is open.
///
/// Only call this for collection `Value` variants (Vector, Set, List, …).
/// Scalar values (Long, Bool, …) must NOT use this — they can escape a region
/// via `RecurJump` and would become dangling pointers after `RegionEnd`.
///
/// Safety: the returned pointer is valid until the active region is popped
/// (i.e. until `rt_region_end`).  The IR analysis guarantees collection values
/// are consumed before that point.
#[inline]
fn box_coll_val(v: Value) -> *const Value {
    if cljrs_gc::region::region_is_active() {
        let ptr: GcPtr<Value> = unsafe { cljrs_gc::region::try_alloc_in_region(v).unwrap() };
        ptr.get() as *const Value
    } else {
        box_val(v)
    }
}

/// Allocate an inner collection type (`PersistentVector`, `PersistentHashSet`,
/// `PersistentList`, …) in the active region when one is open, falling back to
/// the GC heap otherwise.
///
/// # Safety
/// Same region-lifetime constraint as `box_coll_val`.
#[inline]
fn alloc_inner_coll<T: cljrs_gc::Trace + 'static>(val: T) -> GcPtr<T> {
    if cljrs_gc::region::region_is_active() {
        unsafe { cljrs_gc::region::try_alloc_in_region(val).unwrap() }
    } else {
        GcPtr::new(val)
    }
}

// ── Scalar value interning ───────────────────────────────────────────────────
//
// nil, true, false, and integers in 0..INTERN_LONG_MAX are allocated once and
// reused for the lifetime of the process, eliminating the dominant source of
// GC heap allocation in tight loops.
//
// The cache entries are handed to compiled code as raw `*const Value`s and
// nothing ever traces them, so they must NOT live on the GC heap: a heap box
// becomes unreachable the moment its allocation frame pops, survives only the
// lives grace period (one collection), and is then swept — after which every
// compiled use of the cached pointer reads freed memory.  `static_alloc`
// (StaticArena under no-gc, `Box::leak` under GC) gives exactly the
// program-lifetime allocation these caches need; scalars hold no `GcPtr`
// children, so opting out of tracing is sound.

/// Leak a scalar `Value` as program-lifetime memory for an intern cache.
#[inline]
fn intern_static_val(v: Value) -> *const Value {
    cljrs_gc::static_alloc(v).get() as *const Value
}

/// Cached nil pointer (allocated once, reused forever).
#[inline]
fn intern_nil() -> *const Value {
    static PTR: OnceLock<usize> = OnceLock::new();
    *PTR.get_or_init(|| intern_static_val(Value::Nil) as usize) as *const Value
}

/// Cached true/false pointers (allocated once each, reused forever).
#[inline]
fn intern_bool(b: bool) -> *const Value {
    static TRUE_PTR: OnceLock<usize> = OnceLock::new();
    static FALSE_PTR: OnceLock<usize> = OnceLock::new();
    if b {
        *TRUE_PTR.get_or_init(|| intern_static_val(Value::Bool(true)) as usize) as *const Value
    } else {
        *FALSE_PTR.get_or_init(|| intern_static_val(Value::Bool(false)) as usize) as *const Value
    }
}

/// Upper bound (exclusive) of the interned long cache.  Covers loop counters,
/// BFS queue sizes (up to n=1000), and small arithmetic results.
const INTERN_LONG_MAX: i64 = 1024;

/// Route a dynamically-typed `Value` through the intern cache when possible,
/// falling back to `box_val` for all other types.
///
/// Use this anywhere a returned `Value` might be a Nil/Bool/Long — e.g., the
/// result of `rt_call`, `call_global_fn`, interpreter callbacks, etc.
#[inline]
fn box_or_intern_val(v: Value) -> *const Value {
    match &v {
        Value::Nil => intern_nil(),
        Value::Bool(b) => intern_bool(*b),
        Value::Long(n) => intern_long(*n),
        _ => box_val(v),
    }
}

/// Box a `Value` returned from `invoke` / `call_global_fn`.
///
/// Collections go through `box_coll_val` so they land in the active region
/// when one is open; scalars go through `box_or_intern_val`.
#[inline]
fn box_invoke_result(v: Value) -> *const Value {
    match &v {
        Value::Nil => intern_nil(),
        Value::Bool(b) => intern_bool(*b),
        Value::Long(n) => intern_long(*n),
        Value::Vector(_) | Value::Set(_) | Value::List(_) | Value::Map(_) | Value::Cons(_) => {
            box_coll_val(v)
        }
        _ => box_val(v),
    }
}

/// Return a stable pointer for longs in [0, INTERN_LONG_MAX); allocate on the
/// GC heap for everything else.
#[inline]
fn intern_long(n: i64) -> *const Value {
    static CACHE: OnceLock<Vec<usize>> = OnceLock::new();
    if (0..INTERN_LONG_MAX).contains(&n) {
        let cache = CACHE.get_or_init(|| {
            (0..INTERN_LONG_MAX)
                .map(|i| intern_static_val(Value::Long(i)) as usize)
                .collect()
        });
        cache[n as usize] as *const Value
    } else {
        box_val(Value::Long(n))
    }
}

// ── Safepoint ───────────────────────────────────────────────────────────────

/// GC safepoint for AOT-compiled code.
///
/// Emitted at function entry and before recur jumps so that compiled
/// tight loops cooperate with the garbage collector.
#[unsafe(no_mangle)]
pub extern "C" fn rt_safepoint() {
    cljrs_gc::safepoint();
}

// ── Constants ───────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_nil() -> *const Value {
    intern_nil()
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_true() -> *const Value {
    intern_bool(true)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_false() -> *const Value {
    intern_bool(false)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_long(n: i64) -> *const Value {
    intern_long(n)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_double(n: f64) -> *const Value {
    box_val(Value::Double(n))
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_char(c: u32) -> *const Value {
    box_val(Value::Char(char::from_u32(c).unwrap_or('\u{FFFD}')))
}

/// Create a string constant.  `ptr` and `len` describe a UTF-8 byte slice.
///
/// # Safety
/// `ptr` must point to valid UTF-8 data of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_const_string(ptr: *const u8, len: u64) -> *const Value {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as *const () as usize) };
    let s = std::str::from_utf8(bytes).unwrap_or("<invalid utf8>");
    box_val(Value::Str(GcPtr::new(s.to_string())))
}

/// Create a keyword.  `ptr`/`len` is the simple name (no colon prefix).
///
/// Interned globally (Phase 10.6): every distinct keyword literal is boxed
/// once per process and permanently rooted, instead of allocating a fresh
/// `Value::Keyword` on every execution of the constant.  This is also what
/// makes the keyword-constant inline cache sound — the cached pointer can
/// never be swept.
///
/// # Safety
/// `ptr` must point to valid UTF-8 data of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_const_keyword(ptr: *const u8, len: u64) -> *const Value {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as *const () as usize) };
    let name = std::str::from_utf8(bytes).unwrap_or("??");
    intern_keyword(name)
}

/// Create a symbol.  `ptr`/`len` is the simple name.
///
/// # Safety
/// `ptr` must point to valid UTF-8 data of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_const_symbol(ptr: *const u8, len: u64) -> *const Value {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as *const () as usize) };
    let name = std::str::from_utf8(bytes).unwrap_or("??");
    box_val(Value::symbol(Symbol::simple(name)))
}

// ── Truthiness ──────────────────────────────────────────────────────────────

/// Return 1 if the value is truthy (not nil and not false), 0 otherwise.
///
/// # Safety
/// `v` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_truthiness(v: *const Value) -> u8 {
    let v = unsafe { val_ref(v) };
    match v {
        Value::Nil | Value::Bool(false) => 0,
        _ => 1,
    }
}

// ── Arithmetic ──────────────────────────────────────────────────────────────

/// Build a boxed "integer overflow" exception `Value` (mirrors what the
/// interpreter's `throw` wraps non-error values into).
fn make_overflow_exc() -> *const Value {
    let msg = "integer overflow".to_string();
    box_val(Value::Error(GcPtr::new(
        cljrs_value::error::ExceptionInfo::new(
            cljrs_value::ValueError::Other(msg.clone()),
            msg,
            None,
            None,
        ),
    )))
}

/// Construct (but do not throw) the integer-overflow exception value.  The
/// unboxed checked-arithmetic codegen path calls this and feeds it to
/// `rt_throw` on its overflow branch.
///
/// # Safety
/// Trivially safe (no pointer arguments); `extern "C"` for codegen linkage.
#[unsafe(no_mangle)]
pub extern "C" fn rt_overflow_error() -> *const Value {
    make_overflow_exc()
}

/// Raise the integer-overflow exception via the pending-exception slot and
/// return nil, exactly as `(throw …)` does in compiled code.
fn throw_overflow() -> *const Value {
    unsafe { rt_throw(make_overflow_exc()) }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_add(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        // Checked: primitive long `+` throws on overflow (Clojure semantics).
        (Value::Long(x), Value::Long(y)) => match x.checked_add(*y) {
            Some(s) => intern_long(s),
            None => throw_overflow(),
        },
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x + y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 + y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x + *y as f64)),
        _ => rt_const_nil(), // fallback for non-numeric
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_sub(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => match x.checked_sub(*y) {
            Some(s) => intern_long(s),
            None => throw_overflow(),
        },
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x - y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 - y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x - *y as f64)),
        _ => rt_const_nil(),
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_mul(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => match x.checked_mul(*y) {
            Some(s) => intern_long(s),
            None => throw_overflow(),
        },
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x * y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 * y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x * *y as f64)),
        _ => rt_const_nil(),
    }
}

/// Raise an exception carrying `msg` via the pending-exception slot and return
/// nil (used by the array bridges for type and bounds errors).
fn throw_str(msg: String) -> *const Value {
    let exc = box_val(Value::Error(GcPtr::new(
        cljrs_value::error::ExceptionInfo::new(
            cljrs_value::ValueError::Other(msg.clone()),
            msg,
            None,
            None,
        ),
    )));
    unsafe { rt_throw(exc) }
}

/// Raise an out-of-bounds condition carrying the structured
/// `ValueError::IndexOutOfBounds` variant, matching the tree-walk interpreter's
/// `aget`/`aset`/`nth` errors so a caught value has the same shape and message
/// (`index out of bounds: {idx} >= {len}`) regardless of execution tier.
fn throw_index_oob(idx: i64, len: usize) -> *const Value {
    let err = if idx >= 0 {
        cljrs_value::ValueError::IndexOutOfBounds {
            idx: idx as usize,
            count: len,
        }
    } else {
        cljrs_value::ValueError::Other(format!("index out of bounds: {idx} >= {len}"))
    };
    let msg = err.to_string();
    let exc = box_val(Value::Error(GcPtr::new(
        cljrs_value::error::ExceptionInfo::new(err, msg, None, None),
    )));
    unsafe { rt_throw(exc) }
}

// ── Primitive array access ──────────────────────────────────────────────────

/// `(alength arr)` — element count of any primitive/object array, unboxed.
/// Throws on a non-array argument.
///
/// # Safety
/// `arr` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alength(arr: *const Value) -> i64 {
    let arr = unsafe { val_ref(arr) };
    let len = match arr {
        Value::ObjectArray(a) => a.get().0.lock().unwrap().len(),
        Value::IntArray(a) => a.get().lock().unwrap().len(),
        Value::LongArray(a) => a.get().lock().unwrap().len(),
        Value::ShortArray(a) => a.get().lock().unwrap().len(),
        Value::ByteArray(a) => a.get().lock().unwrap().len(),
        Value::FloatArray(a) => a.get().lock().unwrap().len(),
        Value::DoubleArray(a) => a.get().lock().unwrap().len(),
        Value::BooleanArray(a) => a.get().lock().unwrap().len(),
        Value::CharArray(a) => a.get().lock().unwrap().len(),
        _ => {
            throw_str(format!("alength: not an array: {}", arr.type_name()));
            return 0;
        }
    };
    len as i64
}

/// `(aget longs i)` — unboxed `i64` element load from a `LongArray`.
///
/// # Safety
/// `arr` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aget_long(arr: *const Value, i: i64) -> i64 {
    let arr = unsafe { val_ref(arr) };
    match arr {
        Value::LongArray(a) => {
            let v = a.get().lock().unwrap();
            if i < 0 || i as usize >= v.len() {
                throw_index_oob(i, v.len());
                return 0;
            }
            v[i as usize]
        }
        _ => {
            throw_str(format!("aget: not a long array: {}", arr.type_name()));
            0
        }
    }
}

/// `(aget doubles i)` — unboxed `f64` element load from a `DoubleArray`.
///
/// # Safety
/// `arr` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aget_double(arr: *const Value, i: i64) -> f64 {
    let arr = unsafe { val_ref(arr) };
    match arr {
        Value::DoubleArray(a) => {
            let v = a.get().lock().unwrap();
            if i < 0 || i as usize >= v.len() {
                throw_index_oob(i, v.len());
                return 0.0;
            }
            v[i as usize]
        }
        _ => {
            throw_str(format!("aget: not a double array: {}", arr.type_name()));
            0.0
        }
    }
}

/// `(aset longs i v)` — unboxed `i64` element store into a `LongArray`,
/// returning the boxed stored value.
///
/// # Safety
/// `arr` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aset_long(arr: *const Value, i: i64, v: i64) -> *const Value {
    let arr = unsafe { val_ref(arr) };
    match arr {
        Value::LongArray(a) => {
            let mut g = a.get().lock().unwrap();
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            g[i as usize] = v;
            intern_long(v)
        }
        _ => throw_str(format!("aset: not a long array: {}", arr.type_name())),
    }
}

/// `(aset doubles i v)` — unboxed `f64` element store into a `DoubleArray`.
///
/// # Safety
/// `arr` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aset_double(arr: *const Value, i: i64, v: f64) -> *const Value {
    let arr = unsafe { val_ref(arr) };
    match arr {
        Value::DoubleArray(a) => {
            let mut g = a.get().lock().unwrap();
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            g[i as usize] = v;
            box_val(Value::Double(v))
        }
        _ => throw_str(format!("aset: not a double array: {}", arr.type_name())),
    }
}

/// `(aget arr i)` — boxed single-dimension element load for any array type
/// (used when the array's element type is not statically known).
///
/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aget(arr: *const Value, idx: *const Value) -> *const Value {
    let arr = unsafe { val_ref(arr) };
    let idx = unsafe { val_ref(idx) };
    let i = match idx {
        Value::Long(n) => *n,
        _ => return throw_str("aget: index must be an integer".to_string()),
    };
    macro_rules! load {
        ($a:expr, $map:expr) => {{
            let g = $a.get().lock().unwrap();
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            #[allow(clippy::redundant_closure_call)]
            box_val($map(&g[i as usize]))
        }};
    }
    match arr {
        Value::ObjectArray(a) => {
            let g = a.get().0.lock().unwrap();
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            box_val(g[i as usize].clone())
        }
        Value::LongArray(a) => load!(a, |v: &i64| Value::Long(*v)),
        Value::IntArray(a) => load!(a, |v: &i32| Value::Long(*v as i64)),
        Value::ShortArray(a) => load!(a, |v: &i16| Value::Long(*v as i64)),
        Value::ByteArray(a) => load!(a, |v: &i8| Value::Long(*v as i64)),
        Value::DoubleArray(a) => load!(a, |v: &f64| Value::Double(*v)),
        Value::FloatArray(a) => load!(a, |v: &f32| Value::Double(*v as f64)),
        Value::BooleanArray(a) => load!(a, |v: &bool| Value::Bool(*v)),
        Value::CharArray(a) => load!(a, |v: &char| Value::Char(*v)),
        _ => throw_str(format!("aget: not an array: {}", arr.type_name())),
    }
}

/// `(aset arr i v)` — boxed element store for any array type.
///
/// # Safety
/// All pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_aset(
    arr: *const Value,
    idx: *const Value,
    val: *const Value,
) -> *const Value {
    let arr_v = unsafe { val_ref(arr) };
    let idx_v = unsafe { val_ref(idx) };
    let val_v = unsafe { val_ref(val) };
    let i = match idx_v {
        Value::Long(n) => *n,
        _ => return throw_str("aset: index must be an integer".to_string()),
    };
    macro_rules! store {
        ($lock:expr, $conv:expr) => {{
            let mut g = $lock;
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            match $conv(val_v) {
                Some(c) => {
                    g[i as usize] = c;
                    return val;
                }
                None => return throw_str("aset: value type does not match array".to_string()),
            }
        }};
    }
    match arr_v {
        Value::ObjectArray(a) => {
            let mut g = a.get().0.lock().unwrap();
            if i < 0 || i as usize >= g.len() {
                return throw_index_oob(i, g.len());
            }
            g[i as usize] = val_v.clone();
            val
        }
        Value::LongArray(a) => store!(a.get().lock().unwrap(), |v: &Value| match v {
            Value::Long(n) => Some(*n),
            _ => None,
        }),
        Value::DoubleArray(a) => store!(a.get().lock().unwrap(), |v: &Value| match v {
            Value::Double(f) => Some(*f),
            Value::Long(n) => Some(*n as f64),
            _ => None,
        }),
        Value::IntArray(a) => store!(a.get().lock().unwrap(), |v: &Value| match v {
            Value::Long(n) => Some(*n as i32),
            _ => None,
        }),
        _ => throw_str(format!(
            "aset: unsupported array type: {}",
            arr_v.type_name()
        )),
    }
}

/// Unchecked (wrapping) boxed long arithmetic — the `unchecked-*` family.
/// Never throws or promotes; wraps on overflow.
///
/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_unchecked_add(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => intern_long(x.wrapping_add(*y)),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x + y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 + y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x + *y as f64)),
        _ => rt_const_nil(),
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_unchecked_sub(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => intern_long(x.wrapping_sub(*y)),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x - y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 - y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x - *y as f64)),
        _ => rt_const_nil(),
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_unchecked_mul(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => intern_long(x.wrapping_mul(*y)),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x * y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 * y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x * *y as f64)),
        _ => rt_const_nil(),
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_div(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) if *y != 0 => intern_long(x / y),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x / y)),
        (Value::Long(x), Value::Double(y)) => box_val(Value::Double(*x as f64 / y)),
        (Value::Double(x), Value::Long(y)) => box_val(Value::Double(x / *y as f64)),
        _ => rt_const_nil(),
    }
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_rem(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) if *y != 0 => intern_long(x % y),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x % y)),
        _ => rt_const_nil(),
    }
}

// ── Comparison ──────────────────────────────────────────────────────────────

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_eq(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    intern_bool(a == b)
}

/// Type-strict equality for the `case` macro: Long/BigInt are interchangeable;
/// mixed numeric types (e.g. Long vs Double) are never equal; non-numeric types
/// use regular structural equality.
///
/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_case_eq(a: *const Value, b: *const Value) -> *const Value {
    let a_val = unsafe { val_ref(a) };
    let b_val = unsafe { val_ref(b) };
    let a_bare = a_val.unwrap_meta();
    let b_bare = b_val.unwrap_meta();
    let result = match (a_bare, b_bare) {
        // Long and BigInt are interchangeable in case (Clojure JVM behavior).
        (Value::Long(_) | Value::BigInt(_), Value::Long(_) | Value::BigInt(_)) => a_val == b_val,
        (Value::Double(_), Value::Double(_)) => a_val == b_val,
        (Value::BigDecimal(_), Value::BigDecimal(_)) => a_val == b_val,
        (Value::Ratio(_), Value::Ratio(_)) => a_val == b_val,
        // Mixed numeric types are never equal in case dispatch.
        (
            Value::Long(_)
            | Value::BigInt(_)
            | Value::Double(_)
            | Value::BigDecimal(_)
            | Value::Ratio(_),
            Value::Long(_)
            | Value::BigInt(_)
            | Value::Double(_)
            | Value::BigDecimal(_)
            | Value::Ratio(_),
        ) => false,
        // Non-numeric types use regular equality.
        _ => a_val == b_val,
    };
    intern_bool(result)
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lt(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x < y,
        (Value::Double(x), Value::Double(y)) => x < y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) < *y,
        (Value::Double(x), Value::Long(y)) => *x < (*y as f64),
        _ => false,
    };
    intern_bool(result)
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_gt(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x > y,
        (Value::Double(x), Value::Double(y)) => x > y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) > *y,
        (Value::Double(x), Value::Long(y)) => *x > (*y as f64),
        _ => false,
    };
    intern_bool(result)
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lte(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x <= y,
        (Value::Double(x), Value::Double(y)) => x <= y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) <= *y,
        (Value::Double(x), Value::Long(y)) => *x <= (*y as f64),
        _ => false,
    };
    intern_bool(result)
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_gte(a: *const Value, b: *const Value) -> *const Value {
    bump_boxed_arith();
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x >= y,
        (Value::Double(x), Value::Double(y)) => x >= y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) >= *y,
        (Value::Double(x), Value::Long(y)) => *x >= (*y as f64),
        _ => false,
    };
    intern_bool(result)
}

// ── Region allocation ───────────────────────────────────────────────────────

use cljrs_gc::region::Region;
use std::cell::RefCell;

thread_local! {
    /// Stack of boxed regions owned by the rt_abi layer.
    /// Paired with entries on `cljrs_gc::region::REGION_STACK`.
    /// Box is intentional: we hand out raw pointers via `push_region_raw`,
    /// so the Region must not move if the Vec grows.
    #[allow(clippy::vec_box)]
    static RT_REGION_STACK: RefCell<Vec<Box<Region>>> =
        const { RefCell::new(Vec::new()) };
}

/// Allocate `val` directly into the region named by `handle`, falling back to
/// the active thread-local region (then the GC heap) when `handle` is null.
///
/// A null handle is never produced by compiled `RegionStart`/`RegionParam`
/// code today; the fallback keeps the bridge total should a future caller
/// pass one.
#[inline]
fn region_alloc_box<T: cljrs_gc::Trace + 'static>(handle: *mut Region, val: T) -> GcPtr<T> {
    if handle.is_null() {
        alloc_inner_coll(val)
    } else {
        // SAFETY: `handle` was produced by `rt_region_start` (or arrived as
        // the hidden region parameter of a region-parameterised variant) and
        // stays alive until the matching `rt_region_end`.
        unsafe { (*handle).alloc(val) }
    }
}

/// Box a collection `Value` into the region named by `handle` (same fallback
/// rules as [`region_alloc_box`]).
#[inline]
fn region_box_val(handle: *mut Region, v: Value) -> *const Value {
    if handle.is_null() {
        box_coll_val(v)
    } else {
        // SAFETY: see `region_alloc_box`.
        let ptr: GcPtr<Value> = unsafe { (*handle).alloc(v) };
        ptr.get() as *const Value
    }
}

/// Begin a region scope — allocates a new bump region and activates it.
///
/// Returns the region pointer.  Compiled code passes it back to
/// `rt_region_alloc_*` / `rt_region_end`, and threads it into
/// region-parameterised callees as a hidden trailing argument
/// (`CallWithRegion`).  The region is *also* pushed onto the thread-local
/// region stack so opportunistic rt_abi allocation (`box_coll_val`) and GC
/// root tracing see it.
#[unsafe(no_mangle)]
pub extern "C" fn rt_region_start() -> *mut Region {
    let mut region = Box::new(Region::new());
    // SAFETY: the Region lives in the Box on RT_REGION_STACK until
    // rt_region_end pops it.
    let region_ptr: *mut Region = &mut *region;
    unsafe { cljrs_gc::region::push_region_raw(region_ptr) };
    RT_REGION_STACK.with(|s| s.borrow_mut().push(region));
    region_ptr
}

/// End a region scope — pops the region, runs destructors, frees memory.
///
/// Returns nil.
///
/// # Safety
/// Must be paired with a prior `rt_region_start` call; `handle` must be that
/// call's return value.
#[unsafe(no_mangle)]
pub extern "C" fn rt_region_end(handle: *mut Region) -> *const Value {
    let popped = RT_REGION_STACK.with(|s| s.borrow_mut().pop());
    debug_assert!(
        popped
            .as_ref()
            .map(|r| std::ptr::eq(r.as_ref(), handle.cast_const()))
            .unwrap_or(false),
        "rt_region_end: handle does not match the innermost open region"
    );
    if let Some(region) = popped {
        // Pops the thread-local stack entry, then resets the region — or
        // retires it if a publish barrier poisoned it (Phase 10.5
        // heap-promotion fallback).
        cljrs_gc::region::close_region(region);
    }
    rt_const_nil()
}

/// Allocate a vector directly into the region named by `handle`.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers; `handle` must be
/// a live region (see [`region_alloc_box`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_region_alloc_vector(
    handle: *mut Region,
    elems: *const *const Value,
    n: u64,
) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let pv = PersistentVector::from_iter(items);
    let ptr = region_alloc_box(handle, pv);
    region_box_val(handle, Value::Vector(ptr))
}

/// Allocate a map directly into the region named by `handle`.
///
/// # Safety
/// `pairs` must point to `2*n` valid `*const Value` pointers; `handle` must
/// be a live region (see [`region_alloc_box`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_region_alloc_map(
    handle: *mut Region,
    pairs: *const *const Value,
    n: u64,
) -> *const Value {
    let n = n as usize;
    let kv_pairs: Vec<(Value, Value)> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(pairs, n * 2) };
        (0..n)
            .map(|i| {
                let k = unsafe { val_ref(slice[i * 2]) }.clone();
                let v = unsafe { val_ref(slice[i * 2 + 1]) }.clone();
                (k, v)
            })
            .collect()
    } else {
        Vec::new()
    };
    region_box_val(handle, Value::Map(MapValue::from_pairs(kv_pairs)))
}

/// Allocate a set directly into the region named by `handle`.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers; `handle` must be
/// a live region (see [`region_alloc_box`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_region_alloc_set(
    handle: *mut Region,
    elems: *const *const Value,
    n: u64,
) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let set = PersistentHashSet::from_iter(items);
    let ptr = region_alloc_box(handle, set);
    region_box_val(handle, Value::Set(SetValue::Hash(ptr)))
}

/// Allocate a list directly into the region named by `handle`.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers; `handle` must be
/// a live region (see [`region_alloc_box`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_region_alloc_list(
    handle: *mut Region,
    elems: *const *const Value,
    n: u64,
) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let list = PersistentList::from_iter(items);
    let ptr = region_alloc_box(handle, list);
    region_box_val(handle, Value::List(ptr))
}

/// Allocate a cons cell directly into the region named by `handle`.
///
/// # Safety
/// Both value pointers must be valid; `handle` must be a live region (see
/// [`region_alloc_box`]).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_region_alloc_cons(
    handle: *mut Region,
    head: *const Value,
    tail: *const Value,
) -> *const Value {
    let h = unsafe { val_ref(head) }.clone();
    let t = unsafe { val_ref(tail) }.clone();
    let cons = CljxCons { head: h, tail: t };
    let ptr = region_alloc_box(handle, cons);
    region_box_val(handle, Value::Cons(ptr))
}

/// Unwind both region stacks to the given depths on exception.
///
/// Called by `rt_try` to clean up regions that were opened inside a try body
/// that threw.  The two depths are saved (and restored) independently: the
/// gc-side stack can also hold regions pushed by the IR interpreter or the
/// top-level form scope, so its depth is not in general equal to the rt_abi
/// stack's depth.
fn unwind_regions_to(gc_depth: usize, rt_depth: usize) {
    loop {
        let popped = RT_REGION_STACK.with(|s| {
            let mut stack = s.borrow_mut();
            if stack.len() > rt_depth {
                stack.pop()
            } else {
                None
            }
        });
        match popped {
            // Honours the poison protocol (reset vs retire) and pops the
            // matching gc-side stack entry.
            Some(region) => cljrs_gc::region::close_region(region),
            None => break,
        }
    }
    // Belt-and-braces: drop any non-rt entries left above the saved gc depth
    // (interpreter frames clean their own regions on unwind, so this is
    // normally a no-op).
    cljrs_gc::region::unwind_region_stack_to(gc_depth);
}

// ── Scratch buffer ──────────────────────────────────────────────────────────

thread_local! {
    /// A reusable, monotonically growing scratch buffer.  The wasm AOT backend
    /// marshals a contiguous array of element `*const Value` pointers here
    /// before calling the slice-taking `rt_alloc_*` / `rt_region_alloc_*`
    /// bridges (the native backend uses an on-stack slot instead).
    static RT_SCRATCH: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
}

/// Return a pointer to a thread-local scratch buffer at least `n_bytes` wide.
///
/// The buffer is reused across calls and grows monotonically; the returned
/// pointer is valid only until the next `rt_scratch_ptr` call on the same
/// thread.  Only the wasm backend uses this; the native backend marshals
/// element arrays in an explicit stack slot.
#[unsafe(no_mangle)]
pub extern "C" fn rt_scratch_ptr(n_bytes: u32) -> *mut u8 {
    RT_SCRATCH.with(|s| {
        let mut buf = s.borrow_mut();
        if buf.len() < n_bytes as usize {
            buf.resize(n_bytes as usize, 0);
        }
        buf.as_mut_ptr()
    })
}

// ── Collection construction ─────────────────────────────────────────────────

/// Allocate a vector from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers, or may be null
/// when `n` is 0 (codegen passes null for empty literals).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_vector(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let pv = PersistentVector::from_iter(items);
    box_coll_val(Value::Vector(alloc_inner_coll(pv)))
}

/// Allocate a map from `n` key-value pairs (2*n pointers: k0, v0, k1, v1, ...).
///
/// # Safety
/// `pairs` must point to `2*n` valid `*const Value` pointers, or may be null
/// when `n` is 0 (codegen passes null for empty literals).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_map(pairs: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let kv_pairs: Vec<(Value, Value)> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(pairs, n * 2) };
        (0..n)
            .map(|i| {
                let k = unsafe { val_ref(slice[i * 2]) }.clone();
                let v = unsafe { val_ref(slice[i * 2 + 1]) }.clone();
                (k, v)
            })
            .collect()
    } else {
        Vec::new()
    };
    box_coll_val(Value::Map(MapValue::from_pairs(kv_pairs)))
}

/// Allocate a set from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers, or may be null
/// when `n` is 0 (codegen passes null for empty literals).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_set(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let set = PersistentHashSet::from_iter(items);
    box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(set))))
}

/// Allocate a list from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers, or may be null
/// when `n` is 0 (codegen passes null for empty literals).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_list(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let items: Vec<Value> = if n > 0 {
        let slice = unsafe { std::slice::from_raw_parts(elems, n) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        Vec::new()
    };
    let list = PersistentList::from_iter(items);
    box_coll_val(Value::List(alloc_inner_coll(list)))
}

/// Allocate a cons cell.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_cons(head: *const Value, tail: *const Value) -> *const Value {
    let h = unsafe { val_ref(head) }.clone();
    let t = unsafe { val_ref(tail) }.clone();
    box_coll_val(Value::Cons(alloc_inner_coll(CljxCons { head: h, tail: t })))
}

// ── Collection operations ───────────────────────────────────────────────────

/// `(get coll key)` — returns nil for not-found.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_get(coll: *const Value, key: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let key = unsafe { val_ref(key) };
    let result = match coll {
        Value::Map(m) => m.get(key),
        Value::TypeInstance(ti) => ti.get().fields.get(key),
        Value::Vector(v) => {
            if let Value::Long(i) = key {
                v.get().nth(*i as usize).cloned()
            } else {
                None
            }
        }
        _ => None,
    };
    box_or_intern_val(result.unwrap_or(Value::Nil))
}

/// `(count coll)` — returns a Long.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_count(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let n = match coll {
        Value::Vector(v) => v.get().count(),
        Value::Map(m) => m.count(),
        Value::Set(s) => s.count(),
        Value::List(l) => l.get().count(),
        Value::Str(s) => s.get().len(),
        Value::Nil => 0,
        // Lazy/cons seqs — e.g. the result of `filter`/`map`/`mapcat` — must
        // be walked and realized to be counted.  Without this arm `count`
        // returns 0 for every lazy seq, silently corrupting any
        // `(count (filter …))` computation.
        Value::Cons(_) | Value::LazySeq(_) => {
            let mut count = 0usize;
            let mut current = coll.clone();
            loop {
                match current {
                    Value::Nil => break,
                    Value::Cons(c) => {
                        count += 1;
                        current = c.get().tail.clone();
                    }
                    Value::LazySeq(ls) => {
                        current = ls.get().realize();
                    }
                    Value::List(l) => {
                        count += l.get().count();
                        break;
                    }
                    Value::Vector(v) => {
                        count += v.get().count();
                        break;
                    }
                    _ => break,
                }
            }
            count
        }
        _ => 0,
    };
    intern_long(n as i64)
}

/// `(count (filter pred coll))` fused — count matching elements without
/// materializing the intermediate sequence.
///
/// `Set`/`Map` predicates are tested directly; other predicates are `invoke`d
/// per element.  Iterates concrete collections in compiled Rust; falls back to
/// the interpreted lazy `filter` + `count` for inputs `eager_seq_elems` can't
/// walk directly.  Allocates nothing in the common path — the key win over the
/// lazy path, whose per-element cons cells dominate `samples/life.cljrs`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_count_filter(pred: *const Value, coll: *const Value) -> *const Value {
    let pred_ref = unsafe { val_ref(pred) };
    let coll_ref = unsafe { val_ref(coll) };
    let Some(elems) = eager_seq_elems(coll_ref) else {
        // Fall back to the interpreter for lazy/cons inputs, then count.
        let pred_val = pred_ref.clone();
        let coll_val = coll_ref.clone();
        let seq = call_global_fn("clojure.core", "filter", vec![pred_val, coll_val]);
        return unsafe { rt_count(seq) };
    };
    let mut n: usize = 0;
    for elem in elems {
        let keep = match pred_ref {
            Value::Set(s) => s.contains(&elem),
            Value::Map(m) => m
                .get(&elem)
                .map(|v| !matches!(v, Value::Nil | Value::Bool(false)))
                .unwrap_or(false),
            _ => match cljrs_env::callback::invoke(pred_ref, vec![elem]) {
                Ok(r) => !matches!(r, Value::Nil | Value::Bool(false)),
                Err(cljrs_value::ValueError::Thrown(val)) => {
                    stash_pending_exception(val);
                    return rt_const_nil();
                }
                Err(_) => return rt_const_nil(),
            },
        };
        if keep {
            n += 1;
        }
    }
    intern_long(n as i64)
}

/// Eagerly collect the elements of a directly-iterable collection, or `None`
/// for inputs that need the interpreter's full `seq` machinery.
fn eager_seq_elems(coll: &Value) -> Option<Vec<Value>> {
    match coll {
        Value::Vector(v) => Some(v.get().iter().cloned().collect()),
        Value::Set(s) => Some(s.iter().cloned().collect()),
        Value::List(l) => Some(l.get().iter().cloned().collect()),
        Value::Nil => Some(Vec::new()),
        _ => None,
    }
}

/// Store a thrown value in the pending-exception slot (mirrors `rt_call`).
#[inline]
fn stash_pending_exception(val: Value) {
    PENDING_EXCEPTION.with(|cell| {
        *cell.borrow_mut() = Some(box_val(val));
    });
}

/// Apply a filter predicate to `elem`.  Returns `Some(keep)`, or `None` if the
/// predicate threw (exception already stashed).  `Set`/`Map` predicates are
/// tested directly; others are `invoke`d.
#[inline]
fn pred_truthy(pred: &Value, elem: &Value) -> Option<bool> {
    match pred {
        Value::Set(s) => Some(s.contains(elem)),
        Value::Map(m) => Some(
            m.get(elem)
                .map(|v| !matches!(v, Value::Nil | Value::Bool(false)))
                .unwrap_or(false),
        ),
        _ => match cljrs_env::callback::invoke(pred, vec![elem.clone()]) {
            Ok(r) => Some(!matches!(r, Value::Nil | Value::Bool(false))),
            Err(cljrs_value::ValueError::Thrown(val)) => {
                stash_pending_exception(val);
                None
            }
            Err(_) => None,
        },
    }
}

/// `(into to (filter pred coll))` fused — eager, no intermediate lazy seq.
///
/// Iterates `coll`, conj-ing matching elements straight into `to`.  Set and
/// Vector targets are built natively; other targets fall back to interpreted
/// `into` over an eagerly-filtered vector.  Avoids the interpreted lazy
/// `filter` + `into` realization that dominates `samples/life.cljrs`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_into_filter(
    to: *const Value,
    pred: *const Value,
    coll: *const Value,
) -> *const Value {
    let to_ref = unsafe { val_ref(to) };
    let pred_ref = unsafe { val_ref(pred) };
    let coll_ref = unsafe { val_ref(coll) };
    let Some(elems) = eager_seq_elems(coll_ref) else {
        let to_val = to_ref.clone();
        let pred_val = pred_ref.clone();
        let coll_val = coll_ref.clone();
        let seq = call_global_fn("clojure.core", "filter", vec![pred_val, coll_val]);
        let seq_val = unsafe { val_ref(seq) }.clone();
        return call_global_fn("clojure.core", "into", vec![to_val, seq_val]);
    };
    match to_ref {
        Value::Set(SetValue::Hash(set_ptr)) => {
            let mut s = (*set_ptr.get()).clone();
            for elem in elems {
                match pred_truthy(pred_ref, &elem) {
                    Some(true) => {
                        s.conj_mut(elem);
                    }
                    Some(false) => {}
                    None => return rt_const_nil(),
                }
            }
            box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(s))))
        }
        Value::Vector(v) => {
            let mut r = v.get().clone();
            for elem in elems {
                match pred_truthy(pred_ref, &elem) {
                    Some(true) => r = r.conj(elem),
                    Some(false) => {}
                    None => return rt_const_nil(),
                }
            }
            box_coll_val(Value::Vector(alloc_inner_coll(r)))
        }
        _ => {
            let mut kept = Vec::new();
            for elem in elems {
                match pred_truthy(pred_ref, &elem) {
                    Some(true) => kept.push(elem),
                    Some(false) => {}
                    None => return rt_const_nil(),
                }
            }
            let to_val = to_ref.clone();
            let src = Value::Vector(alloc_inner_coll(PersistentVector::from_iter(kept)));
            call_global_fn("clojure.core", "into", vec![to_val, src])
        }
    }
}

/// `(into to (mapcat f coll))` fused — eager, no intermediate lazy seq.
///
/// For each element of `coll`, calls `f` (which must return a collection) and
/// conj-es its elements into `to`.  Set and Vector targets are built natively.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_into_mapcat(
    to: *const Value,
    f: *const Value,
    coll: *const Value,
) -> *const Value {
    let to_ref = unsafe { val_ref(to) };
    let f_ref = unsafe { val_ref(f) };
    let coll_ref = unsafe { val_ref(coll) };

    // Fall back to interpreted `mapcat` + `into` if any input isn't directly
    // iterable.
    let fallback = || {
        let to_val = to_ref.clone();
        let f_val = f_ref.clone();
        let coll_val = coll_ref.clone();
        let seq = call_global_fn("clojure.core", "mapcat", vec![f_val, coll_val]);
        let seq_val = unsafe { val_ref(seq) }.clone();
        call_global_fn("clojure.core", "into", vec![to_val, seq_val])
    };

    let Some(elems) = eager_seq_elems(coll_ref) else {
        return fallback();
    };
    let mut elements: Vec<Value> = Vec::new();
    for elem in elems {
        let r = match cljrs_env::callback::invoke(f_ref, vec![elem]) {
            Ok(v) => v,
            Err(cljrs_value::ValueError::Thrown(val)) => {
                stash_pending_exception(val);
                return rt_const_nil();
            }
            Err(_) => return rt_const_nil(),
        };
        match eager_seq_elems(&r) {
            Some(inner) => elements.extend(inner),
            // `f` returned a non-directly-iterable collection: bail out.  (`f`
            // is pure in the fused patterns we target, so re-running it in the
            // fallback is safe.)
            None => return fallback(),
        }
    }

    match to_ref {
        Value::Set(SetValue::Hash(set_ptr)) => {
            let mut s = (*set_ptr.get()).clone();
            for x in elements {
                s.conj_mut(x);
            }
            box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(s))))
        }
        Value::Vector(v) => {
            let mut r = v.get().clone();
            for x in elements {
                r = r.conj(x);
            }
            box_coll_val(Value::Vector(alloc_inner_coll(r)))
        }
        _ => {
            let to_val = to_ref.clone();
            let src = Value::Vector(alloc_inner_coll(PersistentVector::from_iter(elements)));
            call_global_fn("clojure.core", "into", vec![to_val, src])
        }
    }
}

/// Eagerly realize the elements of any seqable `coll` — including lazy
/// `Cons`/`LazySeq` chains such as `range`/`map` results — into a `Vec`.
/// Returns `None` for genuinely non-seqable values so the caller can fall
/// back to the interpreter.  Unlike [`eager_seq_elems`], this forces lazy
/// thunks; it is used where the caller is an eager consumer that would fully
/// realize the source anyway.
fn realize_seq_elems(coll: &Value) -> Option<Vec<Value>> {
    match coll {
        Value::Vector(v) => Some(v.get().iter().cloned().collect()),
        Value::Set(s) => Some(s.iter().cloned().collect()),
        Value::List(l) => Some(l.get().iter().cloned().collect()),
        Value::Nil => Some(Vec::new()),
        Value::Cons(_) | Value::LazySeq(_) => {
            let mut out = Vec::new();
            let mut current = coll.clone();
            loop {
                match current {
                    Value::Nil => break,
                    Value::Cons(c) => {
                        out.push(c.get().head.clone());
                        current = c.get().tail.clone();
                    }
                    Value::LazySeq(ls) => {
                        current = ls.get().realize();
                    }
                    Value::List(l) => {
                        out.extend(l.get().iter().cloned());
                        break;
                    }
                    Value::Vector(v) => {
                        out.extend(v.get().iter().cloned());
                        break;
                    }
                    _ => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// `(into to (map f coll))` fused — eager, region-aware, and realizes lazy
/// `coll` sources (e.g. `range`) natively.  Because the minimal `for` macro
/// expands to `map`, this is the fused form of the `(into {} (for [x coll]
/// [k v]))` map-comprehension idiom that dominates `samples/graph.cljrs`.
///
/// Applies `f` to each element and builds the target directly: map targets via
/// `MapValue::from_pairs` (last-wins, size-optimal), set/vector targets via
/// `conj`.  Falls back to interpreted `map` + `into` for inputs it can't walk
/// (non-seqable `coll`) or targets it doesn't build natively.  `f` is invoked
/// at most once per element; the map-target "not a pair" case routes the
/// already-mapped values through interpreted `into` rather than re-running `f`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_into_map(
    to: *const Value,
    f: *const Value,
    coll: *const Value,
) -> *const Value {
    let to_ref = unsafe { val_ref(to) };
    let f_ref = unsafe { val_ref(f) };
    let coll_ref = unsafe { val_ref(coll) };

    let Some(elems) = realize_seq_elems(coll_ref) else {
        let to_val = to_ref.clone();
        let f_val = f_ref.clone();
        let coll_val = coll_ref.clone();
        let seq = call_global_fn("clojure.core", "map", vec![f_val, coll_val]);
        let seq_val = unsafe { val_ref(seq) }.clone();
        return call_global_fn("clojure.core", "into", vec![to_val, seq_val]);
    };

    let mut mapped: Vec<Value> = Vec::with_capacity(elems.len());
    for elem in elems {
        match cljrs_env::callback::invoke(f_ref, vec![elem]) {
            Ok(v) => mapped.push(v),
            Err(cljrs_value::ValueError::Thrown(val)) => {
                stash_pending_exception(val);
                return rt_const_nil();
            }
            Err(_) => return rt_const_nil(),
        }
    }

    match to_ref {
        Value::Map(to_map) => {
            // Existing entries first so the mapped pairs win on key conflict.
            let mut pairs: Vec<(Value, Value)> =
                to_map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            let mut all_pairs = true;
            for m in &mapped {
                match as_pair(m) {
                    Some(p) => pairs.push(p),
                    None => {
                        all_pairs = false;
                        break;
                    }
                }
            }
            if all_pairs {
                box_coll_val(Value::Map(MapValue::from_pairs(pairs)))
            } else {
                // Mapped values aren't all key/value pairs — let interpreted
                // `into` handle map-entries/merging, without re-invoking `f`.
                let to_val = to_ref.clone();
                let src = Value::Vector(alloc_inner_coll(PersistentVector::from_iter(mapped)));
                call_global_fn("clojure.core", "into", vec![to_val, src])
            }
        }
        Value::Set(SetValue::Hash(set_ptr)) => {
            let mut s = (*set_ptr.get()).clone();
            for m in mapped {
                s.conj_mut(m);
            }
            box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(s))))
        }
        Value::Vector(v) => {
            let mut r = v.get().clone();
            for m in mapped {
                r = r.conj(m);
            }
            box_coll_val(Value::Vector(alloc_inner_coll(r)))
        }
        _ => {
            let to_val = to_ref.clone();
            let src = Value::Vector(alloc_inner_coll(PersistentVector::from_iter(mapped)));
            call_global_fn("clojure.core", "into", vec![to_val, src])
        }
    }
}

/// `(empty? coll)` — returns Bool without converting to a seq.
///
/// The KnownFn::IsEmpty codegen dispatches here so that BFS loops do not
/// pay the cost of `builtin_seq` (which copies the entire queue into a
/// cons-list on the GC heap) just to check emptiness.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_empty(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let empty = match coll {
        Value::Vector(v) => v.get().is_empty(),
        Value::Map(m) => m.count() == 0,
        Value::Set(s) => s.is_empty(),
        Value::List(l) => l.get().is_empty(),
        Value::Nil => true,
        Value::Cons(_) => false,
        _ => false,
    };
    intern_bool(empty)
}

/// `(first coll)`.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_first(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    match coll {
        // Return interior pointer for Vector — no alloc needed.
        Value::Vector(v) => match v.get().nth(0) {
            Some(val) => val as *const Value,
            None => intern_nil(),
        },
        Value::List(l) => box_or_intern_val(l.get().first().cloned().unwrap_or(Value::Nil)),
        Value::Cons(c) => box_or_intern_val(c.get().head.clone()),
        _ => intern_nil(),
    }
}

/// `(rest coll)`.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_rest(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    match coll {
        Value::List(l) => {
            let rest = (*l.get().rest()).clone();
            box_coll_val(Value::List(alloc_inner_coll(rest)))
        }
        Value::Vector(v) => {
            if v.get().count() <= 1 {
                box_coll_val(Value::List(alloc_inner_coll(PersistentList::empty())))
            } else {
                let items: Vec<Value> = v.get().iter().skip(1).cloned().collect();
                box_coll_val(Value::List(alloc_inner_coll(PersistentList::from_iter(
                    items,
                ))))
            }
        }
        Value::Cons(c) => box_coll_val(c.get().tail.clone()),
        _ => box_coll_val(Value::List(alloc_inner_coll(PersistentList::empty()))),
    }
}

/// `(assoc m k v)`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_assoc(
    m: *const Value,
    k: *const Value,
    v: *const Value,
) -> *const Value {
    let m = unsafe { val_ref(m) };
    let k = unsafe { val_ref(k) }.clone();
    let v = unsafe { val_ref(v) }.clone();
    match m {
        Value::Map(map) => {
            let new_map = map.assoc(k, v);
            box_coll_val(Value::Map(new_map))
        }
        Value::TypeInstance(ti) => {
            let mut fields = ti.get().fields.clone();
            fields = fields.assoc(k, v);
            box_coll_val(Value::TypeInstance(alloc_inner_coll(TypeInstance {
                type_tag: ti.get().type_tag.clone(),
                fields,
            })))
        }
        _ => rt_const_nil(),
    }
}

/// `(conj coll val)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_conj(coll: *const Value, val: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let val = unsafe { val_ref(val) }.clone();
    match coll {
        Value::Vector(v) => {
            let new_pv = v.get().conj(val);
            box_coll_val(Value::Vector(alloc_inner_coll(new_pv)))
        }
        Value::List(l) => {
            let new_list = PersistentList::cons(val, Arc::new((*l.get()).clone()));
            box_coll_val(Value::List(alloc_inner_coll(new_list)))
        }
        Value::Set(SetValue::Hash(m)) => {
            let new_phs = m.get().conj(val);
            box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(new_phs))))
        }
        Value::Set(s) => {
            // Sorted sets: delegate to interpreter (rare path)
            let new_set = s.conj(val);
            box_coll_val(Value::Set(new_set))
        }
        _ => rt_const_nil(),
    }
}

// ── Function/closure construction ───────────────────────────────────────────

/// Hook invoked whenever `rt_make_fn*` wraps a compiled function pointer into
/// a closure value.
///
/// Installed by `cljrs_jit::init`: the resulting `Value::NativeFunction` lives
/// on the GC heap and captures a raw pointer into the executing JIT module, so
/// the JIT pins that module's reclamation epoch.  Unset under AOT, where code
/// is never unloaded.
static CLOSURE_ESCAPE_HOOK: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// Install the closure-escape hook (installed once by `cljrs_jit::init`).
pub fn set_closure_escape_hook(f: fn()) {
    let _ = CLOSURE_ESCAPE_HOOK.set(f);
}

#[inline]
fn notify_closure_escape() {
    if let Some(hook) = CLOSURE_ESCAPE_HOOK.get() {
        hook();
    }
}

/// Create a `Value::NativeFunction` wrapping a compiled function pointer.
///
/// `fn_ptr` is a pointer to a compiled Cranelift function with signature:
///   `extern "C" fn(capture0, capture1, ..., arg0, arg1, ...) -> *const Value`
///
/// `param_count` is the number of user-visible parameters (excludes captures).
/// `captures` points to `ncaptures` `*const Value` pointers that are closed over.
///
/// The returned NativeFn, when called with `param_count` args, prepends the
/// captured values and calls `fn_ptr`.
///
/// # Safety
/// `name_ptr`/`name_len` must describe valid UTF-8.
/// `fn_ptr` must be a valid function pointer with the expected signature.
/// `captures` must point to `ncaptures` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_make_fn(
    name_ptr: *const u8,
    name_len: u64,
    fn_ptr: *const u8,
    param_count: u64,
    captures: *const *const Value,
    ncaptures: u64,
) -> *const Value {
    notify_closure_escape();
    let name_str = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let name: Arc<str> = Arc::from(name_str);
    let param_count = param_count as usize;
    let ncaptures = ncaptures as usize;

    // Clone captured values so they outlive this call.
    let captured_values: Vec<Value> = if ncaptures > 0 {
        let capture_slice = unsafe { std::slice::from_raw_parts(captures, ncaptures) };
        capture_slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        vec![]
    };

    // Store the raw function pointer as a usize for the closure.
    let fn_addr = fn_ptr as usize;
    let total_params = ncaptures + param_count;

    let native_fn = cljrs_value::NativeFn {
        name: name.clone(),
        arity: cljrs_value::Arity::Fixed(param_count),
        func: Arc::new(move |args: &[Value]| {
            if args.len() != param_count {
                return Err(cljrs_value::ValueError::ArityError {
                    name: "compiled-fn".to_string(),
                    expected: param_count.to_string(),
                    got: args.len(),
                });
            }

            // Build the full argument array: captures + args
            let mut all_ptrs: Vec<*const Value> = Vec::with_capacity(total_params);

            // Add captured values as pointers
            for cap in &captured_values {
                all_ptrs.push(box_val(cap.clone()));
            }

            // Add user args as pointers
            for arg in args {
                all_ptrs.push(box_val(arg.clone()));
            }

            // Call the compiled function.
            // The compiled function signature is:
            //   extern "C" fn(*const Value, *const Value, ...) -> *const Value
            // We call it via a trampoline that passes args through a pointer array.
            let result_ptr = unsafe { rt_call_compiled(fn_addr, all_ptrs.as_ptr(), total_params) };

            Ok(unsafe { val_ref(result_ptr) }.clone())
        }),
    };

    box_val(Value::NativeFunction(GcPtr::new(native_fn)))
}

/// Create a single-arity variadic compiled function value.
///
/// The compiled function takes `fixed_param_count` fixed params + 1 rest param (a list).
/// At call time, extra arguments beyond `fixed_param_count` are packed into a list.
///
/// # Safety
/// All pointer parameters must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_make_fn_variadic(
    name_ptr: *const u8,
    name_len: u64,
    fn_ptr: *const u8,
    fixed_param_count: u64,
    captures: *const *const Value,
    ncaptures: u64,
) -> *const Value {
    notify_closure_escape();
    let name_str = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let name: Arc<str> = Arc::from(name_str);
    let fixed_count = fixed_param_count as usize;
    let ncaptures = ncaptures as usize;

    let captured_values: Vec<Value> = if ncaptures > 0 {
        let capture_slice = unsafe { std::slice::from_raw_parts(captures, ncaptures) };
        capture_slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        vec![]
    };

    let fn_addr = fn_ptr as usize;
    // Compiled function receives: captures + fixed_params + 1 (rest list)
    let total_compiled_params = ncaptures + fixed_count + 1;

    let native_fn = cljrs_value::NativeFn {
        name: name.clone(),
        arity: cljrs_value::Arity::Variadic { min: fixed_count },
        func: Arc::new(move |args: &[Value]| {
            if args.len() < fixed_count {
                return Err(cljrs_value::ValueError::ArityError {
                    name: "compiled-fn".to_string(),
                    expected: format!("{fixed_count}+"),
                    got: args.len(),
                });
            }

            let mut all_ptrs: Vec<*const Value> = Vec::with_capacity(total_compiled_params);

            // Add captured values
            for cap in &captured_values {
                all_ptrs.push(box_val(cap.clone()));
            }

            // Add fixed args
            for arg in &args[..fixed_count] {
                all_ptrs.push(box_val(arg.clone()));
            }

            // Pack remaining args into a list for the rest parameter
            let rest_args: Vec<Value> = args[fixed_count..].to_vec();
            let rest_list = if rest_args.is_empty() {
                Value::Nil
            } else {
                Value::List(GcPtr::new(PersistentList::from_iter(rest_args)))
            };
            all_ptrs.push(box_val(rest_list));

            let result_ptr =
                unsafe { rt_call_compiled(fn_addr, all_ptrs.as_ptr(), total_compiled_params) };
            Ok(unsafe { val_ref(result_ptr) }.clone())
        }),
    };

    box_val(Value::NativeFunction(GcPtr::new(native_fn)))
}

/// Create a multi-arity compiled function value.
///
/// `fn_ptrs` is an array of `n_arities` function pointers (one per arity).
/// `param_counts` is an array of `n_arities` parameter counts (fixed user params, not including captures).
/// `is_variadic_ptr` is an array of `n_arities` booleans (1 = variadic, 0 = fixed).
/// The dispatch closure selects the right function pointer based on the argument count.
///
/// # Safety
/// All pointer parameters must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_make_fn_multi(
    name_ptr: *const u8,
    name_len: u64,
    fn_ptrs: *const *const u8,
    param_counts_ptr: *const u64,
    is_variadic_ptr: *const u8,
    n_arities: u64,
    captures: *const *const Value,
    ncaptures: u64,
) -> *const Value {
    notify_closure_escape();
    let name_str = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let name: Arc<str> = Arc::from(name_str);
    let n_arities = n_arities as usize;
    let ncaptures = ncaptures as usize;

    // Build arity table: Vec<(fn_addr, fixed_param_count, is_variadic)>
    let fn_ptr_slice = unsafe { std::slice::from_raw_parts(fn_ptrs, n_arities) };
    let param_count_slice = unsafe { std::slice::from_raw_parts(param_counts_ptr, n_arities) };
    let variadic_slice = unsafe { std::slice::from_raw_parts(is_variadic_ptr, n_arities) };
    let arity_table: Vec<(usize, usize, bool)> = fn_ptr_slice
        .iter()
        .zip(param_count_slice.iter())
        .zip(variadic_slice.iter())
        .map(|((&fp, &pc), &v)| (fp as usize, pc as usize, v != 0))
        .collect();

    // Clone captured values.
    let captured_values: Vec<Value> = if ncaptures > 0 {
        let capture_slice = unsafe { std::slice::from_raw_parts(captures, ncaptures) };
        capture_slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        vec![]
    };

    // Determine arity info for the NativeFn.
    let has_variadic = arity_table.iter().any(|(_, _, v)| *v);
    let min_params = arity_table.iter().map(|(_, pc, _)| *pc).min().unwrap_or(0);
    let max_params = arity_table.iter().map(|(_, pc, _)| *pc).max().unwrap_or(0);
    let arity = if has_variadic {
        cljrs_value::Arity::Variadic { min: min_params }
    } else if min_params == max_params {
        cljrs_value::Arity::Fixed(min_params)
    } else {
        cljrs_value::Arity::Variadic { min: min_params }
    };

    let fn_name = name.clone();
    let native_fn = cljrs_value::NativeFn {
        name,
        arity,
        func: Arc::new(move |args: &[Value]| {
            let argc = args.len();
            // Try exact match on fixed arities first.
            let matched = arity_table.iter().find(|(_, pc, v)| !v && *pc == argc);
            if let Some(&(fn_addr, _pc, _v)) = matched {
                let total_params = ncaptures + argc;
                let mut all_ptrs: Vec<*const Value> = Vec::with_capacity(total_params);
                for cap in &captured_values {
                    all_ptrs.push(box_val(cap.clone()));
                }
                for arg in args {
                    all_ptrs.push(box_val(arg.clone()));
                }
                let result_ptr =
                    unsafe { rt_call_compiled(fn_addr, all_ptrs.as_ptr(), total_params) };
                return Ok(unsafe { val_ref(result_ptr) }.clone());
            }

            // Try variadic arity (argc >= fixed_count).
            let variadic_match = arity_table.iter().find(|(_, pc, v)| *v && argc >= *pc);
            if let Some(&(fn_addr, fixed_count, _)) = variadic_match {
                // Compiled function receives: captures + fixed_params + 1 (rest list)
                let total_compiled = ncaptures + fixed_count + 1;
                let mut all_ptrs: Vec<*const Value> = Vec::with_capacity(total_compiled);
                for cap in &captured_values {
                    all_ptrs.push(box_val(cap.clone()));
                }
                for arg in &args[..fixed_count] {
                    all_ptrs.push(box_val(arg.clone()));
                }
                // Pack remaining args into a list
                let rest_args: Vec<Value> = args[fixed_count..].to_vec();
                let rest_list = if rest_args.is_empty() {
                    Value::Nil
                } else {
                    Value::List(GcPtr::new(PersistentList::from_iter(rest_args)))
                };
                all_ptrs.push(box_val(rest_list));
                let result_ptr =
                    unsafe { rt_call_compiled(fn_addr, all_ptrs.as_ptr(), total_compiled) };
                return Ok(unsafe { val_ref(result_ptr) }.clone());
            }

            // No matching arity found.
            let counts: Vec<String> = arity_table
                .iter()
                .map(
                    |(_, pc, v)| {
                        if *v { format!("{pc}+") } else { pc.to_string() }
                    },
                )
                .collect();
            Err(cljrs_value::ValueError::ArityError {
                name: fn_name.to_string(),
                expected: counts.join(" or "),
                got: argc,
            })
        }),
    };

    box_val(Value::NativeFunction(GcPtr::new(native_fn)))
}

/// Call a compiled function by passing arguments through a pointer array.
///
/// This is a trampoline: the compiled function expects individual pointer-sized
/// arguments, but we have them in an array. We dispatch based on the argument
/// count (up to a reasonable maximum).
///
/// # Safety
/// `fn_addr` must be a valid function pointer. `args` must point to `nargs`
/// valid `*const Value` pointers.
unsafe fn rt_call_compiled(
    fn_addr: usize,
    args: *const *const Value,
    nargs: usize,
) -> *const Value {
    let args = unsafe { std::slice::from_raw_parts(args, nargs) };

    match nargs {
        0 => {
            let f: extern "C" fn() -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f()
        }
        1 => {
            let f: extern "C" fn(*const Value) -> *const Value =
                unsafe { std::mem::transmute(fn_addr) };
            f(args[0])
        }
        2 => {
            let f: extern "C" fn(*const Value, *const Value) -> *const Value =
                unsafe { std::mem::transmute(fn_addr) };
            f(args[0], args[1])
        }
        3 => {
            let f: extern "C" fn(*const Value, *const Value, *const Value) -> *const Value =
                unsafe { std::mem::transmute(fn_addr) };
            f(args[0], args[1], args[2])
        }
        4 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(args[0], args[1], args[2], args[3])
        }
        5 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(args[0], args[1], args[2], args[3], args[4])
        }
        6 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(args[0], args[1], args[2], args[3], args[4], args[5])
        }
        7 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6],
            )
        }
        8 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            )
        }
        9 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
            )
        }
        10 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9],
            )
        }
        11 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10],
            )
        }
        12 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10], args[11],
            )
        }
        13 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10], args[11], args[12],
            )
        }
        14 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10], args[11], args[12], args[13],
            )
        }
        15 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10], args[11], args[12], args[13], args[14],
            )
        }
        16 => {
            let f: extern "C" fn(
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
                *const Value,
            ) -> *const Value = unsafe { std::mem::transmute(fn_addr) };
            f(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7], args[8],
                args[9], args[10], args[11], args[12], args[13], args[14], args[15],
            )
        }
        _ => {
            // The compiled-function trampoline supports up to a fixed maximum
            // arity (captures + params).  Exceeding it is rare but must never
            // silently corrupt results (the previous behaviour returned nil,
            // which produced wrong answers, e.g. a reduce over a let-bound
            // collection whose closure over-captures enclosing locals).  Log
            // loudly and surface a thrown error.  `total_params = ncaptures +
            // param_count`, so this only triggers for closures that capture an
            // unusually large number of enclosing locals.
            eprintln!(
                "[rt] error: compiled function arity {nargs} exceeds trampoline maximum (16)"
            );
            stash_pending_exception(Value::Str(GcPtr::new(format!(
                "compiled function arity {nargs} exceeds trampoline maximum (16)"
            ))));
            rt_const_nil()
        }
    }
}

// ── Global variable access ──────────────────────────────────────────────────

/// Load a global var by namespace and name.
///
/// # Safety
/// String pointers must be valid UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_load_global(
    ns_ptr: *const u8,
    ns_len: u64,
    name_ptr: *const u8,
    name_len: u64,
) -> *const Value {
    let ns = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize))
    };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };

    // Versioned reference (`name@<sha>`): resolve through the shared
    // versioned-namespace service.  (The IC bridge below is the fast path
    // for these; this covers any non-IC caller.)
    if let (base_name, Some(commit)) = cljrs_value::symbol::split_version(name) {
        return resolve_versioned_boxed(ns, base_name, commit);
    }

    // Look up in the global environment via the thread-local eval context.
    if let Some((globals, current_ns)) = cljrs_env::callback::capture_eval_context() {
        // Try the specified namespace first.
        if let Some(val) = globals.lookup_in_ns(ns, name) {
            return box_or_intern_val(val);
        }
        // Try resolving ns as an alias in the current namespace.
        if let Some(resolved_ns) = globals.resolve_alias(&current_ns, ns)
            && let Some(val) = globals.lookup_in_ns(&resolved_ns, name)
        {
            return box_or_intern_val(val);
        }
        // If ns is the current namespace, also check refers (e.g. clojure.core).
        if ns == current_ns.as_ref()
            && let Some(val) = globals.lookup_in_ns(&current_ns, name)
        {
            return box_or_intern_val(val);
        }
        // Reference into a versioned namespace (`lib@sha`/name) that has not
        // been loaded this session: load it lazily and retry.
        if let (base, Some(commit)) = cljrs_value::symbol::split_version(ns)
            && !globals.is_loaded(ns)
        {
            match cljrs_env::versioned::ensure_versioned_ns_loaded(&globals, base, commit) {
                Ok(_) => {
                    if let Some(val) = globals.lookup_in_ns(ns, name) {
                        return box_or_intern_val(val);
                    }
                }
                Err(e) => {
                    stash_pending_exception(Value::Str(GcPtr::new(format!("{e}"))));
                    return rt_const_nil();
                }
            }
        }
    }
    rt_const_nil()
}

/// Resolve `ns/name@commit` through the shared versioned resolver, boxing the
/// result.  Resolution failures surface as a pending exception (versioned
/// bindings are pinned by the programmer; silently yielding nil would mask
/// typos and missing commits).
fn resolve_versioned_boxed(ns_part: &str, name: &str, commit: &str) -> *const Value {
    let Some((globals, current_ns)) = cljrs_env::callback::capture_eval_context() else {
        return rt_const_nil();
    };
    match cljrs_env::versioned::resolve_versioned_value(
        &globals,
        &current_ns,
        Some(ns_part),
        name,
        commit,
    ) {
        Ok(val) => box_or_intern_val(val),
        Err(e) => {
            stash_pending_exception(Value::Str(GcPtr::new(format!("{e}"))));
            rt_const_nil()
        }
    }
}

/// Define (intern) a global var.  Returns a pointer to the Var value.
///
/// # Safety
/// String pointers must be valid UTF-8.  `val` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_def_var(
    ns_ptr: *const u8,
    ns_len: u64,
    name_ptr: *const u8,
    name_len: u64,
    val: *const Value,
) -> *const Value {
    let ns = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize))
    };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let val = unsafe { val_ref(val) }.clone();

    if let Some((globals, _)) = cljrs_env::callback::capture_eval_context() {
        let var = globals.intern(ns, Arc::from(name), val);
        box_val(Value::Var(var))
    } else {
        rt_const_nil()
    }
}

/// Load a global Var object (NOT its value) by namespace and name.
///
/// Returns `Value::Var(var)` so it can be used with `set!` and `binding`.
///
/// # Safety
/// String pointers must be valid UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_load_var(
    ns_ptr: *const u8,
    ns_len: u64,
    name_ptr: *const u8,
    name_len: u64,
) -> *const Value {
    let ns = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize))
    };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };

    if let Some((globals, current_ns)) = cljrs_env::callback::capture_eval_context() {
        // Try the specified namespace (interns + refers).
        if let Some(var) = globals.lookup_var_in_ns(ns, name) {
            return box_val(Value::Var(var));
        }
        // Try resolving ns as an alias in the current namespace.
        if let Some(resolved_ns) = globals.resolve_alias(&current_ns, ns)
            && let Some(var) = globals.lookup_var_in_ns(&resolved_ns, name)
        {
            return box_val(Value::Var(var));
        }
        // If ns is the current namespace, also check refers via current ns.
        if ns == current_ns.as_ref()
            && let Some(var) = globals.lookup_var_in_ns(&current_ns, name)
        {
            return box_val(Value::Var(var));
        }
    }
    rt_const_nil()
}

// ── Function calls ──────────────────────────────────────────────────────────

/// Call a Clojure function value with `nargs` arguments.
///
/// Uses the callback infrastructure (thread-local eval context) to dispatch.
///
/// # Safety
/// `callee` must be a valid `*const Value` pointing to a callable.
/// `args` must point to `nargs` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_call(
    callee: *const Value,
    args: *const *const Value,
    nargs: u64,
) -> *const Value {
    let callee = unsafe { val_ref(callee) };
    let nargs = nargs as usize;
    // Zero-arg call sites pass a null args pointer (see emit_unknown_call);
    // from_raw_parts requires non-null even for empty slices.
    let arg_slice: &[*const Value] = if nargs == 0 || args.is_null() {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(args, nargs) }
    };

    // Fast paths for map/set callables — avoid interpreter + GC heap allocation.
    match callee {
        Value::Map(m) if nargs >= 1 => {
            let key = unsafe { val_ref(arg_slice[0]) };
            return match m.get(key) {
                Some(val) => box_coll_val(val.clone()),
                None => rt_const_nil(),
            };
        }
        Value::Set(s) if nargs >= 1 => {
            let key = unsafe { val_ref(arg_slice[0]) };
            return if s.contains(key) {
                arg_slice[0]
            } else {
                rt_const_nil()
            };
        }
        _ => {}
    }

    let arg_values: Vec<Value> = arg_slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();

    match cljrs_env::callback::invoke(callee, arg_values) {
        Ok(result) => box_invoke_result(result),
        Err(cljrs_value::ValueError::Thrown(val)) => {
            PENDING_EXCEPTION.with(|cell| {
                *cell.borrow_mut() = Some(box_val(val));
            });
            rt_const_nil()
        }
        Err(_e) => rt_const_nil(),
    }
}

/// Deref a value (atoms, vars, delays, etc.).
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_deref(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) }.clone();
    match cljrs_interp::eval::deref_value(v) {
        Ok(result) => box_invoke_result(result),
        Err(_) => rt_const_nil(),
    }
}

// ── Printing ────────────────────────────────────────────────────────────────

/// `(println v)`.
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_println(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    cljrs_builtins::builtins::emit_output_ln(&format!("{}", PrintValue(v)));
    rt_const_nil()
}

/// `(pr v)`.
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_pr(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    cljrs_builtins::builtins::emit_output(&format!("{v}"));
    rt_const_nil()
}

// ── Type checks ─────────────────────────────────────────────────────────────

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_nil(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Nil))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_seq(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    intern_bool(matches!(
        v,
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_)
    ))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_vector(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Vector(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_map(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Map(_)))
}

// ── Identity ────────────────────────────────────────────────────────────────

/// `(identical? a b)` — pointer identity.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_identical(a: *const Value, b: *const Value) -> *const Value {
    intern_bool(std::ptr::eq(a, b))
}

// ── Str ─────────────────────────────────────────────────────────────────────

/// `(str v)`.
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_str(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    let s = format!("{}", PrintValue(v));
    box_val(Value::Str(GcPtr::new(s)))
}

/// `(str a b c ...)` — variadic str, concatenates PrintValue representations.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_str_n(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    let mut result = String::new();
    for v in &args {
        if !matches!(v, Value::Nil) {
            result.push_str(&format!("{}", PrintValue(v)));
        }
    }
    box_val(Value::Str(GcPtr::new(result)))
}

/// `(println a b c ...)` — variadic println, space-separated PrintValue representations.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_println_n(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    let s: String = args
        .iter()
        .map(|v| format!("{}", PrintValue(v)))
        .collect::<Vec<_>>()
        .join(" ");
    cljrs_builtins::builtins::emit_output_ln(&s);
    rt_const_nil()
}

// ── Output capture ──────────────────────────────────────────────────────────

/// `(with-out-str body-closure)` — push capture, call closure, pop capture, return string.
///
/// # Safety
/// `body_fn` must be a valid pointer to a Value (a callable).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_with_out_str(body_fn: *const Value) -> *const Value {
    let f = unsafe { val_ref(body_fn) }.clone();
    cljrs_builtins::builtins::push_output_capture();
    let _result = cljrs_env::callback::invoke(&f, vec![]);
    let captured = cljrs_builtins::builtins::pop_output_capture().unwrap_or_default();
    box_val(Value::Str(GcPtr::new(captured)))
}

// ── Collection operations (extended) ─────────────────────────────────────────

/// `(dissoc m k)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_dissoc(m: *const Value, k: *const Value) -> *const Value {
    let m = unsafe { val_ref(m) };
    let k = unsafe { val_ref(k) };
    match m {
        Value::Map(map) => box_coll_val(Value::Map(map.dissoc(k))),
        _ => box_coll_val(m.clone()),
    }
}

/// `(disj set val)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_disj(set: *const Value, val: *const Value) -> *const Value {
    let set = unsafe { val_ref(set) };
    let val = unsafe { val_ref(val) };
    match set {
        Value::Set(s) => box_coll_val(Value::Set(s.disj(val))),
        _ => box_coll_val(set.clone()),
    }
}

/// `(nth coll idx)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_nth(coll: *const Value, idx: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let idx = unsafe { val_ref(idx) };
    let i = match idx {
        Value::Long(n) => *n as usize,
        _ => return rt_const_nil(),
    };
    match coll {
        // Return interior pointer for Vector — no alloc.
        Value::Vector(v) => match v.get().nth(i) {
            Some(val) => val as *const Value,
            None => intern_nil(),
        },
        Value::List(l) => box_or_intern_val(l.get().iter().nth(i).cloned().unwrap_or(Value::Nil)),
        Value::Str(s) => box_or_intern_val(
            s.get()
                .chars()
                .nth(i)
                .map(Value::Char)
                .unwrap_or(Value::Nil),
        ),
        _ => intern_nil(),
    }
}

/// `(contains? coll key)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_contains(coll: *const Value, key: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let key = unsafe { val_ref(key) };
    let result = match coll {
        Value::Map(m) => m.contains_key(key),
        Value::Set(s) => s.contains(key),
        Value::Vector(v) => {
            if let Value::Long(i) = key {
                let i = *i as usize;
                i < v.get().count()
            } else {
                false
            }
        }
        _ => false,
    };
    intern_bool(result)
}

// ── Sequence operations (extended) ──────────────────────────────────────────

/// `(seq coll)` — returns a seq on the collection, or nil if empty.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_seq(coll: *const Value) -> *const Value {
    let coll_ref = unsafe { val_ref(coll) };
    match coll_ref {
        Value::Nil => rt_const_nil(),
        Value::List(l) => {
            if l.get().is_empty() {
                rt_const_nil()
            } else {
                // Return same pointer — already a valid non-nil seq.
                coll
            }
        }
        Value::Vector(v) => {
            if v.get().count() == 0 {
                rt_const_nil()
            } else {
                // Return same pointer — rt_first/rt_rest handle Value::Vector directly,
                // so we avoid O(n) PersistentList allocation here.
                coll
            }
        }
        Value::Map(m) => {
            if m.count() == 0 {
                rt_const_nil()
            } else {
                // rt_first/rt_rest don't handle Map, so we must materialise the entry list.
                let mut pairs = Vec::new();
                m.for_each(|k, v| {
                    pairs.push(Value::Vector(GcPtr::new(PersistentVector::from_iter(
                        vec![k.clone(), v.clone()],
                    ))));
                });
                box_val(Value::List(GcPtr::new(PersistentList::from_iter(pairs))))
            }
        }
        Value::Set(s) => {
            if s.count() == 0 {
                rt_const_nil()
            } else {
                // rt_first/rt_rest don't handle Set, so we must materialise.
                let items: Vec<Value> = s.iter().cloned().collect();
                box_val(Value::List(GcPtr::new(PersistentList::from_iter(items))))
            }
        }
        Value::Str(s) => {
            if s.get().is_empty() {
                rt_const_nil()
            } else {
                let chars: Vec<Value> = s.get().chars().map(Value::Char).collect();
                box_val(Value::List(GcPtr::new(PersistentList::from_iter(chars))))
            }
        }
        // Already a seq — return same pointer.
        Value::Cons(_) | Value::LazySeq(_) => coll,
        _ => rt_const_nil(),
    }
}

/// `(lazy-seq thunk-fn)` — creates a lazy sequence from a zero-arg function.
///
/// # Safety
/// `thunk_fn` must be a valid pointer to a callable Value.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lazy_seq(thunk_fn: *const Value) -> *const Value {
    use cljrs_value::types::{LazySeq, Thunk};

    let f = unsafe { val_ref(thunk_fn) }.clone();

    #[derive(Debug)]
    struct CompiledThunk(Value);

    impl Thunk for CompiledThunk {
        fn force(&self) -> Result<Value, String> {
            let _root = cljrs_env::gc_roots::root_value(&self.0);
            cljrs_env::callback::invoke(&self.0, vec![]).map_err(|e| format!("{e}"))
        }
    }

    impl cljrs_gc::Trace for CompiledThunk {
        fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
            self.0.trace(visitor);
        }
    }

    // SAFETY: Value is Send + Sync
    unsafe impl Send for CompiledThunk {}
    unsafe impl Sync for CompiledThunk {}

    box_val(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(
        CompiledThunk(f),
    )))))
}

// ── Transient operations ────────────────────────────────────────────────────

/// `(transient coll)`.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_transient(coll: *const Value) -> *const Value {
    use cljrs_value::collections::{TransientMap, TransientSet, TransientVector};

    let coll = unsafe { val_ref(coll) };
    match coll {
        Value::Vector(v) => box_val(Value::TransientVector(GcPtr::new(
            TransientVector::new_from_vector(v.get().inner()),
        ))),
        Value::Map(MapValue::Hash(m)) => box_val(Value::TransientMap(GcPtr::new(
            TransientMap::new_from_map(m.get().inner()),
        ))),
        Value::Set(SetValue::Hash(s)) => box_val(Value::TransientSet(GcPtr::new(
            TransientSet::new_from_set(s.get().inner()),
        ))),
        _ => box_val(coll.clone()),
    }
}

/// `(assoc! t k v)`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_assoc_bang(
    t: *const Value,
    k: *const Value,
    v: *const Value,
) -> *const Value {
    let t = unsafe { val_ref(t) };
    let k = unsafe { val_ref(k) }.clone();
    let v = unsafe { val_ref(v) }.clone();
    match t {
        Value::TransientMap(m) => {
            let _ = m.get().assoc(k, v);
            box_val(t.clone())
        }
        Value::TransientVector(tv) => {
            if let Value::Long(idx) = &k {
                let _ = tv.get().set(*idx as usize, v);
            }
            box_val(t.clone())
        }
        _ => box_val(t.clone()),
    }
}

/// `(conj! t v)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_conj_bang(t: *const Value, v: *const Value) -> *const Value {
    let t = unsafe { val_ref(t) };
    let v = unsafe { val_ref(v) }.clone();
    match t {
        Value::TransientVector(tv) => {
            let _ = tv.get().append(v);
            box_val(t.clone())
        }
        Value::TransientMap(m) => {
            // (conj! transient-map [k v])
            if let Value::Vector(pair) = &v
                && pair.get().count() == 2
            {
                let k = pair.get().nth(0).cloned().unwrap_or(Value::Nil);
                let val = pair.get().nth(1).cloned().unwrap_or(Value::Nil);
                let _ = m.get().assoc(k, val);
            }
            box_val(t.clone())
        }
        Value::TransientSet(s) => {
            let _ = s.get().conj(v);
            box_val(t.clone())
        }
        _ => box_val(t.clone()),
    }
}

/// `(persistent! t)`.
///
/// # Safety
/// `t` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_persistent_bang(t: *const Value) -> *const Value {
    let t = unsafe { val_ref(t) };
    match t {
        Value::TransientVector(tv) => match tv.get().persistent() {
            Ok(v) => box_val(Value::Vector(GcPtr::new(v))),
            Err(_) => rt_const_nil(),
        },
        Value::TransientMap(m) => match m.get().persistent() {
            Ok(m) => box_val(Value::Map(MapValue::Hash(GcPtr::new(m)))),
            Err(_) => rt_const_nil(),
        },
        Value::TransientSet(s) => match s.get().persistent() {
            Ok(s) => box_val(Value::Set(SetValue::Hash(GcPtr::new(s)))),
            Err(_) => rt_const_nil(),
        },
        _ => box_val(t.clone()),
    }
}

// ── Atom operations ─────────────────────────────────────────────────────────

/// `(reset! atom val)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atom_reset(atom: *const Value, val: *const Value) -> *const Value {
    let atom = unsafe { val_ref(atom) };
    let val = unsafe { val_ref(val) }.clone();
    match atom {
        Value::Atom(a) => box_val(a.get().reset(val)),
        Value::SharedAtom(sa) => match cljrs_value::promote(&val) {
            Ok(sv) => {
                sa.reset(sv);
                box_val(val)
            }
            Err(_) => rt_const_nil(),
        },
        _ => rt_const_nil(),
    }
}

/// `(swap! atom f & args)`.
///
/// # Safety
/// `atom` and `f` must be valid pointers.
/// `extra_args` must point to `nextra` valid pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_atom_swap(
    atom: *const Value,
    f: *const Value,
    extra_args: *const *const Value,
    nextra: u64,
) -> *const Value {
    let atom_val = unsafe { val_ref(atom) };
    let f = unsafe { val_ref(f) }.clone();
    let nextra = nextra as usize;
    let extra: Vec<Value> = if nextra > 0 {
        let slice = unsafe { std::slice::from_raw_parts(extra_args, nextra) };
        slice
            .iter()
            .map(|p| unsafe { val_ref(*p) }.clone())
            .collect()
    } else {
        vec![]
    };

    match atom_val {
        Value::Atom(a) => {
            // swap! semantics: (f current-val extra-args...)
            // CAS loop
            loop {
                let current = a.get().value.lock().unwrap().clone();
                let mut args = vec![current.clone()];
                args.extend(extra.iter().cloned());
                match cljrs_env::callback::invoke(&f, args) {
                    Ok(new_val) => {
                        let mut guard = a.get().value.lock().unwrap();
                        if *guard == current {
                            *guard = new_val.clone();
                            return box_val(new_val);
                        }
                        // CAS failed, retry
                    }
                    Err(_) => return rt_const_nil(),
                }
            }
        }
        Value::SharedAtom(sa) => {
            // Cross-isolate swap!: load → demote → apply f → promote → CAS-retry.
            loop {
                let cur = sa.deref_val();
                let mut args = vec![cljrs_value::demote(&cur)];
                args.extend(extra.iter().cloned());
                match cljrs_env::callback::invoke(&f, args) {
                    Ok(new_val) => match cljrs_value::promote(&new_val) {
                        Ok(sv) => {
                            if sa.compare_and_set(&cur, sv) {
                                return box_val(new_val);
                            }
                            // CAS failed, retry
                        }
                        Err(_) => return rt_const_nil(),
                    },
                    Err(_) => return rt_const_nil(),
                }
            }
        }
        _ => rt_const_nil(),
    }
}

// ── Apply ───────────────────────────────────────────────────────────────────

/// `(apply f arglist)`.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_apply(f: *const Value, arglist: *const Value) -> *const Value {
    let f = unsafe { val_ref(f) }.clone();
    let arglist = unsafe { val_ref(arglist) };

    // Collect the arg list into a Vec.
    let mut args = Vec::new();
    let mut current = arglist.clone();
    loop {
        match &current {
            Value::Nil => break,
            Value::List(l) => {
                for item in l.get().iter() {
                    args.push(item.clone());
                }
                break;
            }
            Value::Vector(v) => {
                for item in v.get().iter() {
                    args.push(item.clone());
                }
                break;
            }
            Value::Cons(c) => {
                args.push(c.get().head.clone());
                current = c.get().tail.clone();
            }
            Value::LazySeq(ls) => {
                current = ls.get().realize();
            }
            _ => break,
        }
    }

    match cljrs_env::callback::invoke(&f, args) {
        Ok(result) => box_val(result),
        Err(cljrs_value::ValueError::Thrown(val)) => {
            PENDING_EXCEPTION.with(|cell| {
                *cell.borrow_mut() = Some(box_val(val));
            });
            rt_const_nil()
        }
        Err(_) => rt_const_nil(),
    }
}

// ── Higher-order functions ───────────────────────────────────────────────────

/// Helper: look up a global function by namespace and name, then call it.
fn call_global_fn(ns: &str, name: &str, args: Vec<Value>) -> *const Value {
    if let Some((globals, _)) = cljrs_env::callback::capture_eval_context()
        && let Some(val) = globals.lookup_in_ns(ns, name)
    {
        match cljrs_env::callback::invoke(&val, args) {
            Ok(result) => box_invoke_result(result),
            Err(cljrs_value::ValueError::Thrown(v)) => {
                PENDING_EXCEPTION.with(|cell| {
                    *cell.borrow_mut() = Some(box_val(v));
                });
                rt_const_nil()
            }
            Err(_) => rt_const_nil(),
        }
    } else {
        rt_const_nil()
    }
}

/// `(reduce f coll)`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_reduce2(f: *const Value, coll: *const Value) -> *const Value {
    let f = unsafe { val_ref(f) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "reduce", vec![f, coll])
}

/// `(reduce f init coll)`.
///
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_reduce3(
    f: *const Value,
    init: *const Value,
    coll: *const Value,
) -> *const Value {
    let f = unsafe { val_ref(f) }.clone();
    let init = unsafe { val_ref(init) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "reduce", vec![f, init, coll])
}

/// `(map f coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_map(f: *const Value, coll: *const Value) -> *const Value {
    let f = unsafe { val_ref(f) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "map", vec![f, coll])
}

/// `(filter pred coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_filter(pred: *const Value, coll: *const Value) -> *const Value {
    let pred = unsafe { val_ref(pred) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "filter", vec![pred, coll])
}

/// `(mapv f coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_mapv(f: *const Value, coll: *const Value) -> *const Value {
    let f = unsafe { val_ref(f) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "mapv", vec![f, coll])
}

/// `(filterv pred coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_filterv(pred: *const Value, coll: *const Value) -> *const Value {
    let pred = unsafe { val_ref(pred) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "filterv", vec![pred, coll])
}

/// `(some pred coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_some(pred: *const Value, coll: *const Value) -> *const Value {
    let pred = unsafe { val_ref(pred) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "some", vec![pred, coll])
}

/// `(every? pred coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_every(pred: *const Value, coll: *const Value) -> *const Value {
    let pred = unsafe { val_ref(pred) }.clone();
    let coll = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "every?", vec![pred, coll])
}

/// `(into to from)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_into(to: *const Value, from: *const Value) -> *const Value {
    let to_ref = unsafe { val_ref(to) };
    let from_ref = unsafe { val_ref(from) };
    // Fast paths: Vector target — iterate source directly without the interpreter.
    // This keeps all intermediate allocations in the active region when present.
    if let Value::Vector(v) = to_ref {
        let mut result = v.get().clone();
        match from_ref {
            Value::Set(s) => {
                for elem in s.iter() {
                    result = result.conj(elem.clone());
                }
            }
            Value::Vector(v2) => {
                for elem in v2.get().iter() {
                    result = result.conj(elem.clone());
                }
            }
            Value::Nil => {}
            _ => {
                let to_val = to_ref.clone();
                let from_val = from_ref.clone();
                return call_global_fn("clojure.core", "into", vec![to_val, from_val]);
            }
        }
        return box_coll_val(Value::Vector(alloc_inner_coll(result)));
    }
    // Fast path: hash-set target.  `into` always fully realizes its source, so
    // there is no laziness to preserve; iterating an eagerly-walkable source
    // and conj-ing straight into the set avoids the interpreted `into` +
    // lazy-`seq` realization that otherwise allocates every intermediate cons
    // cell on the GC heap (the dominant cost of `(into #{} (repeatedly …))` in
    // `samples/graph.cljrs`).  All allocations land in the active region.
    if let Value::Set(SetValue::Hash(h)) = to_ref
        && let Some(elems) = eager_seq_elems(from_ref)
    {
        let mut result = h.get().clone();
        for elem in elems {
            result.conj_mut(elem);
        }
        return box_coll_val(Value::Set(SetValue::Hash(alloc_inner_coll(result))));
    }
    // Fast path: map target.  `(into {} pairs)` — the very common idiom behind
    // `(into {} (for …))` map comprehensions — otherwise falls through to the
    // interpreted `into`, whose lazy realization allocates on the GC heap.  We
    // build the result in one shot via `MapValue::from_pairs` (last-wins, like
    // `assoc`, and size-optimal) rather than per-element `assoc` so there are
    // no intermediate map boxes; the result is region-boxed when a region is
    // open.  Only taken when every source element is a clean key/value pair
    // (a 2-element vector/list) or the source is itself a map.
    if let Value::Map(to_map) = to_ref
        && let Some(new_pairs) = into_map_pairs(from_ref)
    {
        // Existing entries first, then the new pairs so they win on conflict.
        let mut pairs: Vec<(Value, Value)> =
            to_map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        pairs.extend(new_pairs);
        return box_coll_val(Value::Map(MapValue::from_pairs(pairs)));
    }
    let to_val = to_ref.clone();
    let from_val = from_ref.clone();
    call_global_fn("clojure.core", "into", vec![to_val, from_val])
}

/// Collect a sequence of key/value pairs from `from` for the map-target `into`
/// fast path, or `None` if `from` isn't eagerly walkable or any element isn't a
/// clean 2-element pair (in which case the caller falls back to the
/// interpreter, which handles map-entries, map merging, and lazy sources).
fn into_map_pairs(from: &Value) -> Option<Vec<(Value, Value)>> {
    // A source map contributes its entries directly.
    if let Value::Map(m) = from {
        return Some(m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
    }
    let elems = eager_seq_elems(from)?;
    let mut pairs = Vec::with_capacity(elems.len());
    for elem in elems {
        pairs.push(as_pair(&elem)?);
    }
    Some(pairs)
}

/// Extract a `(key, value)` pair from a 2-element vector or list, else `None`.
fn as_pair(v: &Value) -> Option<(Value, Value)> {
    match v {
        Value::Vector(vec) if vec.get().count() == 2 => {
            Some((vec.get().nth(0)?.clone(), vec.get().nth(1)?.clone()))
        }
        Value::List(l) if l.get().count() == 2 => {
            let mut it = l.get().iter();
            let k = it.next()?.clone();
            let val = it.next()?.clone();
            Some((k, val))
        }
        _ => None,
    }
}

/// `(into to xform from)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_into3(
    to: *const Value,
    xform: *const Value,
    from: *const Value,
) -> *const Value {
    let to = unsafe { val_ref(to) }.clone();
    let xform = unsafe { val_ref(xform) }.clone();
    let from = unsafe { val_ref(from) }.clone();
    call_global_fn("clojure.core", "into", vec![to, xform, from])
}

/// `(peek coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_peek(coll: *const Value) -> *const Value {
    let coll_ref = unsafe { val_ref(coll) };
    match coll_ref {
        Value::Vector(v) => match v.get().peek() {
            // Return an interior pointer into the rpds leaf node that holds
            // the last element.  The leaf is Arc-managed by the
            // PersistentVector; the Vector's GcBox (in the region or on the
            // GC heap) keeps the Arc alive until after any use of this ptr.
            Some(val) => val as *const Value,
            None => rt_const_nil(),
        },
        _ => {
            let coll_val = coll_ref.clone();
            call_global_fn("clojure.core", "peek", vec![coll_val])
        }
    }
}

/// `(pop coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_pop(coll: *const Value) -> *const Value {
    let coll_ref = unsafe { val_ref(coll) };
    match coll_ref {
        Value::Vector(v) => match v.get().pop() {
            Some(new_pv) => box_coll_val(Value::Vector(alloc_inner_coll(new_pv))),
            None => rt_const_nil(),
        },
        _ => {
            let coll_val = coll_ref.clone();
            call_global_fn("clojure.core", "pop", vec![coll_val])
        }
    }
}

/// `(vec coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_vec(coll: *const Value) -> *const Value {
    let coll_ref = unsafe { val_ref(coll) };
    match coll_ref {
        // Already a vector: return the same pointer — no allocation needed.
        Value::Vector(_) => coll,
        _ => {
            let coll_val = coll_ref.clone();
            call_global_fn("clojure.core", "vec", vec![coll_val])
        }
    }
}

/// `(mapcat f coll)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_mapcat(f: *const Value, coll: *const Value) -> *const Value {
    let f_ref = unsafe { val_ref(f) };
    let coll_ref = unsafe { val_ref(coll) };
    // Fast path: f is a Map and coll is a Vector.
    // Applies the map lookup to each element and concatenates the resulting
    // collections into a new Vector — entirely in the active region if present.
    if let (Value::Map(m), Value::Vector(v)) = (f_ref, coll_ref) {
        let mut result = PersistentVector::empty();
        for elem in v.get().iter() {
            if let Some(neighbors) = m.get(elem) {
                match &neighbors {
                    Value::Set(s) => {
                        for item in s.iter() {
                            result = result.conj(item.clone());
                        }
                    }
                    Value::Vector(nv) => {
                        for item in nv.get().iter() {
                            result = result.conj(item.clone());
                        }
                    }
                    _ => {}
                }
            }
        }
        return box_coll_val(Value::Vector(alloc_inner_coll(result)));
    }
    let f_val = f_ref.clone();
    let coll_val = coll_ref.clone();
    call_global_fn("clojure.core", "mapcat", vec![f_val, coll_val])
}

/// `(repeatedly n f)`.
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_repeatedly(n: *const Value, f: *const Value) -> *const Value {
    let n_ref = unsafe { val_ref(n) };
    let f_ref = unsafe { val_ref(f) };
    // Fast path: explicit count.  `(repeatedly n f)` with a fixed count is a
    // finite sequence, so realizing it eagerly into a region-allocated vector
    // is observationally compatible with the lazy seq for the eager consumers
    // it is used with (`into`, `count`, `reduce`, `vec`, …) — the same liberty
    // the eager `mapcat`/`map` fast paths already take.  This keeps the per-
    // element results in the active region instead of allocating interpreted
    // lazy-seq cons cells on the GC heap.
    if let Value::Long(k) = n_ref
        && *k >= 0
    {
        let mut items: Vec<Value> = Vec::with_capacity(*k as usize);
        for _ in 0..*k {
            match cljrs_env::callback::invoke(f_ref, vec![]) {
                Ok(v) => items.push(v),
                Err(cljrs_value::ValueError::Thrown(val)) => {
                    stash_pending_exception(val);
                    return rt_const_nil();
                }
                Err(_) => return rt_const_nil(),
            }
        }
        let pv = PersistentVector::from_iter(items);
        return box_coll_val(Value::Vector(alloc_inner_coll(pv)));
    }
    let n = n_ref.clone();
    let f = f_ref.clone();
    call_global_fn("clojure.core", "repeatedly", vec![n, f])
}

// ── set! ─────────────────────────────────────────────────────────────────────

/// `(set! var val)` — set a dynamic var's thread-local or root binding.
///
/// `var_ptr` is a `Value::Var` (the resolved var).
/// `val_ptr` is the new value.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_set_bang(var_ptr: *const Value, val_ptr: *const Value) -> *const Value {
    let var_val = unsafe { val_ref(var_ptr) };
    let val = unsafe { val_ref(val_ptr) }.clone();
    match var_val {
        Value::Var(var) => {
            // Prefer updating thread-local binding if one exists.
            if !cljrs_env::dynamics::set_thread_local(var, val.clone()) {
                var.get().bind(val.clone());
            }
            box_val(val)
        }
        _ => box_val(val),
    }
}

// ── binding ──────────────────────────────────────────────────────────────────

/// `(binding [var1 val1 var2 val2 ...] body-fn)` — push dynamic bindings, call
/// body, pop bindings (even on exception).
///
/// `bindings` is an array of alternating `*const Value` pairs: [var, val, var, val, ...]
/// `npairs` is the number of var/val pairs (half the array length).
/// `body_fn` is a zero-arg callable to invoke with bindings in effect.
///
/// # Safety
/// `bindings` must point to `2 * npairs` valid `*const Value` pointers.
/// `body_fn` must be a valid `*const Value` pointing to a callable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_with_bindings(
    bindings: *const *const Value,
    npairs: u64,
    body_fn: *const Value,
) -> *const Value {
    use std::collections::HashMap;

    let npairs = npairs as usize;
    let binding_slice = unsafe { std::slice::from_raw_parts(bindings, npairs * 2) };

    let mut frame: HashMap<usize, Value> = HashMap::new();
    for i in 0..npairs {
        let var_val = unsafe { val_ref(binding_slice[i * 2]) };
        let val = unsafe { val_ref(binding_slice[i * 2 + 1]) }.clone();
        if let Value::Var(var) = var_val {
            frame.insert(cljrs_env::dynamics::var_key_of(var), val);
        }
    }

    let _guard = cljrs_env::dynamics::push_frame(frame);
    let body = unsafe { val_ref(body_fn) }.clone();

    match cljrs_env::callback::invoke(&body, vec![]) {
        Ok(result) => box_val(result),
        Err(cljrs_value::ValueError::Thrown(val)) => {
            PENDING_EXCEPTION.with(|cell| {
                *cell.borrow_mut() = Some(box_val(val));
            });
            rt_const_nil()
        }
        Err(_) => rt_const_nil(),
    }
    // _guard drops here → pop_frame()
}

// ── Exception handling ───────────────────────────────────────────────────────

// Thread-local pending exception for compiled code.
// When rt_throw is called, it stores the exception here and returns a
// sentinel. rt_try checks this after calling the body function.
thread_local! {
    static PENDING_EXCEPTION: std::cell::RefCell<Option<*const Value>> = const { std::cell::RefCell::new(None) };
}

/// `(throw val)` — throws a Clojure value as an exception.
///
/// Stores the value in a thread-local and returns nil.  The caller (compiled
/// code) will reach an `unreachable` terminator; `rt_try` checks the
/// thread-local after each body call.
///
/// # Safety
/// `val` must be a valid pointer to a live `Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_throw(val: *const Value) -> *const Value {
    PENDING_EXCEPTION.with(|cell| {
        *cell.borrow_mut() = Some(val);
    });
    rt_const_nil()
}

/// Check if a thrown exception is pending and clear it.
fn take_pending_exception() -> Option<*const Value> {
    PENDING_EXCEPTION.with(|cell| cell.borrow_mut().take())
}

/// Take (and clear) the thread's pending exception as an owned `Value`.
///
/// Called by the JIT-native dispatch seam (via the hook installed by
/// `cljrs_jit::init`) right after native code returns, so an uncaught throw
/// propagates to the interpreter caller instead of being swallowed as nil.
/// The caller must invoke this while the JIT frame's alloc roots are still
/// live (the pending pointer targets a Value boxed inside the native frame).
pub fn take_pending_exception_value() -> Option<Value> {
    take_pending_exception().map(|ptr| unsafe { val_ref(ptr) }.clone())
}

/// `(try body (catch Ex e handler) (finally cleanup))` — exception handling.
///
/// `body_fn`: a zero-arg Clojure function for the try body.
/// `catch_fn`: a one-arg Clojure function for the catch handler (receives the
///             thrown value), or a nil Value pointer if no catch clause.
/// `finally_fn`: a zero-arg Clojure function for the finally clause, or a nil
///               Value pointer if no finally clause.
///
/// # Safety
/// All pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_try(
    body_fn: *const Value,
    catch_fn: *const Value,
    finally_fn: *const Value,
) -> *const Value {
    let body = unsafe { val_ref(body_fn) }.clone();
    let catch = unsafe { val_ref(catch_fn) }.clone();
    let finally = unsafe { val_ref(finally_fn) }.clone();

    // Save both region-stack depths so we can unwind on exception.
    let region_depth = cljrs_gc::region::region_stack_depth();
    let rt_region_depth = RT_REGION_STACK.with(|s| s.borrow().len());

    // Call the body.
    let body_result = cljrs_env::callback::invoke(&body, vec![]);

    // Check for thrown exception (set by rt_throw in compiled code).
    let ret = if let Some(thrown_ptr) = take_pending_exception() {
        // Unwind any regions opened inside the try body.
        unwind_regions_to(region_depth, rt_region_depth);
        // Exception was thrown from compiled code.
        if !matches!(catch, Value::Nil) {
            let thrown_val = unsafe { val_ref(thrown_ptr) }.clone();
            match cljrs_env::callback::invoke(&catch, vec![thrown_val]) {
                Ok(v) => box_val(v),
                Err(_) => rt_const_nil(),
            }
        } else {
            rt_const_nil()
        }
    } else {
        match body_result {
            Ok(val) => box_val(val),
            Err(val_err) => {
                // Unwind any regions opened inside the try body.
                unwind_regions_to(region_depth, rt_region_depth);
                // Body returned a ValueError (e.g. thrown via interpreter).
                if !matches!(catch, Value::Nil) {
                    let thrown_val = match val_err {
                        cljrs_value::ValueError::Thrown(v) => v,
                        other => Value::Str(GcPtr::new(other.to_string())),
                    };
                    match cljrs_env::callback::invoke(&catch, vec![thrown_val]) {
                        Ok(v) => box_val(v),
                        Err(_) => rt_const_nil(),
                    }
                } else {
                    rt_const_nil()
                }
            }
        }
    };

    // Always run finally.
    if !matches!(finally, Value::Nil) {
        let _ = cljrs_env::callback::invoke(&finally, vec![]);
    }

    ret
}

// ── More HOFs ───────────────────────────────────────────────────────────────

/// `(group-by f coll)`
#[unsafe(no_mangle)]
pub extern "C" fn rt_group_by(f: *const Value, coll: *const Value) -> *const Value {
    let fv = unsafe { val_ref(f) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "group-by", vec![fv, cv])
}

/// `(partition n coll)`
#[unsafe(no_mangle)]
pub extern "C" fn rt_partition2(n: *const Value, coll: *const Value) -> *const Value {
    let nv = unsafe { val_ref(n) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "partition", vec![nv, cv])
}

/// `(partition n step coll)`
#[unsafe(no_mangle)]
pub extern "C" fn rt_partition3(
    n: *const Value,
    step: *const Value,
    coll: *const Value,
) -> *const Value {
    let nv = unsafe { val_ref(n) }.clone();
    let sv = unsafe { val_ref(step) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "partition", vec![nv, sv, cv])
}

/// `(partition n step pad coll)`
#[unsafe(no_mangle)]
pub extern "C" fn rt_partition4(
    n: *const Value,
    step: *const Value,
    pad: *const Value,
    coll: *const Value,
) -> *const Value {
    let nv = unsafe { val_ref(n) }.clone();
    let sv = unsafe { val_ref(step) }.clone();
    let pv = unsafe { val_ref(pad) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "partition", vec![nv, sv, pv, cv])
}

/// `(frequencies coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub extern "C" fn rt_frequencies(coll: *const Value) -> *const Value {
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "frequencies", vec![cv])
}

/// `(keep f coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub extern "C" fn rt_keep(f: *const Value, coll: *const Value) -> *const Value {
    let fv = unsafe { val_ref(f) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "keep", vec![fv, cv])
}

/// `(remove pred coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub extern "C" fn rt_remove(pred: *const Value, coll: *const Value) -> *const Value {
    let pv = unsafe { val_ref(pred) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "remove", vec![pv, cv])
}

/// `(map-indexed f coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub extern "C" fn rt_map_indexed(f: *const Value, coll: *const Value) -> *const Value {
    let fv = unsafe { val_ref(f) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "map-indexed", vec![fv, cv])
}

/// `(zipmap keys vals)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub extern "C" fn rt_zipmap(keys: *const Value, vals: *const Value) -> *const Value {
    let kv = unsafe { val_ref(keys) }.clone();
    let vv = unsafe { val_ref(vals) }.clone();
    call_global_fn("clojure.core", "zipmap", vec![kv, vv])
}

/// `(juxt f1 f2 ...)` — variadic, stack-spilled
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_juxt(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    call_global_fn("clojure.core", "juxt", args)
}

/// `(comp f1 f2 ...)` — variadic, stack-spilled
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_comp(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    call_global_fn("clojure.core", "comp", args)
}

/// `(partial f arg1 arg2 ...)` — variadic, stack-spilled
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_partial(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    call_global_fn("clojure.core", "partial", args)
}

/// `(complement f)`
///
/// # Safety
/// `f` must be a valid pointer to a callable Value.
#[unsafe(no_mangle)]
pub extern "C" fn rt_complement(f: *const Value) -> *const Value {
    let fv = unsafe { val_ref(f) }.clone();
    call_global_fn("clojure.core", "complement", vec![fv])
}

// ── Sequence operations ──────────────────────────────────────────────────────

/// `(concat & colls)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_concat(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    call_global_fn("clojure.core", "concat", args)
}

/// `(range end)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_range1(end: *const Value) -> *const Value {
    let e = unsafe { val_ref(end) }.clone();
    call_global_fn("clojure.core", "range", vec![e])
}

/// `(range start end)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_range2(start: *const Value, end: *const Value) -> *const Value {
    let s = unsafe { val_ref(start) }.clone();
    let e = unsafe { val_ref(end) }.clone();
    call_global_fn("clojure.core", "range", vec![s, e])
}

/// `(range start end step)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_range3(
    start: *const Value,
    end: *const Value,
    step: *const Value,
) -> *const Value {
    let s = unsafe { val_ref(start) }.clone();
    let e = unsafe { val_ref(end) }.clone();
    let st = unsafe { val_ref(step) }.clone();
    call_global_fn("clojure.core", "range", vec![s, e, st])
}

/// `(take n coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_take(n: *const Value, coll: *const Value) -> *const Value {
    let nv = unsafe { val_ref(n) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "take", vec![nv, cv])
}

/// `(drop n coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_drop(n: *const Value, coll: *const Value) -> *const Value {
    let nv = unsafe { val_ref(n) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "drop", vec![nv, cv])
}

/// `(reverse coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_reverse(coll: *const Value) -> *const Value {
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "reverse", vec![cv])
}

/// `(sort coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_sort(coll: *const Value) -> *const Value {
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "sort", vec![cv])
}

/// `(sort-by keyfn coll)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_sort_by(keyfn: *const Value, coll: *const Value) -> *const Value {
    let kf = unsafe { val_ref(keyfn) }.clone();
    let cv = unsafe { val_ref(coll) }.clone();
    call_global_fn("clojure.core", "sort-by", vec![kf, cv])
}

// ── Collection operations ───────────────────────────────────────────────────

/// `(keys m)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_keys(m: *const Value) -> *const Value {
    let mv = unsafe { val_ref(m) }.clone();
    call_global_fn("clojure.core", "keys", vec![mv])
}

/// `(vals m)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_vals(m: *const Value) -> *const Value {
    let mv = unsafe { val_ref(m) }.clone();
    call_global_fn("clojure.core", "vals", vec![mv])
}

/// `(merge & maps)` — variadic, stack-spilled
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_merge(elems: *const *const Value, n: i64) -> *const Value {
    let args = unsafe { collect_args(elems, n) };
    call_global_fn("clojure.core", "merge", args)
}

/// `(update m k f)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_update(
    m: *const Value,
    k: *const Value,
    f: *const Value,
) -> *const Value {
    let mv = unsafe { val_ref(m) }.clone();
    let kv = unsafe { val_ref(k) }.clone();
    let fv = unsafe { val_ref(f) }.clone();
    call_global_fn("clojure.core", "update", vec![mv, kv, fv])
}

/// `(get-in m ks)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_get_in(m: *const Value, ks: *const Value) -> *const Value {
    let mv = unsafe { val_ref(m) }.clone();
    let kv = unsafe { val_ref(ks) }.clone();
    call_global_fn("clojure.core", "get-in", vec![mv, kv])
}

/// `(assoc-in m ks v)`
/// # Safety
/// All pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_assoc_in(
    m: *const Value,
    ks: *const Value,
    v: *const Value,
) -> *const Value {
    let mv = unsafe { val_ref(m) }.clone();
    let kv = unsafe { val_ref(ks) }.clone();
    let vv = unsafe { val_ref(v) }.clone();
    call_global_fn("clojure.core", "assoc-in", vec![mv, kv, vv])
}

// ── Type predicates ─────────────────────────────────────────────────────────

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_number(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    intern_bool(matches!(val, Value::Long(_) | Value::Double(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_string(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Str(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_keyword(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Keyword(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_symbol(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Symbol(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_bool(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Bool(_)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_int(v: *const Value) -> *const Value {
    intern_bool(matches!(unsafe { val_ref(v) }, Value::Long(_)))
}

// ── Additional I/O ──────────────────────────────────────────────────────────

/// `(prn x)` — print readably + newline
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_prn(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    cljrs_builtins::builtins::emit_output_ln(&format!("{val}"));
    box_val(Value::Nil)
}

/// `(print x)` — print without newline
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_print(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    cljrs_builtins::builtins::emit_output(&format!("{}", PrintValue(val)));
    box_val(Value::Nil)
}

// ── Atom construction ───────────────────────────────────────────────────────

/// `(atom x)`
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_atom(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) }.clone();
    call_global_fn("clojure.core", "atom", vec![val])
}

// ── Symbol anchor ────────────────────────────────────────────────────────────

/// Force the linker to include all `rt_*` symbols.
///
/// Call this from the AOT harness `main()` so the linker doesn't strip the
/// runtime bridge functions as dead code.  The function itself is a no-op at
/// runtime — the compiler cannot optimise it away because of the opaque
/// `std::hint::black_box` fence.
#[inline(never)]
pub fn anchor_rt_symbols() {
    std::hint::black_box(rt_const_nil as *const () as usize);
    std::hint::black_box(rt_const_true as *const () as usize);
    std::hint::black_box(rt_const_false as *const () as usize);
    std::hint::black_box(rt_const_long as *const () as usize);
    std::hint::black_box(rt_const_double as *const () as usize);
    std::hint::black_box(rt_const_char as *const () as usize);
    std::hint::black_box(rt_const_string as *const () as usize);
    std::hint::black_box(rt_const_keyword as *const () as usize);
    std::hint::black_box(rt_const_symbol as *const () as usize);
    std::hint::black_box(rt_truthiness as *const () as usize);
    std::hint::black_box(rt_add as *const () as usize);
    std::hint::black_box(rt_sub as *const () as usize);
    std::hint::black_box(rt_mul as *const () as usize);
    std::hint::black_box(rt_unchecked_add as *const () as usize);
    std::hint::black_box(rt_unchecked_sub as *const () as usize);
    std::hint::black_box(rt_unchecked_mul as *const () as usize);
    std::hint::black_box(rt_overflow_error as *const () as usize);
    std::hint::black_box(rt_alength as *const () as usize);
    std::hint::black_box(rt_aget_long as *const () as usize);
    std::hint::black_box(rt_aget_double as *const () as usize);
    std::hint::black_box(rt_aset_long as *const () as usize);
    std::hint::black_box(rt_aset_double as *const () as usize);
    std::hint::black_box(rt_aget as *const () as usize);
    std::hint::black_box(rt_aset as *const () as usize);
    std::hint::black_box(rt_div as *const () as usize);
    std::hint::black_box(rt_rem as *const () as usize);
    std::hint::black_box(rt_eq as *const () as usize);
    std::hint::black_box(rt_case_eq as *const () as usize);
    std::hint::black_box(rt_lt as *const () as usize);
    std::hint::black_box(rt_gt as *const () as usize);
    std::hint::black_box(rt_lte as *const () as usize);
    std::hint::black_box(rt_gte as *const () as usize);
    std::hint::black_box(rt_alloc_vector as *const () as usize);
    std::hint::black_box(rt_alloc_map as *const () as usize);
    std::hint::black_box(rt_alloc_set as *const () as usize);
    std::hint::black_box(rt_alloc_list as *const () as usize);
    std::hint::black_box(rt_alloc_cons as *const () as usize);
    std::hint::black_box(rt_get as *const () as usize);
    std::hint::black_box(rt_count as *const () as usize);
    std::hint::black_box(rt_first as *const () as usize);
    std::hint::black_box(rt_rest as *const () as usize);
    std::hint::black_box(rt_assoc as *const () as usize);
    std::hint::black_box(rt_conj as *const () as usize);
    std::hint::black_box(rt_call as *const () as usize);
    std::hint::black_box(rt_deref as *const () as usize);
    std::hint::black_box(rt_load_global as *const () as usize);
    std::hint::black_box(rt_def_var as *const () as usize);
    std::hint::black_box(rt_println as *const () as usize);
    std::hint::black_box(rt_pr as *const () as usize);
    std::hint::black_box(rt_is_nil as *const () as usize);
    std::hint::black_box(rt_is_vector as *const () as usize);
    std::hint::black_box(rt_is_map as *const () as usize);
    std::hint::black_box(rt_is_seq as *const () as usize);
    std::hint::black_box(rt_identical as *const () as usize);
    std::hint::black_box(rt_str as *const () as usize);
    std::hint::black_box(rt_str_n as *const () as usize);
    std::hint::black_box(rt_println_n as *const () as usize);
    std::hint::black_box(rt_with_out_str as *const () as usize);
    std::hint::black_box(rt_make_fn as *const () as usize);
    std::hint::black_box(rt_make_fn_variadic as *const () as usize);
    std::hint::black_box(rt_make_fn_multi as *const () as usize);
    std::hint::black_box(rt_throw as *const () as usize);
    std::hint::black_box(rt_try as *const () as usize);
    std::hint::black_box(rt_dissoc as *const () as usize);
    std::hint::black_box(rt_disj as *const () as usize);
    std::hint::black_box(rt_nth as *const () as usize);
    std::hint::black_box(rt_contains as *const () as usize);
    std::hint::black_box(rt_seq as *const () as usize);
    std::hint::black_box(rt_lazy_seq as *const () as usize);
    std::hint::black_box(rt_transient as *const () as usize);
    std::hint::black_box(rt_assoc_bang as *const () as usize);
    std::hint::black_box(rt_conj_bang as *const () as usize);
    std::hint::black_box(rt_persistent_bang as *const () as usize);
    std::hint::black_box(rt_atom_reset as *const () as usize);
    std::hint::black_box(rt_atom_swap as *const () as usize);
    std::hint::black_box(rt_apply as *const () as usize);
    std::hint::black_box(rt_reduce2 as *const () as usize);
    std::hint::black_box(rt_reduce3 as *const () as usize);
    std::hint::black_box(rt_map as *const () as usize);
    std::hint::black_box(rt_filter as *const () as usize);
    std::hint::black_box(rt_mapv as *const () as usize);
    std::hint::black_box(rt_filterv as *const () as usize);
    std::hint::black_box(rt_some as *const () as usize);
    std::hint::black_box(rt_every as *const () as usize);
    std::hint::black_box(rt_into as *const () as usize);
    std::hint::black_box(rt_into3 as *const () as usize);
    std::hint::black_box(rt_set_bang as *const () as usize);
    std::hint::black_box(rt_with_bindings as *const () as usize);
    std::hint::black_box(rt_load_var as *const () as usize);
    std::hint::black_box(rt_concat as *const () as usize);
    std::hint::black_box(rt_range1 as *const () as usize);
    std::hint::black_box(rt_range2 as *const () as usize);
    std::hint::black_box(rt_range3 as *const () as usize);
    std::hint::black_box(rt_take as *const () as usize);
    std::hint::black_box(rt_drop as *const () as usize);
    std::hint::black_box(rt_reverse as *const () as usize);
    std::hint::black_box(rt_sort as *const () as usize);
    std::hint::black_box(rt_sort_by as *const () as usize);
    std::hint::black_box(rt_keys as *const () as usize);
    std::hint::black_box(rt_vals as *const () as usize);
    std::hint::black_box(rt_merge as *const () as usize);
    std::hint::black_box(rt_update as *const () as usize);
    std::hint::black_box(rt_get_in as *const () as usize);
    std::hint::black_box(rt_assoc_in as *const () as usize);
    std::hint::black_box(rt_is_number as *const () as usize);
    std::hint::black_box(rt_is_string as *const () as usize);
    std::hint::black_box(rt_is_keyword as *const () as usize);
    std::hint::black_box(rt_is_symbol as *const () as usize);
    std::hint::black_box(rt_is_bool as *const () as usize);
    std::hint::black_box(rt_is_int as *const () as usize);
    std::hint::black_box(rt_prn as *const () as usize);
    std::hint::black_box(rt_print as *const () as usize);
    std::hint::black_box(rt_atom as *const () as usize);
    std::hint::black_box(rt_group_by as *const () as usize);
    std::hint::black_box(rt_partition2 as *const () as usize);
    std::hint::black_box(rt_partition3 as *const () as usize);
    std::hint::black_box(rt_partition4 as *const () as usize);
    std::hint::black_box(rt_frequencies as *const () as usize);
    std::hint::black_box(rt_keep as *const () as usize);
    std::hint::black_box(rt_remove as *const () as usize);
    std::hint::black_box(rt_map_indexed as *const () as usize);
    std::hint::black_box(rt_zipmap as *const () as usize);
    std::hint::black_box(rt_juxt as *const () as usize);
    std::hint::black_box(rt_comp as *const () as usize);
    std::hint::black_box(rt_partial as *const () as usize);
    std::hint::black_box(rt_complement as *const () as usize);
    std::hint::black_box(rt_peek as *const () as usize);
    std::hint::black_box(rt_pop as *const () as usize);
    std::hint::black_box(rt_vec as *const () as usize);
    std::hint::black_box(rt_mapcat as *const () as usize);
    std::hint::black_box(rt_repeatedly as *const () as usize);
    std::hint::black_box(rt_value_tag as *const () as usize);
    std::hint::black_box(rt_unbox_long as *const () as usize);
    std::hint::black_box(rt_unbox_double as *const () as usize);
    std::hint::black_box(rt_box_bool as *const () as usize);
    std::hint::black_box(rt_deopt as *const () as usize);
    std::hint::black_box(rt_kw_ic_fill as *const () as usize);
    std::hint::black_box(rt_load_global_versioned_ic as *const () as usize);
    std::hint::black_box(rt_call_ic as *const () as usize);
}

// ═════════════════════════════════════════════════════════════════════════════
// Phase 10.6 — specialization & inline caches
// ═════════════════════════════════════════════════════════════════════════════

/// Runtime-type tag classes used by specialization entry guards
/// (`rt_value_tag` result; must match `codegen.rs`'s guard constants).
pub const TAG_OTHER: i64 = 0;
pub const TAG_LONG: i64 = 1;
pub const TAG_DOUBLE: i64 = 2;
pub const TAG_BOOL: i64 = 3;
pub const TAG_NIL: i64 = 4;

/// Classify a value's runtime type for specialization guards.
///
/// # Safety
/// `v` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_value_tag(v: *const Value) -> i64 {
    match unsafe { val_ref(v) } {
        Value::Long(_) => TAG_LONG,
        Value::Double(_) => TAG_DOUBLE,
        Value::Bool(_) => TAG_BOOL,
        Value::Nil => TAG_NIL,
        _ => TAG_OTHER,
    }
}

/// Extract the raw `i64` payload of a `Value::Long`.
///
/// Only emitted after a successful `rt_value_tag(v) == TAG_LONG` guard.
///
/// # Safety
/// `v` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_unbox_long(v: *const Value) -> i64 {
    match unsafe { val_ref(v) } {
        Value::Long(n) => *n,
        _ => 0,
    }
}

/// Extract the raw `f64` payload of a `Value::Double`.
///
/// # Safety
/// `v` must be a valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_unbox_double(v: *const Value) -> f64 {
    match unsafe { val_ref(v) } {
        Value::Double(n) => *n,
        _ => 0.0,
    }
}

/// Box a raw boolean (0/1) as an interned `Value::Bool`.
#[unsafe(no_mangle)]
pub extern "C" fn rt_box_bool(b: u8) -> *const Value {
    intern_bool(b != 0)
}

/// The deoptimization sentinel: a unique, process-lifetime `Value` address
/// that compiled code can never produce as an ordinary result.
///
/// A specialized function whose entry type guard fails returns this pointer;
/// the dispatch seam (`call_jit_native`, cljrs-eval/src/apply.rs) compares
/// the raw result address against it and, on a match, re-executes the call at
/// Tier 1.  The sentinel is `Box::leak`ed — deliberately **not** a GC heap
/// object — so it can never be swept, reused, or aliased by a real result.
fn deopt_sentinel() -> *const Value {
    static PTR: OnceLock<usize> = OnceLock::new();
    *PTR.get_or_init(|| Box::leak(Box::new(Value::Nil)) as *const Value as usize) as *const Value
}

/// Address of the deopt sentinel, for the dispatch seam's pointer compare
/// (installed into `cljrs_eval::jit_state` as a hook by `cljrs_jit::init`).
pub fn deopt_sentinel_addr() -> usize {
    deopt_sentinel() as usize
}

/// Entry-guard failure: count it and return the deopt sentinel.
#[unsafe(no_mangle)]
pub extern "C" fn rt_deopt() -> *const Value {
    jit_stats::GUARD_DEOPTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    deopt_sentinel()
}

// ── Specialization / IC statistics ───────────────────────────────────────────

/// Counters behind `--jit-stats` and the Phase 10.6 milestone tests.  All
/// relaxed: they are diagnostics, never control flow.
pub mod jit_stats {
    use std::sync::atomic::{AtomicU64, Ordering};

    /// Boxed arithmetic/comparison bridge calls (`rt_add`, `rt_lt`, …).
    /// Unboxed compiled code stops bumping this — the unboxing evidence.
    pub static BOXED_ARITH_CALLS: AtomicU64 = AtomicU64::new(0);
    /// Specialization entry-guard failures (deopts to Tier 1).
    pub static GUARD_DEOPTS: AtomicU64 = AtomicU64::new(0);
    /// Keyword-constant inline-cache fills (slow path; once per call site).
    pub static KW_IC_FILLS: AtomicU64 = AtomicU64::new(0);
    /// Protocol-dispatch inline-cache hits / misses in `rt_call_ic`.
    pub static PROTO_IC_HITS: AtomicU64 = AtomicU64::new(0);
    pub static PROTO_IC_MISSES: AtomicU64 = AtomicU64::new(0);

    /// Human-readable snapshot (written by `cljrs --jit-stats`).
    pub fn snapshot() -> String {
        format!(
            "JIT specialization stats:\n  Boxed arith calls:    {}\n  Guard deopts:         {}\n  Keyword IC fills:     {}\n  Protocol IC hits:     {}\n  Protocol IC misses:   {}\n",
            BOXED_ARITH_CALLS.load(Ordering::Relaxed),
            GUARD_DEOPTS.load(Ordering::Relaxed),
            KW_IC_FILLS.load(Ordering::Relaxed),
            PROTO_IC_HITS.load(Ordering::Relaxed),
            PROTO_IC_MISSES.load(Ordering::Relaxed),
        )
    }
}

#[inline]
fn bump_boxed_arith() {
    jit_stats::BOXED_ARITH_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

// ── IC value registry (GC rooting for cached values) ─────────────────────────
//
// Inline-cache slots live in compiled-module data sections, invisible to the
// collector.  Every Value an IC hands back to compiled code (interned keyword
// constants) or holds for later dispatch (cached protocol impl fns) is
// therefore *also* owned by the global tables below, and a root tracer
// registered on each allocating thread's heap keeps those objects alive
// permanently.  Both tables are bounded: keyword constants by the program's
// distinct keyword literals, protocol IC entries by compiled call sites.

/// A `GcPtr<Value>` shared through a global table.
///
/// SAFETY: the pointee is immutable after construction and kept alive by the
/// table's root tracer; collections are stop-the-world, so cross-thread reads
/// of the pointer never race a sweep.
struct SharedVal(GcPtr<Value>);
unsafe impl Send for SharedVal {}
unsafe impl Sync for SharedVal {}

/// One protocol-dispatch inline-cache entry (per compiled call site).
struct ProtoIcEntry {
    /// Identity of the `ProtocolFn` this site last dispatched (the
    /// `GcPtr<ProtocolFn>` target address).
    callee: usize,
    /// `cljrs_value::protocol_generation()` at fill time.
    generation: u64,
    /// Dispatch type tag of the cached impl.
    tag: Arc<str>,
    /// The resolved impl fn.  Rooted by the IC root tracer.
    impl_fn: Value,
}

/// SAFETY: same argument as [`SharedVal`]; the entry is only read/replaced
/// under its `Mutex`.
struct ProtoIcCell(std::sync::Mutex<Option<ProtoIcEntry>>);
unsafe impl Send for ProtoIcCell {}
unsafe impl Sync for ProtoIcCell {}

use std::sync::RwLock;

static KW_INTERN: OnceLock<RwLock<std::collections::HashMap<String, SharedVal>>> = OnceLock::new();
static PROTO_ICS: OnceLock<RwLock<Vec<Arc<ProtoIcCell>>>> = OnceLock::new();
/// Resolved versioned values cached by compiled call sites, keyed by
/// `"<ns>/<name@commit>"`.  Versioned bindings are immutable, so entries are
/// never invalidated; the table permanently roots each boxed value.
static VERSIONED_IC: OnceLock<RwLock<std::collections::HashMap<String, SharedVal>>> =
    OnceLock::new();

fn kw_intern_table() -> &'static RwLock<std::collections::HashMap<String, SharedVal>> {
    KW_INTERN.get_or_init(|| RwLock::new(std::collections::HashMap::new()))
}

fn versioned_ic_table() -> &'static RwLock<std::collections::HashMap<String, SharedVal>> {
    VERSIONED_IC.get_or_init(|| RwLock::new(std::collections::HashMap::new()))
}

fn proto_ic_table() -> &'static RwLock<Vec<Arc<ProtoIcCell>>> {
    PROTO_ICS.get_or_init(|| RwLock::new(Vec::new()))
}

thread_local! {
    static IC_TRACER_REGISTERED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Register the IC root tracer on the current thread's heap (once per
/// thread).  Must be called by any thread that *allocates* into the IC
/// tables — only the owning thread's collector can free its objects, so the
/// tracer has to run there.
fn ensure_ic_tracer_registered() {
    IC_TRACER_REGISTERED.with(|flag| {
        if flag.get() {
            return;
        }
        flag.set(true);
        cljrs_gc::HEAP.register_root_tracer(|visitor| {
            use cljrs_gc::{GcVisitor as _, Trace as _};
            if let Some(table) = KW_INTERN.get() {
                for v in table.read().unwrap().values() {
                    visitor.visit(&v.0);
                }
            }
            if let Some(table) = VERSIONED_IC.get() {
                for v in table.read().unwrap().values() {
                    visitor.visit(&v.0);
                }
            }
            if let Some(ics) = PROTO_ICS.get() {
                for cell in ics.read().unwrap().iter() {
                    if let Some(entry) = cell.0.lock().unwrap().as_ref() {
                        entry.impl_fn.trace(visitor);
                    }
                }
            }
        });
    });
}

/// Intern a keyword `Value` by name, returning a stable, permanently rooted
/// pointer.  Every call site (and `rt_const_keyword` itself) shares one
/// allocation per distinct keyword instead of boxing a fresh
/// `Value::Keyword` per execution.
fn intern_keyword(name: &str) -> *const Value {
    if let Some(v) = kw_intern_table().read().unwrap().get(name) {
        return v.0.get() as *const Value;
    }
    ensure_ic_tracer_registered();
    let mut table = kw_intern_table().write().unwrap();
    // Re-check under the write lock so a racing intern of the same name
    // cannot replace (and un-root) a pointer already handed out.
    if let Some(v) = table.get(name) {
        return v.0.get() as *const Value;
    }
    let ptr = GcPtr::new(Value::Keyword(GcPtr::new(Keyword::parse(name))));
    let raw = ptr.get() as *const Value;
    table.insert(name.to_string(), SharedVal(ptr));
    raw
}

/// Keyword-constant inline-cache fill (slow path, once per call site).
///
/// Compiled code keeps an 8-byte writable slot per `Const::Keyword` site:
/// the fast path is an inline load + branch on the slot; on the first
/// execution the slot is zero and this fill interns the keyword, stores the
/// stable pointer into the slot, and returns it.
///
/// # Safety
/// `ptr`/`len` must describe valid UTF-8; `slot` must point to the call
/// site's 8-byte data slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_kw_ic_fill(ptr: *const u8, len: u64, slot: *mut usize) -> *const Value {
    jit_stats::KW_IC_FILLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as usize) };
    let name = std::str::from_utf8(bytes).unwrap_or("??");
    let interned = intern_keyword(name);
    // Plain (possibly racy) store: every filler stores the same interned
    // pointer, and aligned 8-byte stores cannot tear.
    unsafe { *slot = interned as usize };
    interned
}

/// Versioned-load inline-cache fill (slow path, once per call site).
///
/// Compiled code keeps an 8-byte writable slot per versioned `LoadGlobal`
/// site (`ns/name@sha`).  Versioned bindings are immutable for the lifetime
/// of the process, so the slot is filled once and never invalidated — no
/// epoch or rebind machinery is needed.  On the first execution this bridge
/// resolves the symbol through the shared versioned resolver (which may
/// lazily load the `ns@sha` namespace from embedded source or git), interns
/// the boxed value in a permanently rooted table, and stores the stable
/// pointer into the slot.
///
/// Resolution failure leaves the slot empty (so a later execution retries)
/// and surfaces a pending exception.
///
/// # Safety
/// `ns_ptr`/`name_ptr` must describe valid UTF-8; `slot` must point to the
/// call site's 8-byte data slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_load_global_versioned_ic(
    ns_ptr: *const u8,
    ns_len: u64,
    name_ptr: *const u8,
    name_len: u64,
    slot: *mut usize,
) -> *const Value {
    let ns = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize))
    };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };

    let (base_name, version) = cljrs_value::symbol::split_version(name);
    let Some(commit) = version else {
        // Codegen only emits this bridge for versioned names; tolerate a
        // stray unversioned call by deferring to the generic path (uncached).
        return unsafe { rt_load_global(ns_ptr, ns_len, name_ptr, name_len) };
    };

    let key = format!("{ns}/{name}");
    if let Some(v) = versioned_ic_table().read().unwrap().get(&key) {
        let raw = v.0.get() as *const Value;
        unsafe { *slot = raw as usize };
        return raw;
    }

    let Some((globals, current_ns)) = cljrs_env::callback::capture_eval_context() else {
        return rt_const_nil();
    };
    match cljrs_env::versioned::resolve_versioned_value(
        &globals,
        &current_ns,
        Some(ns),
        base_name,
        commit,
    ) {
        Ok(val) => {
            ensure_ic_tracer_registered();
            let mut table = versioned_ic_table().write().unwrap();
            // Re-check under the write lock so a racing resolution of the
            // same symbol cannot replace (and un-root) a pointer already
            // handed out.
            if let Some(v) = table.get(&key) {
                let raw = v.0.get() as *const Value;
                unsafe { *slot = raw as usize };
                return raw;
            }
            let ptr = GcPtr::new(val);
            let raw = ptr.get() as *const Value;
            table.insert(key, SharedVal(ptr));
            unsafe { *slot = raw as usize };
            raw
        }
        Err(e) => {
            stash_pending_exception(Value::Str(GcPtr::new(format!("{e}"))));
            rt_const_nil()
        }
    }
}

/// Invoke `callee` with already-cloned args, boxing the result and stashing a
/// thrown value exactly like `rt_call` (shared tail of the call bridges).
fn invoke_boxed(callee: &Value, args: Vec<Value>) -> *const Value {
    match cljrs_env::callback::invoke(callee, args) {
        Ok(result) => box_invoke_result(result),
        Err(cljrs_value::ValueError::Thrown(val)) => {
            stash_pending_exception(val);
            rt_const_nil()
        }
        Err(_e) => rt_const_nil(),
    }
}

/// `rt_call` with a per-call-site inline cache for protocol dispatch.
///
/// For a `Value::ProtocolFn` callee, the uncached path computes the dispatch
/// type tag (allocating an `Arc<str>`), locks the protocol's impl map, and
/// performs two hash lookups — on *every* call.  This bridge caches the
/// resolved `(callee, dispatch tag) → impl fn` per call site, validated by
/// the global protocol generation (bumped on every `extend-type` /
/// `extend-protocol` / inline impl), and on a hit invokes the impl directly.
/// Non-protocol callees fall through to `rt_call` unchanged.
///
/// `slot` holds `0` (empty) or `index + 1` into the global IC entry table —
/// never a GC pointer, so compiled modules stay free of GC roots.
///
/// # Safety
/// Same contract as `rt_call`, plus `slot` must point to the call site's
/// 8-byte data slot.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_call_ic(
    callee: *const Value,
    args: *const *const Value,
    nargs: u64,
    slot: *mut usize,
) -> *const Value {
    let callee_ref = unsafe { val_ref(callee) };
    if let Value::ProtocolFn(pf) = callee_ref
        && nargs >= 1
    {
        let arg_slice = unsafe { std::slice::from_raw_parts(args, nargs as usize) };
        let dispatch_val = unsafe { val_ref(arg_slice[0]) };
        let generation = cljrs_value::protocol_generation();
        let callee_id = pf.get() as *const _ as usize;

        // Fast path: validate the cached entry.
        let idx = unsafe { *slot };
        if idx != 0 {
            let cell = {
                let table = proto_ic_table().read().unwrap();
                table.get(idx - 1).cloned()
            };
            if let Some(cell) = cell {
                let guard = cell.0.lock().unwrap();
                if let Some(entry) = guard.as_ref()
                    && entry.callee == callee_id
                    && entry.generation == generation
                    && cljrs_env::apply::type_tag_matches(dispatch_val, &entry.tag)
                {
                    let impl_fn = entry.impl_fn.clone();
                    drop(guard);
                    jit_stats::PROTO_IC_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    return invoke_boxed(&impl_fn, unsafe { collect_args(args, nargs as i64) });
                }
            }
        }

        // Miss: resolve the impl the same way `apply_value` does, refill the
        // cache, and invoke directly.  An unimplemented type falls through to
        // `rt_call` for the canonical error path.
        jit_stats::PROTO_IC_MISSES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tag = cljrs_env::apply::type_tag_of(dispatch_val);
        let pf_ref = pf.get();
        let impl_fn = {
            let impls = pf_ref.protocol.get().impls.lock().unwrap();
            impls
                .get(tag.as_ref())
                .and_then(|m| m.get(pf_ref.method_name.as_ref()))
                .cloned()
        };
        if let Some(impl_fn) = impl_fn {
            ensure_ic_tracer_registered();
            let entry = ProtoIcEntry {
                callee: callee_id,
                generation,
                tag,
                impl_fn: impl_fn.clone(),
            };
            if idx != 0 {
                let cell = {
                    let table = proto_ic_table().read().unwrap();
                    table.get(idx - 1).cloned()
                };
                if let Some(cell) = cell {
                    *cell.0.lock().unwrap() = Some(entry);
                }
            } else {
                let mut table = proto_ic_table().write().unwrap();
                table.push(Arc::new(ProtoIcCell(std::sync::Mutex::new(Some(entry)))));
                let new_idx = table.len(); // index + 1
                drop(table);
                unsafe { *slot = new_idx };
            }
            return invoke_boxed(&impl_fn, unsafe { collect_args(args, nargs as i64) });
        }
    }
    unsafe { rt_call(callee, args, nargs) }
}

// ── Async state machine (Phase H) ────────────────────────────────────────────
//
// A compiled `^:async` poll function receives a `*mut CljxStateMachine` as its
// hidden leading parameter and drives the state machine through these bridges.
// They mirror `cljrs_async::state_machine`'s native helpers but with the
// `extern "C"` / `*const Value` ABI compiled code speaks.

use cljrs_async::state_machine::{CljxStateMachine, Readiness, check_ready};

/// Read the current resume state (for the poll function's `switch(state)`
/// prologue).
///
/// # Safety
/// `sm` must point to a live `CljxStateMachine`.
#[unsafe(no_mangle)]
pub extern "C" fn rt_sm_state(sm: *mut CljxStateMachine) -> i32 {
    let sm = unsafe { &*sm };
    sm.state
}

/// Set the resume state before suspending.
///
/// # Safety
/// `sm` must point to a live `CljxStateMachine`.
#[unsafe(no_mangle)]
pub extern "C" fn rt_sm_set_state(sm: *mut CljxStateMachine, state: i32) {
    let sm = unsafe { &mut *sm };
    sm.state = state;
}

/// Save a live value into a state-machine slot before suspending.
///
/// # Safety
/// `sm` must be live, `slot` in range, and `val` a valid `Value` pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_state_store(sm: *mut CljxStateMachine, slot: u32, val: *const Value) {
    let v = unsafe { val_ref(val) }.clone();
    let sm = unsafe { &mut *sm };
    sm.slots[slot as usize] = v;
}

/// Restore a live value from a state-machine slot after a resume.  Returns a
/// *fresh* GC-boxed copy, decoupled from the slot: a loaded value can flow
/// through a `recur` phi and outlive a later `rt_state_store` to the same slot
/// (e.g. a loop counter reloaded, then the slot overwritten with `(inc i)`
/// before the old value is used), so it must not alias the mutable slot.
///
/// # Safety
/// `sm` must be live and `slot` in range.
#[unsafe(no_mangle)]
pub extern "C" fn rt_state_load(sm: *mut CljxStateMachine, slot: u32) -> *const Value {
    let sm = unsafe { &*sm };
    box_val(sm.slots[slot as usize].clone())
}

/// Register the value being awaited at a suspend point.
///
/// # Safety
/// `sm` must be live and `val` a valid `Value` pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_async_register(sm: *mut CljxStateMachine, val: *const Value) {
    let v = unsafe { val_ref(val) }.clone();
    let sm = unsafe { &mut *sm };
    sm.pending = v;
}

/// Check whether the registered (`pending`) value has resolved.  Returns the
/// poll code: `0` pending, `1` ready, `2` failed.  On ready/failed the resolved
/// (or thrown) value replaces `pending`, so [`rt_async_take_result`] returns it.
///
/// # Safety
/// `sm` must point to a live `CljxStateMachine`.
#[unsafe(no_mangle)]
pub extern "C" fn rt_async_poll_ready(sm: *mut CljxStateMachine) -> i32 {
    let sm = unsafe { &mut *sm };
    match check_ready(&sm.pending) {
        Readiness::Pending => 0,
        Readiness::Ready(v) => {
            sm.pending = v;
            1
        }
        Readiness::Failed(e) => {
            sm.pending = e;
            2
        }
    }
}

/// Return the resolved value stashed by [`rt_async_poll_ready`], as a *fresh*
/// GC-boxed copy decoupled from `pending` (which the next suspend overwrites) —
/// the awaited value may flow through a `recur` phi past that point.
///
/// # Safety
/// `sm` must point to a live `CljxStateMachine`.
#[unsafe(no_mangle)]
pub extern "C" fn rt_async_take_result(sm: *mut CljxStateMachine) -> *const Value {
    let sm = unsafe { &*sm };
    box_val(sm.pending.clone())
}

/// Store the poll function's final result into the state machine (used by
/// `Return` in a poll fn).  The result lands in `pending` — the GC-rooted slot
/// the `CompiledAsyncTask` adapter reads on completion — so the result never
/// crosses the FFI boundary as a raw pointer.
///
/// # Safety
/// `sm` must be live and `val` a valid `Value` pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_async_set_result(sm: *mut CljxStateMachine, val: *const Value) {
    let v = unsafe { val_ref(val) }.clone();
    let sm = unsafe { &mut *sm };
    sm.pending = v;
}

#[cfg(test)]
mod async_sm_tests {
    use super::*;
    use cljrs_async::state_machine::CljxStateMachine;

    extern "C" fn dummy_poll(_sm: *mut CljxStateMachine) -> i32 {
        0
    }

    fn machine(n_slots: usize) -> Box<CljxStateMachine> {
        Box::new(CljxStateMachine::new(dummy_poll, n_slots, vec![]))
    }

    #[test]
    fn state_roundtrips() {
        let mut sm = machine(0);
        let p: *mut CljxStateMachine = &mut *sm;
        assert_eq!(rt_sm_state(p), 0);
        rt_sm_set_state(p, 3);
        assert_eq!(rt_sm_state(p), 3);
    }

    #[test]
    fn slot_store_and_load_roundtrip() {
        let mut sm = machine(2);
        let p: *mut CljxStateMachine = &mut *sm;
        let v = rt_const_long(99);
        rt_state_store(p, 1, v);
        let loaded = rt_state_load(p, 1);
        assert!(matches!(unsafe { &*loaded }, Value::Long(99)));
    }

    #[test]
    fn register_and_poll_ready_on_plain_value() {
        let mut sm = machine(1);
        let p: *mut CljxStateMachine = &mut *sm;
        // Awaiting a non-future value is immediately ready (identity).
        rt_async_register(p, rt_const_long(7));
        assert_eq!(rt_async_poll_ready(p), 1);
        let r = rt_async_take_result(p);
        assert!(matches!(unsafe { &*r }, Value::Long(7)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_const_nil() {
        let p = rt_const_nil();
        assert!(!p.is_null());
        assert!(matches!(unsafe { &*p }, Value::Nil));
    }

    #[test]
    fn test_const_long() {
        let p = rt_const_long(42);
        assert!(matches!(unsafe { &*p }, Value::Long(42)));
    }

    #[test]
    fn test_truthiness() {
        assert_eq!(unsafe { rt_truthiness(rt_const_nil()) }, 0);
        assert_eq!(unsafe { rt_truthiness(rt_const_false()) }, 0);
        assert_eq!(unsafe { rt_truthiness(rt_const_true()) }, 1);
        assert_eq!(unsafe { rt_truthiness(rt_const_long(0)) }, 1);
    }

    #[test]
    fn test_add_longs() {
        let a = rt_const_long(10);
        let b = rt_const_long(32);
        let result = unsafe { rt_add(a, b) };
        assert!(matches!(unsafe { &*result }, Value::Long(42)));
    }

    #[test]
    fn test_eq() {
        let a = rt_const_long(5);
        let b = rt_const_long(5);
        let result = unsafe { rt_eq(a, b) };
        assert!(matches!(unsafe { &*result }, Value::Bool(true)));

        let c = rt_const_long(6);
        let result2 = unsafe { rt_eq(a, c) };
        assert!(matches!(unsafe { &*result2 }, Value::Bool(false)));
    }

    #[test]
    fn test_alloc_vector() {
        let elems = [rt_const_long(1), rt_const_long(2), rt_const_long(3)];
        let v = unsafe { rt_alloc_vector(elems.as_ptr(), 3) };
        let val = unsafe { val_ref(v) };
        if let Value::Vector(vec) = val {
            assert_eq!(vec.get().count(), 3);
        } else {
            panic!("expected vector");
        }
    }

    #[test]
    fn test_count() {
        let elems = [rt_const_long(1), rt_const_long(2)];
        let v = unsafe { rt_alloc_vector(elems.as_ptr(), 2) };
        let n = unsafe { rt_count(v) };
        assert!(matches!(unsafe { &*n }, Value::Long(2)));
    }

    #[test]
    fn test_count_cons_chain() {
        // Regression: `count` over a cons/seq chain (e.g. the result of
        // `filter`/`map`) must walk the chain, not return 0.
        let nil = rt_const_nil();
        let c1 = unsafe { rt_alloc_cons(rt_const_long(3), nil) };
        let c2 = unsafe { rt_alloc_cons(rt_const_long(2), c1) };
        let c3 = unsafe { rt_alloc_cons(rt_const_long(1), c2) };
        let n = unsafe { rt_count(c3) };
        assert!(
            matches!(unsafe { &*n }, Value::Long(3)),
            "count of a 3-element cons chain must be 3, got {:?}",
            unsafe { &*n }
        );
    }
}
