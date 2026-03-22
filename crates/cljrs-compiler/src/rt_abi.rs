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
use cljrs_value::{CljxCons, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value};

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
    let items: Vec<Value> = slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();
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
    let items: Vec<Value> = slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();
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
    let items: Vec<Value> = slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();
    box_val(Value::List(GcPtr::new(PersistentList::from_iter(items))))
}

/// Allocate a cons cell.
///
/// # Safety
/// Both pointers must be valid.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_alloc_cons(head: *const Value, tail: *const Value) -> *const Value {
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
        Value::Vector(v) => {
            if v.get().count() <= 1 {
                Value::List(GcPtr::new(PersistentList::empty()))
            } else {
                let items: Vec<Value> = v.get().iter().skip(1).cloned().collect();
                Value::List(GcPtr::new(PersistentList::from_iter(items)))
            }
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

// ── Function/closure construction ───────────────────────────────────────────

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

/// Create a multi-arity compiled function value.
///
/// `fn_ptrs` is an array of `n_arities` function pointers (one per arity).
/// `param_counts` is an array of `n_arities` parameter counts (user params, not including captures).
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
    n_arities: u64,
    captures: *const *const Value,
    ncaptures: u64,
) -> *const Value {
    let name_str = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let name: Arc<str> = Arc::from(name_str);
    let n_arities = n_arities as usize;
    let ncaptures = ncaptures as usize;

    // Build arity table: Vec<(fn_addr, user_param_count)>
    let fn_ptr_slice = unsafe { std::slice::from_raw_parts(fn_ptrs, n_arities) };
    let param_count_slice = unsafe { std::slice::from_raw_parts(param_counts_ptr, n_arities) };
    let arity_table: Vec<(usize, usize)> = fn_ptr_slice
        .iter()
        .zip(param_count_slice.iter())
        .map(|(&fp, &pc)| (fp as usize, pc as usize))
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
    let min_params = arity_table.iter().map(|(_, pc)| *pc).min().unwrap_or(0);
    let max_params = arity_table.iter().map(|(_, pc)| *pc).max().unwrap_or(0);
    let arity = if min_params == max_params {
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
            // Find matching arity.
            let matched = arity_table.iter().find(|(_, pc)| *pc == argc);
            let (fn_addr, _user_pc) = match matched {
                Some(entry) => *entry,
                None => {
                    let counts: Vec<String> =
                        arity_table.iter().map(|(_, pc)| pc.to_string()).collect();
                    return Err(cljrs_value::ValueError::ArityError {
                        name: fn_name.to_string(),
                        expected: counts.join(" or "),
                        got: argc,
                    });
                }
            };
            let total_params = ncaptures + argc;

            // Build: captures + args
            let mut all_ptrs: Vec<*const Value> = Vec::with_capacity(total_params);
            for cap in &captured_values {
                all_ptrs.push(box_val(cap.clone()));
            }
            for arg in args {
                all_ptrs.push(box_val(arg.clone()));
            }

            let result_ptr = unsafe { rt_call_compiled(fn_addr, all_ptrs.as_ptr(), total_params) };
            Ok(unsafe { val_ref(result_ptr) }.clone())
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
        _ => {
            // For functions with more than 8 params, fall back to rt_call style.
            // This shouldn't happen in practice for most Clojure code.
            eprintln!("[rt] warning: compiled function with {nargs} args, falling back");
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
    let ns = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(ns_ptr, ns_len as usize))
    };
    let name = unsafe {
        std::str::from_utf8_unchecked(std::slice::from_raw_parts(name_ptr, name_len as usize))
    };
    let val = unsafe { val_ref(val) }.clone();

    if let Some((globals, _)) = cljrs_eval::callback::capture_eval_context() {
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

    if let Some((globals, current_ns)) = cljrs_eval::callback::capture_eval_context() {
        // Try the specified namespace (interns + refers).
        if let Some(var) = globals.lookup_var_in_ns(ns, name) {
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
    let arg_slice = unsafe { std::slice::from_raw_parts(args, nargs) };
    let arg_values: Vec<Value> = arg_slice
        .iter()
        .map(|p| unsafe { val_ref(*p) }.clone())
        .collect();

    match cljrs_eval::callback::invoke(callee, arg_values) {
        Ok(result) => box_val(result),
        Err(cljrs_value::ValueError::Thrown(val)) => {
            // Store thrown exception in thread-local for rt_try to find.
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
    box_val(Value::Bool(matches!(unsafe { val_ref(v) }, Value::Map(_))))
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
        Value::Map(map) => box_val(Value::Map(map.dissoc(k))),
        _ => box_val(m.clone()),
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
        Value::Set(s) => box_val(Value::Set(s.disj(val))),
        _ => box_val(set.clone()),
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
    let result = match coll {
        Value::Vector(v) => v.get().nth(i).cloned(),
        Value::List(l) => l.get().iter().nth(i).cloned(),
        Value::Str(s) => s.get().chars().nth(i).map(Value::Char),
        _ => None,
    };
    box_val(result.unwrap_or(Value::Nil))
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
    box_val(Value::Bool(result))
}

// ── Sequence operations (extended) ──────────────────────────────────────────

/// `(seq coll)` — returns a seq on the collection, or nil if empty.
///
/// # Safety
/// `coll` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_seq(coll: *const Value) -> *const Value {
    let coll = unsafe { val_ref(coll) };
    match coll {
        Value::Nil => rt_const_nil(),
        Value::List(l) => {
            if l.get().is_empty() {
                rt_const_nil()
            } else {
                box_val(coll.clone())
            }
        }
        Value::Vector(v) => {
            if v.get().count() == 0 {
                rt_const_nil()
            } else {
                // Convert vector to list for seq iteration.
                let items: Vec<Value> = v.get().iter().cloned().collect();
                box_val(Value::List(GcPtr::new(PersistentList::from_iter(items))))
            }
        }
        Value::Map(m) => {
            if m.count() == 0 {
                rt_const_nil()
            } else {
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
        Value::Cons(_) | Value::LazySeq(_) => box_val(coll.clone()),
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
        fn force(&self) -> Value {
            match cljrs_eval::callback::invoke(&self.0, vec![]) {
                Ok(v) => v,
                Err(_) => Value::Nil,
            }
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
                match cljrs_eval::callback::invoke(&f, args) {
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

    match cljrs_eval::callback::invoke(&f, args) {
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
            if !cljrs_eval::dynamics::set_thread_local(var, val.clone()) {
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
            frame.insert(cljrs_eval::dynamics::var_key_of(var), val);
        }
    }

    let _guard = cljrs_eval::dynamics::push_frame(frame);
    let body = unsafe { val_ref(body_fn) }.clone();

    match cljrs_eval::callback::invoke(&body, vec![]) {
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

    // Call the body.
    let body_result = cljrs_eval::callback::invoke(&body, vec![]);

    // Check for thrown exception (set by rt_throw in compiled code).
    let ret = if let Some(thrown_ptr) = take_pending_exception() {
        // Exception was thrown from compiled code.
        if !matches!(catch, Value::Nil) {
            let thrown_val = unsafe { val_ref(thrown_ptr) }.clone();
            match cljrs_eval::callback::invoke(&catch, vec![thrown_val]) {
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
                // Body returned a ValueError (e.g. thrown via interpreter).
                if !matches!(catch, Value::Nil) {
                    let thrown_val = match val_err {
                        cljrs_value::ValueError::Thrown(v) => v,
                        other => Value::Str(GcPtr::new(other.to_string())),
                    };
                    match cljrs_eval::callback::invoke(&catch, vec![thrown_val]) {
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
        let _ = cljrs_eval::callback::invoke(&finally, vec![]);
    }

    ret
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
    std::hint::black_box(rt_make_fn as *const () as usize);
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
    std::hint::black_box(rt_set_bang as *const () as usize);
    std::hint::black_box(rt_with_bindings as *const () as usize);
    std::hint::black_box(rt_load_var as *const () as usize);
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
