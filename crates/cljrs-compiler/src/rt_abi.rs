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
        Value::TypeInstance(ti) => ti.get().fields.get(key),
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
        Value::TypeInstance(ti) => {
            let mut fields = ti.get().fields.clone();
            fields = fields.assoc(k, v);
            box_val(Value::TypeInstance(GcPtr::new(TypeInstance {
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
        // Try resolving ns as an alias in the current namespace.
        if let Some(resolved_ns) = globals.resolve_alias(&current_ns, ns)
            && let Some(val) = globals.lookup_in_ns(&resolved_ns, name)
        {
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
    cljrs_eval::builtins::emit_output_ln(&format!("{}", PrintValue(v)));
    rt_const_nil()
}

/// `(pr v)`.
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_pr(v: *const Value) -> *const Value {
    let v = unsafe { val_ref(v) };
    cljrs_eval::builtins::emit_output(&format!("{v}"));
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
    cljrs_eval::builtins::emit_output_ln(&s);
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
    cljrs_eval::builtins::push_output_capture();
    let _result = cljrs_eval::callback::invoke(&f, vec![]);
    let captured = cljrs_eval::builtins::pop_output_capture().unwrap_or_default();
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

// ── Higher-order functions ───────────────────────────────────────────────────

/// Helper: look up a global function by namespace and name, then call it.
fn call_global_fn(ns: &str, name: &str, args: Vec<Value>) -> *const Value {
    if let Some((globals, _)) = cljrs_eval::callback::capture_eval_context()
        && let Some(val) = globals.lookup_in_ns(ns, name)
    {
        match cljrs_eval::callback::invoke(&val, args) {
            Ok(result) => box_val(result),
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
    let to = unsafe { val_ref(to) }.clone();
    let from = unsafe { val_ref(from) }.clone();
    call_global_fn("clojure.core", "into", vec![to, from])
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
    box_val(Value::Bool(matches!(
        val,
        Value::Long(_) | Value::Double(_)
    )))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_string(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(val, Value::Str(_))))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_keyword(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(val, Value::Keyword(_))))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_symbol(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(val, Value::Symbol(_))))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_bool(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(val, Value::Bool(_))))
}

/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_is_int(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    box_val(Value::Bool(matches!(val, Value::Long(_))))
}

// ── Additional I/O ──────────────────────────────────────────────────────────

/// `(prn x)` — print readably + newline
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_prn(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    cljrs_eval::builtins::emit_output_ln(&format!("{val}"));
    box_val(Value::Nil)
}

/// `(print x)` — print without newline
///
/// # Safety
/// `v` must be a valid pointer.
#[unsafe(no_mangle)]
pub extern "C" fn rt_print(v: *const Value) -> *const Value {
    let val = unsafe { val_ref(v) };
    cljrs_eval::builtins::emit_output(&format!("{}", PrintValue(val)));
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
