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
    CljxCons, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value,
};

use std::sync::Arc;

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

// ── Constants ───────────────────────────────────────────────────────────────

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_nil() -> *const Value {
    box_val(Value::Nil)
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_true() -> *const Value {
    box_val(Value::Bool(true))
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_false() -> *const Value {
    box_val(Value::Bool(false))
}

#[unsafe(no_mangle)]
pub extern "C" fn rt_const_long(n: i64) -> *const Value {
    box_val(Value::Long(n))
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
/// # Safety
/// `ptr` must point to valid UTF-8 data of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_const_keyword(ptr: *const u8, len: u64) -> *const Value {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as *const () as usize) };
    let name = std::str::from_utf8(bytes).unwrap_or("??");
    box_val(Value::Keyword(GcPtr::new(Keyword::simple(name))))
}

/// Create a symbol.  `ptr`/`len` is the simple name.
///
/// # Safety
/// `ptr` must point to valid UTF-8 data of `len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_const_symbol(ptr: *const u8, len: u64) -> *const Value {
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len as *const () as usize) };
    let name = std::str::from_utf8(bytes).unwrap_or("??");
    box_val(Value::Symbol(GcPtr::new(Symbol {
        namespace: None,
        name: Arc::from(name),
    })))
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

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_add(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => box_val(Value::Long(x.wrapping_add(*y))),
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
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => box_val(Value::Long(x.wrapping_sub(*y))),
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
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => box_val(Value::Long(x.wrapping_mul(*y))),
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
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) if *y != 0 => box_val(Value::Long(x / y)),
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
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    match (a, b) {
        (Value::Long(x), Value::Long(y)) if *y != 0 => box_val(Value::Long(x % y)),
        (Value::Double(x), Value::Double(y)) => box_val(Value::Double(x % y)),
        _ => rt_const_nil(),
    }
}

// ── Comparison ──────────────────────────────────────────────────────────────

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_eq(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    box_val(Value::Bool(a == b))
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lt(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x < y,
        (Value::Double(x), Value::Double(y)) => x < y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) < *y,
        (Value::Double(x), Value::Long(y)) => *x < (*y as f64),
        _ => false,
    };
    box_val(Value::Bool(result))
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_gt(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x > y,
        (Value::Double(x), Value::Double(y)) => x > y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) > *y,
        (Value::Double(x), Value::Long(y)) => *x > (*y as f64),
        _ => false,
    };
    box_val(Value::Bool(result))
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_lte(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x <= y,
        (Value::Double(x), Value::Double(y)) => x <= y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) <= *y,
        (Value::Double(x), Value::Long(y)) => *x <= (*y as f64),
        _ => false,
    };
    box_val(Value::Bool(result))
}

/// # Safety
/// Both pointers must be valid `*const Value`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_gte(a: *const Value, b: *const Value) -> *const Value {
    let a = unsafe { val_ref(a) };
    let b = unsafe { val_ref(b) };
    let result = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x >= y,
        (Value::Double(x), Value::Double(y)) => x >= y,
        (Value::Long(x), Value::Double(y)) => (*x as f64) >= *y,
        (Value::Double(x), Value::Long(y)) => *x >= (*y as f64),
        _ => false,
    };
    box_val(Value::Bool(result))
}

// ── Collection construction ─────────────────────────────────────────────────

/// Allocate a vector from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_vector(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let slice = unsafe { std::slice::from_raw_parts(elems, n) };
    let items: Vec<Value> = slice.iter().map(|p| unsafe { val_ref(*p) }.clone()).collect();
    box_val(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        items,
    ))))
}

/// Allocate a map from `n` key-value pairs (2*n pointers: k0, v0, k1, v1, ...).
///
/// # Safety
/// `pairs` must point to `2*n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_map(pairs: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let slice = unsafe { std::slice::from_raw_parts(pairs, n * 2) };
    let kv_pairs: Vec<(Value, Value)> = (0..n)
        .map(|i| {
            let k = unsafe { val_ref(slice[i * 2]) }.clone();
            let v = unsafe { val_ref(slice[i * 2 + 1]) }.clone();
            (k, v)
        })
        .collect();
    box_val(Value::Map(MapValue::from_pairs(kv_pairs)))
}

/// Allocate a set from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_set(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let slice = unsafe { std::slice::from_raw_parts(elems, n) };
    let items: Vec<Value> = slice.iter().map(|p| unsafe { val_ref(*p) }.clone()).collect();
    let set = PersistentHashSet::from_iter(items);
    box_val(Value::Set(SetValue::Hash(GcPtr::new(set))))
}

/// Allocate a list from `n` element pointers.
///
/// # Safety
/// `elems` must point to `n` valid `*const Value` pointers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_list(elems: *const *const Value, n: u64) -> *const Value {
    let n = n as usize;
    let slice = unsafe { std::slice::from_raw_parts(elems, n) };
    let items: Vec<Value> = slice.iter().map(|p| unsafe { val_ref(*p) }.clone()).collect();
    box_val(Value::List(GcPtr::new(PersistentList::from_iter(items))))
}

/// Allocate a cons cell.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_cons(
    head: *const Value,
    tail: *const Value,
) -> *const Value {
    let h = unsafe { val_ref(head) }.clone();
    let t = unsafe { val_ref(tail) }.clone();
    box_val(Value::Cons(GcPtr::new(CljxCons { head: h, tail: t })))
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
        Value::Vector(v) => {
            if let Value::Long(i) = key {
                v.get().nth(*i as *const () as usize).cloned()
            } else {
                None
            }
        }
        _ => None,
    };
    box_val(result.unwrap_or(Value::Nil))
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
        _ => 0,
    };
    box_val(Value::Long(n as i64))
}

/// `(first coll)`.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_first(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let result = match coll {
        Value::List(l) => l.get().first().cloned().unwrap_or(Value::Nil),
        Value::Vector(v) => v.get().nth(0).cloned().unwrap_or(Value::Nil),
        Value::Cons(c) => c.get().head.clone(),
        Value::Nil => Value::Nil,
        _ => Value::Nil,
    };
    box_val(result)
}

/// `(rest coll)`.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_rest(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    let result = match coll {
        Value::List(l) => {
            let rest = l.get().rest();
            Value::List(GcPtr::new((*rest).clone()))
        }
        Value::Cons(c) => c.get().tail.clone(),
        Value::Nil => Value::List(GcPtr::new(PersistentList::empty())),
        _ => Value::List(GcPtr::new(PersistentList::empty())),
    };
    box_val(result)
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
            box_val(Value::Map(new_map))
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
        Value::Vector(v) => box_val(Value::Vector(GcPtr::new(v.get().conj(val)))),
        Value::List(l) => {
            let new_list = PersistentList::cons(val, Arc::new((*l.get()).clone()));
            box_val(Value::List(GcPtr::new(new_list)))
        }
        Value::Set(s) => {
            let new_set = s.conj(val);
            box_val(Value::Set(new_set))
        }
        _ => rt_const_nil(),
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
    let ns = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize)) };
    let name = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize)) };

    // Look up in the global environment via the thread-local eval context.
    if let Some((globals, current_ns)) = cljrs_eval::callback::capture_eval_context() {
        // Try the specified namespace first.
        if let Some(val) = globals.lookup_in_ns(ns, name) {
            return box_val(val);
        }
        // If ns is the current namespace, also check refers (e.g. clojure.core).
        if ns == current_ns.as_ref()
            && let Some(val) = globals.lookup_in_ns(&current_ns, name)
        {
            return box_val(val);
        }
    }
    rt_const_nil()
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
    let ns = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize)) };
    let name = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize)) };
    let val = unsafe { val_ref(val) }.clone();

    if let Some((globals, _)) = cljrs_eval::callback::capture_eval_context() {
        let var = globals.intern(ns, Arc::from(name), val);
        box_val(Value::Var(var))
    } else {
        rt_const_nil()
    }
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
    let arg_slice = unsafe { std::slice::from_raw_parts(args, nargs) };
    let arg_values: Vec<Value> = arg_slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();

    match cljrs_eval::callback::invoke(callee, arg_values) {
        Ok(result) => box_val(result),
        Err(_e) => rt_const_nil(), // TODO: proper error handling / unwinding
    }
}

/// Deref a value (atoms, vars, delays, etc.).
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_deref(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) }.clone();
    match cljrs_eval::eval::deref_value(v) {
        Ok(result) => box_val(result),
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
    // PrintValue uses non-readable format (like Clojure's print/println).
    println!("{}", PrintValue(v));
    rt_const_nil()
}

/// `(pr v)`.
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_pr(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    // Value's Display impl uses readable format (like Clojure's pr).
    print!("{v}");
    rt_const_nil()
}

// ── Type checks ─────────────────────────────────────────────────────────────

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_nil(v: *const Value) -> *const Value {
    box_val(Value::Bool(matches!(unsafe { val_ref(v) }, Value::Nil)))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_seq(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(
        v,
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_)
    )))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_vector(v: *const Value) -> *const Value {
    box_val(Value::Bool(matches!(
        unsafe { val_ref(v) },
        Value::Vector(_)
    )))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_is_map(v: *const Value) -> *const Value {
    box_val(Value::Bool(matches!(
        unsafe { val_ref(v) },
        Value::Map(_)
    )))
}

// ── Identity ────────────────────────────────────────────────────────────────

/// `(identical? a b)` — pointer identity.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_identical(a: *const Value, b: *const Value) -> *const Value {
    box_val(Value::Bool(std::ptr::eq(a, b)))
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
    std::hint::black_box(rt_div as *const () as usize);
    std::hint::black_box(rt_rem as *const () as usize);
    std::hint::black_box(rt_eq as *const () as usize);
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
}
