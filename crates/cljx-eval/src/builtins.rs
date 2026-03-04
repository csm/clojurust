//! All native (Rust) built-in functions registered in `clojure.core`.

use std::sync::Arc;

use cljx_gc::GcPtr;
use cljx_value::{
    Agent, AgentFn, AgentMsg, Arity, Atom, CljxCons, CljxPromise, FutureState, Keyword, MapValue,
    NativeFn, PersistentHashSet, PersistentList, PersistentVector, Symbol, TypeInstance, Value,
    ValueError, ValueResult, Volatile,
};

use crate::env::GlobalEnv;

// ── Registration ──────────────────────────────────────────────────────────────

pub fn register_all(globals: &Arc<GlobalEnv>, ns: &str) {
    let fns: Vec<(&str, Arity, fn(&[Value]) -> ValueResult<Value>)> = vec![
        // Arithmetic
        ("+", Arity::Variadic { min: 0 }, builtin_add),
        ("-", Arity::Variadic { min: 1 }, builtin_sub),
        ("*", Arity::Variadic { min: 0 }, builtin_mul),
        ("/", Arity::Variadic { min: 1 }, builtin_div),
        ("mod", Arity::Fixed(2), builtin_mod),
        ("rem", Arity::Fixed(2), builtin_rem),
        ("quot", Arity::Fixed(2), builtin_quot),
        ("inc", Arity::Fixed(1), builtin_inc),
        ("dec", Arity::Fixed(1), builtin_dec),
        ("max", Arity::Variadic { min: 1 }, builtin_max),
        ("min", Arity::Variadic { min: 1 }, builtin_min),
        ("abs", Arity::Fixed(1), builtin_abs),
        // Comparison
        ("=", Arity::Variadic { min: 1 }, builtin_eq),
        ("not=", Arity::Variadic { min: 1 }, builtin_not_eq),
        ("<", Arity::Variadic { min: 1 }, builtin_lt),
        (">", Arity::Variadic { min: 1 }, builtin_gt),
        ("<=", Arity::Variadic { min: 1 }, builtin_lte),
        (">=", Arity::Variadic { min: 1 }, builtin_gte),
        ("identical?", Arity::Fixed(2), builtin_identical),
        ("compare", Arity::Fixed(2), builtin_compare),
        // Predicates
        ("nil?", Arity::Fixed(1), builtin_nil_q),
        ("zero?", Arity::Fixed(1), builtin_zero_q),
        ("pos?", Arity::Fixed(1), builtin_pos_q),
        ("neg?", Arity::Fixed(1), builtin_neg_q),
        ("not", Arity::Fixed(1), builtin_not),
        ("true?", Arity::Fixed(1), builtin_true_q),
        ("false?", Arity::Fixed(1), builtin_false_q),
        ("number?", Arity::Fixed(1), builtin_number_q),
        ("integer?", Arity::Fixed(1), builtin_integer_q),
        ("float?", Arity::Fixed(1), builtin_float_q),
        ("string?", Arity::Fixed(1), builtin_string_q),
        ("keyword?", Arity::Fixed(1), builtin_keyword_q),
        ("symbol?", Arity::Fixed(1), builtin_symbol_q),
        ("fn?", Arity::Fixed(1), builtin_fn_q),
        ("seq?", Arity::Fixed(1), builtin_seq_q),
        ("map?", Arity::Fixed(1), builtin_map_q),
        ("vector?", Arity::Fixed(1), builtin_vector_q),
        ("set?", Arity::Fixed(1), builtin_set_q),
        ("coll?", Arity::Fixed(1), builtin_coll_q),
        ("boolean?", Arity::Fixed(1), builtin_boolean_q),
        ("char?", Arity::Fixed(1), builtin_char_q),
        ("var?", Arity::Fixed(1), builtin_var_q),
        ("atom?", Arity::Fixed(1), builtin_atom_q),
        ("empty?", Arity::Fixed(1), builtin_empty_q),
        ("even?", Arity::Fixed(1), builtin_even_q),
        ("odd?", Arity::Fixed(1), builtin_odd_q),
        // Collections (non-HOF)
        ("list", Arity::Variadic { min: 0 }, builtin_list),
        ("list*", Arity::Variadic { min: 1 }, builtin_list_star),
        ("vector", Arity::Variadic { min: 0 }, builtin_vector),
        ("hash-map", Arity::Variadic { min: 0 }, builtin_hash_map),
        ("hash-set", Arity::Variadic { min: 0 }, builtin_hash_set),
        ("conj", Arity::Variadic { min: 1 }, builtin_conj),
        ("assoc", Arity::Variadic { min: 3 }, builtin_assoc),
        ("dissoc", Arity::Variadic { min: 1 }, builtin_dissoc),
        ("get", Arity::Variadic { min: 2 }, builtin_get),
        ("get-in", Arity::Variadic { min: 2 }, builtin_get_in),
        ("count", Arity::Fixed(1), builtin_count),
        ("seq", Arity::Fixed(1), builtin_seq),
        ("first", Arity::Fixed(1), builtin_first),
        ("rest", Arity::Fixed(1), builtin_rest),
        ("next", Arity::Fixed(1), builtin_next),
        ("cons", Arity::Fixed(2), builtin_cons),
        ("nth", Arity::Variadic { min: 2 }, builtin_nth),
        ("last", Arity::Fixed(1), builtin_last),
        ("reverse", Arity::Fixed(1), builtin_reverse),
        ("concat", Arity::Variadic { min: 0 }, builtin_concat),
        ("keys", Arity::Fixed(1), builtin_keys),
        ("vals", Arity::Fixed(1), builtin_vals),
        ("contains?", Arity::Fixed(2), builtin_contains_q),
        ("merge", Arity::Variadic { min: 0 }, builtin_merge),
        ("into", Arity::Fixed(2), builtin_into),
        ("empty", Arity::Fixed(1), builtin_empty),
        ("vec", Arity::Fixed(1), builtin_vec),
        ("set", Arity::Fixed(1), builtin_set_fn),
        ("disj", Arity::Variadic { min: 1 }, builtin_disj),
        ("peek", Arity::Fixed(1), builtin_peek),
        ("pop", Arity::Fixed(1), builtin_pop),
        ("subvec", Arity::Variadic { min: 2 }, builtin_subvec),
        ("assoc-in", Arity::Fixed(3), builtin_assoc_in),
        (
            "update-in",
            Arity::Variadic { min: 3 },
            builtin_update_in_stub,
        ),
        ("flatten", Arity::Fixed(1), builtin_flatten),
        ("distinct", Arity::Fixed(1), builtin_distinct),
        ("frequencies", Arity::Fixed(1), builtin_frequencies),
        ("interleave", Arity::Variadic { min: 0 }, builtin_interleave),
        ("interpose", Arity::Fixed(2), builtin_interpose),
        ("partition", Arity::Variadic { min: 2 }, builtin_partition),
        ("zipmap", Arity::Fixed(2), builtin_zipmap),
        ("select-keys", Arity::Fixed(2), builtin_select_keys),
        ("find", Arity::Fixed(2), builtin_find),
        ("map-keys", Arity::Fixed(2), builtin_map_keys_stub),
        ("map-vals", Arity::Fixed(2), builtin_map_vals_stub),
        // Atoms
        ("atom", Arity::Fixed(1), builtin_atom),
        ("deref", Arity::Variadic { min: 1 }, builtin_deref),
        ("reset!", Arity::Fixed(2), builtin_reset_bang),
        // Phase 7 — Concurrency primitives
        ("compare-and-set!", Arity::Fixed(3), builtin_compare_and_set),
        ("volatile!", Arity::Fixed(1), builtin_volatile),
        ("vreset!", Arity::Fixed(2), builtin_vreset),
        ("vswap!", Arity::Variadic { min: 2 }, builtin_vswap_sentinel),
        ("volatile?", Arity::Fixed(1), builtin_volatile_q),
        ("force", Arity::Fixed(1), builtin_force),
        ("realized?", Arity::Fixed(1), builtin_realized_q),
        ("promise", Arity::Fixed(0), builtin_promise),
        ("deliver", Arity::Fixed(2), builtin_deliver),
        ("future-done?", Arity::Fixed(1), builtin_future_done_q),
        (
            "future-cancelled?",
            Arity::Fixed(1),
            builtin_future_cancelled_q,
        ),
        ("future-cancel", Arity::Fixed(1), builtin_future_cancel),
        ("agent", Arity::Fixed(1), builtin_agent),
        ("await", Arity::Variadic { min: 1 }, builtin_await),
        ("agent-error", Arity::Fixed(1), builtin_agent_error),
        ("restart-agent", Arity::Fixed(2), builtin_restart_agent),
        ("send", Arity::Variadic { min: 2 }, builtin_send_sentinel),
        (
            "send-off",
            Arity::Variadic { min: 2 },
            builtin_send_sentinel,
        ),
        ("make-delay", Arity::Fixed(1), builtin_make_delay_sentinel),
        // I/O
        ("print", Arity::Variadic { min: 0 }, builtin_print),
        ("println", Arity::Variadic { min: 0 }, builtin_println),
        ("prn", Arity::Variadic { min: 0 }, builtin_prn),
        ("pr", Arity::Variadic { min: 0 }, builtin_pr),
        ("pr-str", Arity::Variadic { min: 0 }, builtin_pr_str),
        ("str", Arity::Variadic { min: 0 }, builtin_str),
        ("read-string", Arity::Fixed(1), builtin_read_string),
        ("spit", Arity::Fixed(2), builtin_spit),
        ("slurp", Arity::Fixed(1), builtin_slurp),
        // Misc
        ("gensym", Arity::Variadic { min: 0 }, builtin_gensym),
        ("type", Arity::Fixed(1), builtin_type),
        ("hash", Arity::Fixed(1), builtin_hash),
        ("name", Arity::Fixed(1), builtin_name),
        ("namespace", Arity::Fixed(1), builtin_namespace),
        ("ex-info", Arity::Variadic { min: 2 }, builtin_ex_info),
        ("ex-data", Arity::Fixed(1), builtin_ex_data),
        ("ex-message", Arity::Fixed(1), builtin_ex_message),
        ("ex-cause", Arity::Fixed(1), builtin_ex_cause),
        ("range", Arity::Variadic { min: 0 }, builtin_range),
        ("repeat", Arity::Variadic { min: 1 }, builtin_repeat),
        ("replicate", Arity::Fixed(2), builtin_replicate),
        ("symbol", Arity::Variadic { min: 1 }, builtin_symbol),
        ("keyword", Arity::Variadic { min: 1 }, builtin_keyword_fn),
        ("boolean", Arity::Fixed(1), builtin_boolean),
        ("int", Arity::Fixed(1), builtin_int),
        ("long", Arity::Fixed(1), builtin_long),
        ("double", Arity::Fixed(1), builtin_double_fn),
        ("char", Arity::Fixed(1), builtin_char_fn),
        ("apply", Arity::Variadic { min: 2 }, builtin_apply_sentinel),
        ("swap!", Arity::Variadic { min: 2 }, builtin_swap_sentinel),
        (
            "make-lazy-seq",
            Arity::Fixed(1),
            builtin_make_lazy_seq_sentinel,
        ),
        ("format", Arity::Variadic { min: 1 }, builtin_format),
        ("re-find", Arity::Fixed(2), builtin_re_find_stub),
        ("re-seq", Arity::Fixed(2), builtin_re_seq_stub),
        ("re-matches", Arity::Fixed(2), builtin_re_matches_stub),
        ("subs", Arity::Variadic { min: 2 }, builtin_subs),
        ("split", Arity::Variadic { min: 2 }, builtin_split_stub),
        ("join", Arity::Variadic { min: 1 }, builtin_join),
        ("trim", Arity::Fixed(1), builtin_trim),
        ("upper-case", Arity::Fixed(1), builtin_upper_case),
        ("lower-case", Arity::Fixed(1), builtin_lower_case),
        ("starts-with?", Arity::Fixed(2), builtin_starts_with),
        ("ends-with?", Arity::Fixed(2), builtin_ends_with),
        ("includes?", Arity::Fixed(2), builtin_includes),
        ("clojure-version", Arity::Fixed(0), builtin_clojure_version),
        ("rand", Arity::Variadic { min: 0 }, builtin_rand),
        ("rand-int", Arity::Fixed(1), builtin_rand_int),
        ("sort", Arity::Variadic { min: 1 }, builtin_sort),
        ("sort-by", Arity::Variadic { min: 2 }, builtin_sort_by_stub),
        ("group-by", Arity::Fixed(2), builtin_group_by_stub),
        ("max-key", Arity::Variadic { min: 2 }, builtin_max_key_stub),
        ("min-key", Arity::Variadic { min: 2 }, builtin_min_key_stub),
        ("juxt", Arity::Variadic { min: 1 }, builtin_juxt_stub),
        ("fnil", Arity::Variadic { min: 2 }, builtin_fnil_stub),
        ("every?", Arity::Fixed(2), builtin_every_stub),
        ("some", Arity::Fixed(2), builtin_some_stub),
        ("not-any?", Arity::Fixed(2), builtin_not_any_stub),
        ("not-every?", Arity::Fixed(2), builtin_not_every_stub),
        ("mapv", Arity::Variadic { min: 2 }, builtin_mapv_stub),
        ("filterv", Arity::Fixed(2), builtin_filterv_stub),
        ("reduce-kv", Arity::Fixed(3), builtin_reduce_kv_stub),
        ("walk", Arity::Fixed(3), builtin_walk_stub),
        ("postwalk", Arity::Fixed(2), builtin_postwalk_stub),
        ("prewalk", Arity::Fixed(2), builtin_prewalk_stub),
        ("tree-seq", Arity::Fixed(3), builtin_tree_seq_stub),
        ("printf", Arity::Variadic { min: 1 }, builtin_printf),
        ("newline", Arity::Fixed(0), builtin_newline),
        ("flush", Arity::Fixed(0), builtin_flush),
        ("with-out-str", Arity::Variadic { min: 0 }, builtin_stub_nil),
        ("binding", Arity::Variadic { min: 0 }, builtin_stub_nil),
        ("num", Arity::Fixed(1), builtin_num),
        ("bit-and", Arity::Fixed(2), builtin_bit_and),
        ("bit-or", Arity::Fixed(2), builtin_bit_or),
        ("bit-xor", Arity::Fixed(2), builtin_bit_xor),
        ("bit-not", Arity::Fixed(1), builtin_bit_not),
        ("bit-shift-left", Arity::Fixed(2), builtin_bit_shl),
        ("bit-shift-right", Arity::Fixed(2), builtin_bit_shr),
        (
            "unsigned-bit-shift-right",
            Arity::Fixed(2),
            builtin_bit_ushr,
        ),
        ("char-code", Arity::Fixed(1), builtin_char_code),
        ("char-at", Arity::Fixed(2), builtin_char_at),
        ("string->list", Arity::Fixed(1), builtin_string_to_list),
        ("number->string", Arity::Fixed(1), builtin_number_to_string),
        (
            "string->number",
            Arity::Variadic { min: 1 },
            builtin_string_to_number,
        ),
        ("floor", Arity::Fixed(1), builtin_floor),
        ("ceil", Arity::Fixed(1), builtin_ceil),
        ("round", Arity::Fixed(1), builtin_round),
        ("sqrt", Arity::Fixed(1), builtin_sqrt),
        ("pow", Arity::Fixed(2), builtin_pow),
        ("log", Arity::Fixed(1), builtin_log),
        ("exp", Arity::Fixed(1), builtin_exp),
        ("Math/abs", Arity::Fixed(1), builtin_abs),
        ("Math/floor", Arity::Fixed(1), builtin_floor),
        ("Math/ceil", Arity::Fixed(1), builtin_ceil),
        ("Math/round", Arity::Fixed(1), builtin_round),
        ("Math/sqrt", Arity::Fixed(1), builtin_sqrt),
        ("Math/pow", Arity::Fixed(2), builtin_pow),
        ("Math/log", Arity::Fixed(1), builtin_log),
        ("Math/log10", Arity::Fixed(1), builtin_log10),
        ("Math/exp", Arity::Fixed(1), builtin_exp),
        ("Math/sin", Arity::Fixed(1), builtin_sin),
        ("Math/cos", Arity::Fixed(1), builtin_cos),
        ("Math/tan", Arity::Fixed(1), builtin_tan),
        ("Math/asin", Arity::Fixed(1), builtin_asin),
        ("Math/acos", Arity::Fixed(1), builtin_acos),
        ("Math/atan", Arity::Fixed(1), builtin_atan),
        ("Math/atan2", Arity::Fixed(2), builtin_atan2),
        ("Math/sinh", Arity::Fixed(1), builtin_sinh),
        ("Math/cosh", Arity::Fixed(1), builtin_cosh),
        ("Math/tanh", Arity::Fixed(1), builtin_tanh),
        ("Math/hypot", Arity::Fixed(2), builtin_hypot),
        ("log10", Arity::Fixed(1), builtin_log10),
        ("sin", Arity::Fixed(1), builtin_sin),
        ("cos", Arity::Fixed(1), builtin_cos),
        ("tan", Arity::Fixed(1), builtin_tan),
        ("asin", Arity::Fixed(1), builtin_asin),
        ("acos", Arity::Fixed(1), builtin_acos),
        ("atan", Arity::Fixed(1), builtin_atan),
        ("atan2", Arity::Fixed(2), builtin_atan2),
        // Protocols & Multimethods
        ("satisfies?", Arity::Fixed(2), builtin_satisfies_q),
        ("extends?", Arity::Fixed(2), builtin_extends_q),
        ("prefer-method", Arity::Fixed(3), builtin_prefer_method),
        ("remove-method", Arity::Fixed(2), builtin_remove_method),
        ("methods", Arity::Fixed(1), builtin_methods),
        ("isa?", Arity::Fixed(2), builtin_isa_q),
        // Records / reify
        (
            "make-type-instance",
            Arity::Fixed(2),
            builtin_make_type_instance,
        ),
        ("record?", Arity::Fixed(1), builtin_record_q),
        ("instance?", Arity::Fixed(2), builtin_instance_q),
    ];

    for (name, arity, func) in fns {
        let nf = NativeFn::new(name, arity, func);
        globals.intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(nf)));
    }

    // Math constants.
    globals.intern(
        ns,
        Arc::from("Math/PI"),
        Value::Double(std::f64::consts::PI),
    );
    globals.intern(ns, Arc::from("Math/E"), Value::Double(std::f64::consts::E));
}

// Bootstrap Clojure source defining higher-order functions.
pub const BOOTSTRAP_SOURCE: &str = include_str!("bootstrap.cljx");

// ── Helper: value to sequence vector ─────────────────────────────────────────

fn value_to_seq(v: &Value) -> ValueResult<Vec<Value>> {
    match v {
        Value::Nil => Ok(vec![]),
        Value::LazySeq(ls) => value_to_seq(&ls.get().realize()),
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
                    other => {
                        return Err(ValueError::WrongType {
                            expected: "seq",
                            got: other.type_name().to_string(),
                        });
                    }
                }
            }
            Ok(result)
        }
        Value::List(l) => Ok(l.get().iter().cloned().collect()),
        Value::Vector(v) => Ok(v.get().iter().cloned().collect()),
        Value::Set(s) => Ok(s.get().iter().collect()),
        Value::Map(m) => {
            let mut pairs = Vec::new();
            m.for_each(|k, v| {
                let pair = Value::Vector(GcPtr::new(PersistentVector::from_iter([
                    k.clone(),
                    v.clone(),
                ])));
                pairs.push(pair);
            });
            Ok(pairs)
        }
        Value::Str(s) => Ok(s.get().chars().map(Value::Char).collect()),
        _ => Err(ValueError::WrongType {
            expected: "seqable",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_f64(v: &Value) -> ValueResult<f64> {
    match v {
        Value::Long(n) => Ok(*n as f64),
        Value::Double(f) => Ok(*f),
        _ => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_i64(v: &Value) -> ValueResult<i64> {
    match v {
        Value::Long(n) => Ok(*n),
        Value::Double(f) => Ok(*f as i64),
        _ => Err(ValueError::WrongType {
            expected: "integer",
            got: v.type_name().to_string(),
        }),
    }
}

fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Bool(false))
}

// ── Arithmetic ────────────────────────────────────────────────────────────────

fn builtin_add(args: &[Value]) -> ValueResult<Value> {
    if args.iter().any(|v| matches!(v, Value::Double(_))) {
        let mut sum = 0.0f64;
        for v in args {
            sum += numeric_as_f64(v)?;
        }
        Ok(Value::Double(sum))
    } else {
        let mut sum = 0i64;
        for v in args {
            sum = sum.wrapping_add(numeric_as_i64(v)?);
        }
        Ok(Value::Long(sum))
    }
}

fn builtin_sub(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Err(ValueError::ArityError {
            name: "-".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    if args.len() == 1 {
        return match &args[0] {
            Value::Long(n) => Ok(Value::Long(-n)),
            Value::Double(f) => Ok(Value::Double(-f)),
            v => Err(ValueError::WrongType {
                expected: "number",
                got: v.type_name().to_string(),
            }),
        };
    }
    if args.iter().any(|v| matches!(v, Value::Double(_))) {
        let mut result = numeric_as_f64(&args[0])?;
        for v in &args[1..] {
            result -= numeric_as_f64(v)?;
        }
        Ok(Value::Double(result))
    } else {
        let mut result = numeric_as_i64(&args[0])?;
        for v in &args[1..] {
            result = result.wrapping_sub(numeric_as_i64(v)?);
        }
        Ok(Value::Long(result))
    }
}

fn builtin_mul(args: &[Value]) -> ValueResult<Value> {
    if args.iter().any(|v| matches!(v, Value::Double(_))) {
        let mut result = 1.0f64;
        for v in args {
            result *= numeric_as_f64(v)?;
        }
        Ok(Value::Double(result))
    } else {
        let mut result = 1i64;
        for v in args {
            result = result.wrapping_mul(numeric_as_i64(v)?);
        }
        Ok(Value::Long(result))
    }
}

fn builtin_div(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Err(ValueError::ArityError {
            name: "/".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    if args.len() == 1 {
        let d = numeric_as_f64(&args[0])?;
        return Ok(Value::Double(1.0 / d));
    }
    // If any arg is float, return float.
    if args.iter().any(|v| matches!(v, Value::Double(_))) {
        let mut result = numeric_as_f64(&args[0])?;
        for v in &args[1..] {
            let d = numeric_as_f64(v)?;
            if d == 0.0 {
                return Err(ValueError::Other("divide by zero".into()));
            }
            result /= d;
        }
        Ok(Value::Double(result))
    } else {
        // Integer division — check for exact division.
        let mut result = numeric_as_i64(&args[0])?;
        for v in &args[1..] {
            let d = numeric_as_i64(v)?;
            if d == 0 {
                return Err(ValueError::Other("divide by zero".into()));
            }
            if result % d != 0 {
                // Non-exact: promote to float division from the start.
                let mut fr = numeric_as_f64(&args[0])?;
                for v2 in &args[1..] {
                    fr /= numeric_as_f64(v2)?;
                }
                return Ok(Value::Double(fr));
            }
            result /= d;
        }
        Ok(Value::Long(result))
    }
}

fn builtin_mod(args: &[Value]) -> ValueResult<Value> {
    let a = numeric_as_i64(&args[0])?;
    let b = numeric_as_i64(&args[1])?;
    if b == 0 {
        return Err(ValueError::Other("mod by zero".into()));
    }
    Ok(Value::Long(((a % b) + b) % b))
}

fn builtin_rem(args: &[Value]) -> ValueResult<Value> {
    let a = numeric_as_i64(&args[0])?;
    let b = numeric_as_i64(&args[1])?;
    if b == 0 {
        return Err(ValueError::Other("rem by zero".into()));
    }
    Ok(Value::Long(a % b))
}

fn builtin_quot(args: &[Value]) -> ValueResult<Value> {
    let a = numeric_as_i64(&args[0])?;
    let b = numeric_as_i64(&args[1])?;
    if b == 0 {
        return Err(ValueError::Other("quot by zero".into()));
    }
    Ok(Value::Long(a / b))
}

fn builtin_inc(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Long(n.wrapping_add(1))),
        Value::Double(f) => Ok(Value::Double(f + 1.0)),
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_dec(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Long(n.wrapping_sub(1))),
        Value::Double(f) => Ok(Value::Double(f - 1.0)),
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_max(args: &[Value]) -> ValueResult<Value> {
    let mut result = args[0].clone();
    for v in &args[1..] {
        let gt = match (&result, v) {
            (Value::Long(a), Value::Long(b)) => b > a,
            (Value::Double(a), Value::Double(b)) => b > a,
            (Value::Long(a), Value::Double(b)) => b > &(*a as f64),
            (Value::Double(a), Value::Long(b)) => (*b as f64) > *a,
            _ => {
                return Err(ValueError::WrongType {
                    expected: "number",
                    got: v.type_name().to_string(),
                });
            }
        };
        if gt {
            result = v.clone();
        }
    }
    Ok(result)
}

fn builtin_min(args: &[Value]) -> ValueResult<Value> {
    let mut result = args[0].clone();
    for v in &args[1..] {
        let lt = match (&result, v) {
            (Value::Long(a), Value::Long(b)) => b < a,
            (Value::Double(a), Value::Double(b)) => b < a,
            (Value::Long(a), Value::Double(b)) => b < &(*a as f64),
            (Value::Double(a), Value::Long(b)) => (*b as f64) < *a,
            _ => {
                return Err(ValueError::WrongType {
                    expected: "number",
                    got: v.type_name().to_string(),
                });
            }
        };
        if lt {
            result = v.clone();
        }
    }
    Ok(result)
}

fn builtin_abs(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Long(n.abs())),
        Value::Double(f) => Ok(Value::Double(f.abs())),
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

// ── Comparison ────────────────────────────────────────────────────────────────

fn builtin_eq(args: &[Value]) -> ValueResult<Value> {
    if args.len() < 2 {
        return Ok(Value::Bool(true));
    }
    for pair in args.windows(2) {
        if pair[0] != pair[1] {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn builtin_not_eq(args: &[Value]) -> ValueResult<Value> {
    match builtin_eq(args)? {
        Value::Bool(b) => Ok(Value::Bool(!b)),
        v => Ok(v),
    }
}

fn num_compare(a: &Value, b: &Value) -> ValueResult<std::cmp::Ordering> {
    let r = match (a, b) {
        (Value::Long(x), Value::Long(y)) => x.cmp(y),
        (Value::Double(x), Value::Double(y)) => {
            x.partial_cmp(y).unwrap_or(std::cmp::Ordering::Equal)
        }
        (Value::Long(x), Value::Double(y)) => (*x as f64)
            .partial_cmp(y)
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Double(x), Value::Long(y)) => x
            .partial_cmp(&(*y as f64))
            .unwrap_or(std::cmp::Ordering::Equal),
        _ => {
            return Err(ValueError::WrongType {
                expected: "number",
                got: a.type_name().to_string(),
            });
        }
    };
    Ok(r)
}

fn builtin_lt(args: &[Value]) -> ValueResult<Value> {
    for pair in args.windows(2) {
        if num_compare(&pair[0], &pair[1])? != std::cmp::Ordering::Less {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn builtin_gt(args: &[Value]) -> ValueResult<Value> {
    for pair in args.windows(2) {
        if num_compare(&pair[0], &pair[1])? != std::cmp::Ordering::Greater {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn builtin_lte(args: &[Value]) -> ValueResult<Value> {
    for pair in args.windows(2) {
        if num_compare(&pair[0], &pair[1])? == std::cmp::Ordering::Greater {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn builtin_gte(args: &[Value]) -> ValueResult<Value> {
    for pair in args.windows(2) {
        if num_compare(&pair[0], &pair[1])? == std::cmp::Ordering::Less {
            return Ok(Value::Bool(false));
        }
    }
    Ok(Value::Bool(true))
}

fn builtin_identical(args: &[Value]) -> ValueResult<Value> {
    let same = match (&args[0], &args[1]) {
        (Value::Nil, Value::Nil) => true,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Long(a), Value::Long(b)) => a == b,
        (Value::Fn(a), Value::Fn(b)) => GcPtr::ptr_eq(a, b),
        (Value::Var(a), Value::Var(b)) => GcPtr::ptr_eq(a, b),
        (Value::Atom(a), Value::Atom(b)) => GcPtr::ptr_eq(a, b),
        _ => false,
    };
    Ok(Value::Bool(same))
}

fn builtin_compare(args: &[Value]) -> ValueResult<Value> {
    match num_compare(&args[0], &args[1]) {
        Ok(std::cmp::Ordering::Less) => Ok(Value::Long(-1)),
        Ok(std::cmp::Ordering::Equal) => Ok(Value::Long(0)),
        Ok(std::cmp::Ordering::Greater) => Ok(Value::Long(1)),
        Err(_) => {
            // Fall back to string comparison for non-numerics.
            let a = format!("{}", args[0]);
            let b = format!("{}", args[1]);
            Ok(Value::Long(a.cmp(&b) as i64))
        }
    }
}

// ── Predicates ────────────────────────────────────────────────────────────────

fn builtin_nil_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Nil)))
}
fn builtin_zero_q(args: &[Value]) -> ValueResult<Value> {
    let zero = match &args[0] {
        Value::Long(n) => *n == 0,
        Value::Double(f) => *f == 0.0,
        _ => false,
    };
    Ok(Value::Bool(zero))
}
fn builtin_pos_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Long(n) => *n > 0,
        Value::Double(f) => *f > 0.0,
        _ => false,
    }))
}
fn builtin_neg_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Long(n) => *n < 0,
        Value::Double(f) => *f < 0.0,
        _ => false,
    }))
}
fn builtin_not(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(!is_truthy(&args[0])))
}
fn builtin_true_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Bool(true))))
}
fn builtin_false_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Bool(false))))
}
fn builtin_number_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::Long(_) | Value::Double(_) | Value::BigInt(_)
    )))
}
fn builtin_integer_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::Long(_) | Value::BigInt(_)
    )))
}
fn builtin_float_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Double(_))))
}
fn builtin_string_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Str(_))))
}
fn builtin_keyword_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Keyword(_))))
}
fn builtin_symbol_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Symbol(_))))
}
fn builtin_fn_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::Fn(_) | Value::NativeFunction(_)
    )))
}
fn builtin_seq_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::List(_) | Value::Cons(_)
    )))
}
fn builtin_map_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Map(_))))
}
fn builtin_vector_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Vector(_))))
}
fn builtin_set_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Set(_))))
}
fn builtin_coll_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(args[0].is_coll()))
}
fn builtin_boolean_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Bool(_))))
}
fn builtin_char_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Char(_))))
}
fn builtin_var_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Var(_))))
}
fn builtin_atom_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Atom(_))))
}
fn builtin_empty_q(args: &[Value]) -> ValueResult<Value> {
    let empty = match &args[0] {
        Value::Nil => true,
        Value::List(l) => l.get().is_empty(),
        Value::Vector(v) => v.get().is_empty(),
        Value::Map(m) => m.count() == 0,
        Value::Set(s) => s.get().is_empty(),
        Value::Str(s) => s.get().is_empty(),
        _ => false,
    };
    Ok(Value::Bool(empty))
}
fn builtin_even_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(numeric_as_i64(&args[0])? % 2 == 0))
}
fn builtin_odd_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(numeric_as_i64(&args[0])? % 2 != 0))
}

// ── Collections ───────────────────────────────────────────────────────────────

fn builtin_list(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(
        args.iter().cloned(),
    ))))
}

fn builtin_list_star(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Err(ValueError::ArityError {
            name: "list*".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    let last = &args[args.len() - 1];
    let mut items: Vec<Value> = args[..args.len() - 1].to_vec();
    let tail = value_to_seq(last)?;
    items.extend(tail);
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
}

fn builtin_vector(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        args.iter().cloned(),
    ))))
}

fn builtin_hash_map(args: &[Value]) -> ValueResult<Value> {
    if !args.len().is_multiple_of(2) {
        return Err(ValueError::OddMap { count: args.len() });
    }
    let mut m = MapValue::empty();
    for pair in args.chunks(2) {
        m = m.assoc(pair[0].clone(), pair[1].clone());
    }
    Ok(Value::Map(m))
}

fn builtin_hash_set(args: &[Value]) -> ValueResult<Value> {
    let set = args
        .iter()
        .cloned()
        .fold(PersistentHashSet::empty(), |s, v| s.conj(v));
    Ok(Value::Set(GcPtr::new(set)))
}

fn builtin_conj(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Ok(Value::Nil);
    }
    let mut result = args[0].clone();
    for v in &args[1..] {
        result = match result {
            Value::Nil => Value::List(GcPtr::new(PersistentList::from_iter([v.clone()]))),
            Value::List(l) => {
                let tail_clone: std::sync::Arc<PersistentList> =
                    std::sync::Arc::new((*l.get()).clone());
                Value::List(GcPtr::new(PersistentList::cons(v.clone(), tail_clone)))
            }
            Value::Vector(vec) => Value::Vector(GcPtr::new(vec.get().conj(v.clone()))),
            Value::Set(s) => Value::Set(GcPtr::new(s.get().conj(v.clone()))),
            Value::Map(m) => {
                // v should be a [key val] pair.
                let pair = value_to_seq(v)?;
                if pair.len() != 2 {
                    return Err(ValueError::Other(
                        "conj on map requires [key val] pairs".into(),
                    ));
                }
                Value::Map(m.assoc(pair[0].clone(), pair[1].clone()))
            }
            _ => {
                return Err(ValueError::WrongType {
                    expected: "collection",
                    got: result.type_name().to_string(),
                });
            }
        };
    }
    Ok(result)
}

fn builtin_assoc(args: &[Value]) -> ValueResult<Value> {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Err(ValueError::Other(
            "assoc requires map followed by key-value pairs".into(),
        ));
    }
    // assoc on a TypeInstance: update field(s), return new TypeInstance.
    if let Value::TypeInstance(ti) = &args[0] {
        let mut fields = ti.get().fields.clone();
        for pair in args[1..].chunks(2) {
            fields = fields.assoc(pair[0].clone(), pair[1].clone());
        }
        return Ok(Value::TypeInstance(GcPtr::new(cljx_value::TypeInstance {
            type_tag: ti.get().type_tag.clone(),
            fields,
        })));
    }
    let mut result = match &args[0] {
        Value::Nil => MapValue::empty(),
        Value::Map(m) => m.clone(),
        Value::Vector(_) => {
            // assoc on vector: (assoc v idx val)
            let mut result = args[0].clone();
            for pair in args[1..].chunks(2) {
                let idx = numeric_as_i64(&pair[0])? as usize;
                let val = pair[1].clone();
                if let Value::Vector(v) = &result {
                    result = Value::Vector(GcPtr::new(v.get().assoc_nth(idx, val).ok_or_else(
                        || ValueError::IndexOutOfBounds {
                            idx,
                            count: v.get().count(),
                        },
                    )?));
                }
            }
            return Ok(result);
        }
        v => {
            return Err(ValueError::WrongType {
                expected: "map or vector",
                got: v.type_name().to_string(),
            });
        }
    };
    for pair in args[1..].chunks(2) {
        result = result.assoc(pair[0].clone(), pair[1].clone());
    }
    Ok(Value::Map(result))
}

fn builtin_dissoc(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Nil => Ok(Value::Nil),
        Value::Map(m) => {
            let mut result = m.clone();
            for k in &args[1..] {
                result = result.dissoc(k);
            }
            Ok(Value::Map(result))
        }
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_get(args: &[Value]) -> ValueResult<Value> {
    let default = args.get(2).cloned().unwrap_or(Value::Nil);
    match &args[0] {
        Value::Nil => Ok(default),
        Value::Map(m) => Ok(m.get(&args[1]).unwrap_or(default)),
        Value::TypeInstance(ti) => Ok(ti.get().fields.get(&args[1]).unwrap_or(default)),
        Value::Vector(v) => {
            if let Value::Long(idx) = &args[1] {
                Ok(v.get().nth(*idx as usize).cloned().unwrap_or(default))
            } else {
                Ok(default)
            }
        }
        Value::Set(s) => {
            if s.get().contains(&args[1]) {
                Ok(args[1].clone())
            } else {
                Ok(default)
            }
        }
        _ => Ok(default),
    }
}

fn builtin_get_in(args: &[Value]) -> ValueResult<Value> {
    let mut current = args[0].clone();
    let keys = value_to_seq(&args[1])?;
    let default = args.get(2).cloned().unwrap_or(Value::Nil);
    for k in keys {
        current = match current {
            Value::Map(m) => m.get(&k).unwrap_or(Value::Nil),
            Value::Vector(v) => {
                if let Value::Long(idx) = &k {
                    v.get().nth(*idx as usize).cloned().unwrap_or(Value::Nil)
                } else {
                    Value::Nil
                }
            }
            Value::Nil => {
                return Ok(default);
            }
            _ => {
                return Ok(default);
            }
        };
    }
    if current == Value::Nil {
        Ok(default)
    } else {
        Ok(current)
    }
}

fn builtin_count(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::LazySeq(_) | Value::Cons(_) => {
            // Realize and count elements (linear time).
            let items = value_to_seq(&args[0])?;
            return Ok(Value::Long(items.len() as i64));
        }
        _ => {}
    }
    let n = match &args[0] {
        Value::Nil => 0,
        Value::List(l) => l.get().count(),
        Value::Vector(v) => v.get().count(),
        Value::Map(m) => m.count(),
        Value::Set(s) => s.get().count(),
        Value::Str(s) => s.get().chars().count(),
        Value::TypeInstance(ti) => ti.get().fields.count(),
        _ => {
            return Err(ValueError::WrongType {
                expected: "collection",
                got: args[0].type_name().to_string(),
            });
        }
    };
    Ok(Value::Long(n as i64))
}

fn builtin_seq(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::LazySeq(ls) => {
            // Realize the lazy seq then apply seq to the result.
            let realized = ls.get().realize();
            builtin_seq(&[realized])
        }
        Value::Cons(_) => Ok(args[0].clone()), // cons is always non-empty
        Value::Nil => Ok(Value::Nil),
        Value::List(l) => {
            if l.get().is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(args[0].clone())
            }
        }
        Value::Vector(v) => {
            if v.get().is_empty() {
                Ok(Value::Nil)
            } else {
                let items: Vec<Value> = v.get().iter().cloned().collect();
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
            }
        }
        Value::Map(m) => {
            if m.count() == 0 {
                return Ok(Value::Nil);
            }
            let mut pairs = Vec::new();
            m.for_each(|k, v| {
                let pair = Value::Vector(GcPtr::new(PersistentVector::from_iter([
                    k.clone(),
                    v.clone(),
                ])));
                pairs.push(pair);
            });
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(pairs))))
        }
        Value::Set(s) => {
            if s.get().is_empty() {
                Ok(Value::Nil)
            } else {
                let items: Vec<Value> = s.get().iter().collect();
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
            }
        }
        Value::Str(s) => {
            if s.get().is_empty() {
                Ok(Value::Nil)
            } else {
                let chars: Vec<Value> = s.get().chars().map(Value::Char).collect();
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(chars))))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "seqable",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_first(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::LazySeq(ls) => builtin_first(&[ls.get().realize()]),
        Value::Cons(c) => Ok(c.get().head.clone()),
        Value::Nil => Ok(Value::Nil),
        Value::List(l) => Ok(l.get().first().cloned().unwrap_or(Value::Nil)),
        Value::Vector(v) => Ok(v.get().nth(0).cloned().unwrap_or(Value::Nil)),
        Value::Map(m) => {
            let mut result = None;
            m.for_each(|k, v| {
                if result.is_none() {
                    result = Some(Value::Vector(GcPtr::new(PersistentVector::from_iter([
                        k.clone(),
                        v.clone(),
                    ]))));
                }
            });
            Ok(result.unwrap_or(Value::Nil))
        }
        _ => Ok(Value::Nil),
    }
}

fn builtin_rest(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::LazySeq(ls) => builtin_rest(&[ls.get().realize()]),
        Value::Cons(c) => {
            // Return the tail directly; it may be another LazySeq, Cons, List, or Nil.
            match &c.get().tail {
                Value::Nil => Ok(Value::List(GcPtr::new(PersistentList::empty()))),
                tail => Ok(tail.clone()),
            }
        }
        Value::Nil => Ok(Value::List(GcPtr::new(PersistentList::empty()))),
        Value::List(l) => {
            // rest() returns Arc<PersistentList>; clone the pointed-to list.
            Ok(Value::List(GcPtr::new((*l.get().rest()).clone())))
        }
        Value::Vector(v) => {
            let items: Vec<Value> = v.get().iter().skip(1).cloned().collect();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
        }
        _ => Ok(Value::List(GcPtr::new(PersistentList::empty()))),
    }
}

fn builtin_next(args: &[Value]) -> ValueResult<Value> {
    // next = (seq (rest x)) — returns nil for empty, seq otherwise.
    let rest = builtin_rest(args)?;
    builtin_seq(&[rest])
}

fn builtin_cons(args: &[Value]) -> ValueResult<Value> {
    let head = args[0].clone();
    match &args[1] {
        // Lazy tails produce a CljxCons to preserve laziness.
        Value::LazySeq(_) | Value::Cons(_) => Ok(Value::Cons(GcPtr::new(CljxCons {
            head,
            tail: args[1].clone(),
        }))),
        Value::Nil => {
            let new_list = PersistentList::cons(head, std::sync::Arc::new(PersistentList::empty()));
            Ok(Value::List(GcPtr::new(new_list)))
        }
        Value::List(l) => {
            let new_list = PersistentList::cons(head, std::sync::Arc::new((*l.get()).clone()));
            Ok(Value::List(GcPtr::new(new_list)))
        }
        Value::Vector(v) => {
            let tail = PersistentList::from_iter(v.get().iter().cloned());
            let new_list = PersistentList::cons(head, std::sync::Arc::new(tail));
            Ok(Value::List(GcPtr::new(new_list)))
        }
        v => Err(ValueError::WrongType {
            expected: "seq",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_nth(args: &[Value]) -> ValueResult<Value> {
    let idx = numeric_as_i64(&args[1])? as usize;
    let default = args.get(2).cloned();
    match &args[0] {
        Value::LazySeq(_) | Value::Cons(_) => {
            let items = value_to_seq(&args[0])?;
            Ok(items.into_iter().nth(idx).or(default).unwrap_or(Value::Nil))
        }
        Value::List(l) => Ok(l
            .get()
            .iter()
            .nth(idx)
            .cloned()
            .or(default)
            .unwrap_or(Value::Nil)),
        Value::Vector(v) => Ok(v.get().nth(idx).cloned().or(default).unwrap_or(Value::Nil)),
        Value::Str(s) => Ok(s
            .get()
            .chars()
            .nth(idx)
            .map(Value::Char)
            .or(default)
            .unwrap_or(Value::Nil)),
        v => Err(ValueError::WrongType {
            expected: "sequential",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_last(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Nil => Ok(Value::Nil),
        Value::List(l) => Ok(l.get().iter().last().cloned().unwrap_or(Value::Nil)),
        Value::Vector(v) => Ok(v.get().peek().cloned().unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_reverse(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[0])?;
    let reversed: Vec<Value> = items.into_iter().rev().collect();
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(reversed))))
}

fn builtin_concat(args: &[Value]) -> ValueResult<Value> {
    let mut out = Vec::new();
    for arg in args {
        out.extend(value_to_seq(arg)?);
    }
    if out.is_empty() {
        Ok(Value::List(GcPtr::new(PersistentList::empty())))
    } else {
        Ok(Value::List(GcPtr::new(PersistentList::from_iter(out))))
    }
}

fn builtin_keys(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Nil => Ok(Value::Nil),
        Value::Map(m) => {
            let mut keys = Vec::new();
            m.for_each(|k, _| keys.push(k.clone()));
            if keys.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(keys))))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_vals(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Nil => Ok(Value::Nil),
        Value::Map(m) => {
            let mut vals = Vec::new();
            m.for_each(|_, v| vals.push(v.clone()));
            if vals.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(vals))))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_contains_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Map(m) => m.contains_key(&args[1]),
        Value::Set(s) => s.get().contains(&args[1]),
        Value::Vector(v) => {
            if let Value::Long(idx) = &args[1] {
                *idx >= 0 && (*idx as usize) < v.get().count()
            } else {
                false
            }
        }
        Value::Nil => false,
        _ => false,
    }))
}

fn builtin_merge(args: &[Value]) -> ValueResult<Value> {
    let mut result = MapValue::empty();
    let mut any = false;
    for arg in args {
        match arg {
            Value::Nil => {}
            Value::Map(m) => {
                any = true;
                m.for_each(|k, v| {
                    result = result.assoc(k.clone(), v.clone());
                });
            }
            v => {
                return Err(ValueError::WrongType {
                    expected: "map",
                    got: v.type_name().to_string(),
                });
            }
        }
    }
    if !any {
        return Ok(Value::Nil);
    }
    Ok(Value::Map(result))
}

fn builtin_into(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[1])?;
    let mut result = args[0].clone();
    for item in items {
        result = match result {
            Value::Nil => Value::List(GcPtr::new(PersistentList::from_iter([item]))),
            Value::List(l) => {
                let tail = std::sync::Arc::new((*l.get()).clone());
                Value::List(GcPtr::new(PersistentList::cons(item, tail)))
            }
            Value::Vector(v) => Value::Vector(GcPtr::new(v.get().conj(item))),
            Value::Set(s) => Value::Set(GcPtr::new(s.get().conj(item))),
            Value::Map(m) => {
                let pair = value_to_seq(&item)?;
                if pair.len() != 2 {
                    return Err(ValueError::Other("into map requires [k v] pairs".into()));
                }
                Value::Map(m.assoc(pair[0].clone(), pair[1].clone()))
            }
            other => {
                return Err(ValueError::WrongType {
                    expected: "collection",
                    got: other.type_name().to_string(),
                });
            }
        };
    }
    Ok(result)
}

fn builtin_empty(args: &[Value]) -> ValueResult<Value> {
    Ok(match &args[0] {
        Value::List(_) => Value::List(GcPtr::new(PersistentList::empty())),
        Value::Vector(_) => Value::Vector(GcPtr::new(PersistentVector::empty())),
        Value::Map(_) => Value::Map(MapValue::empty()),
        Value::Set(_) => Value::Set(GcPtr::new(PersistentHashSet::empty())),
        Value::Nil => Value::Nil,
        _ => Value::Nil,
    })
}

fn builtin_vec(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[0])?;
    Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
        items,
    ))))
}

fn builtin_set_fn(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[0])?;
    let set = items
        .into_iter()
        .fold(PersistentHashSet::empty(), |s, v| s.conj(v));
    Ok(Value::Set(GcPtr::new(set)))
}

fn builtin_disj(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Set(s) => {
            let mut result = s.get().clone();
            for k in &args[1..] {
                result = result.disj(k);
            }
            Ok(Value::Set(GcPtr::new(result)))
        }
        Value::Nil => Ok(Value::Nil),
        v => Err(ValueError::WrongType {
            expected: "set",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_peek(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::List(l) => Ok(l.get().first().cloned().unwrap_or(Value::Nil)),
        Value::Vector(v) => Ok(v.get().peek().cloned().unwrap_or(Value::Nil)),
        Value::Nil => Ok(Value::Nil),
        v => Err(ValueError::WrongType {
            expected: "stack",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_pop(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::List(l) => {
            let rest = l.get().rest();
            Ok(Value::List(GcPtr::new((*rest).clone())))
        }
        Value::Vector(v) => {
            if v.get().is_empty() {
                Err(ValueError::Other("pop on empty vector".into()))
            } else {
                Ok(Value::Vector(GcPtr::new(v.get().pop().unwrap())))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "stack",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_subvec(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Vector(v) => {
            let start = numeric_as_i64(&args[1])? as usize;
            let end = if let Some(e) = args.get(2) {
                numeric_as_i64(e)? as usize
            } else {
                v.get().count()
            };
            let items: Vec<Value> = v
                .get()
                .iter()
                .skip(start)
                .take(end - start)
                .cloned()
                .collect();
            Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
                items,
            ))))
        }
        v => Err(ValueError::WrongType {
            expected: "vector",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_assoc_in(args: &[Value]) -> ValueResult<Value> {
    let keys = value_to_seq(&args[1])?;
    let val = args[2].clone();
    assoc_in_impl(args[0].clone(), &keys, val)
}

fn assoc_in_impl(m: Value, keys: &[Value], val: Value) -> ValueResult<Value> {
    if keys.is_empty() {
        return Ok(val);
    }
    let k = &keys[0];
    let inner = match &m {
        Value::Map(map) => map.get(k).unwrap_or(Value::Nil),
        Value::Nil => Value::Nil,
        _ => Value::Nil,
    };
    let updated = assoc_in_impl(inner, &keys[1..], val)?;
    match m {
        Value::Map(map) => Ok(Value::Map(map.assoc(k.clone(), updated))),
        Value::Nil => Ok(Value::Map(MapValue::empty().assoc(k.clone(), updated))),
        _ => Ok(Value::Map(MapValue::empty().assoc(k.clone(), updated))),
    }
}

fn builtin_update_in_stub(_args: &[Value]) -> ValueResult<Value> {
    // update-in needs to call a function, stubs to nil for now.
    Ok(Value::Nil)
}

fn builtin_flatten(args: &[Value]) -> ValueResult<Value> {
    fn flatten_val(v: &Value) -> Vec<Value> {
        match v {
            Value::Nil => vec![],
            Value::List(l) => l.get().iter().flat_map(flatten_val).collect(),
            Value::Vector(v) => v.get().iter().flat_map(flatten_val).collect(),
            other => vec![other.clone()],
        }
    }
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(
        flatten_val(&args[0]),
    ))))
}

fn builtin_distinct(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[0])?;
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in items {
        use cljx_value::ClojureHash;
        let h = v.clojure_hash();
        if !seen.contains(&h) {
            seen.insert(h);
            out.push(v);
        }
    }
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(out))))
}

fn builtin_frequencies(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(&args[0])?;
    let mut m = MapValue::empty();
    for v in items {
        let count = m
            .get(&v)
            .and_then(|c| {
                if let Value::Long(n) = c {
                    Some(n)
                } else {
                    None
                }
            })
            .unwrap_or(0);
        m = m.assoc(v, Value::Long(count + 1));
    }
    Ok(Value::Map(m))
}

fn builtin_interleave(args: &[Value]) -> ValueResult<Value> {
    let seqs: Vec<Vec<Value>> = args.iter().map(value_to_seq).collect::<Result<_, _>>()?;
    let len = seqs.iter().map(|s| s.len()).min().unwrap_or(0);
    let mut out = Vec::new();
    for i in 0..len {
        for seq in &seqs {
            out.push(seq[i].clone());
        }
    }
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(out))))
}

fn builtin_interpose(args: &[Value]) -> ValueResult<Value> {
    let sep = &args[0];
    let items = value_to_seq(&args[1])?;
    let mut out = Vec::new();
    for (i, v) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(sep.clone());
        }
        out.push(v);
    }
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(out))))
}

fn builtin_partition(args: &[Value]) -> ValueResult<Value> {
    let n = numeric_as_i64(&args[0])? as usize;
    let items = value_to_seq(&args[1])?;
    let chunks: Vec<Value> = items
        .chunks(n)
        .filter(|c| c.len() == n)
        .map(|c| Value::List(GcPtr::new(PersistentList::from_iter(c.iter().cloned()))))
        .collect();
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(chunks))))
}

fn builtin_zipmap(args: &[Value]) -> ValueResult<Value> {
    let keys = value_to_seq(&args[0])?;
    let vals = value_to_seq(&args[1])?;
    let mut m = MapValue::empty();
    for (k, v) in keys.into_iter().zip(vals.into_iter()) {
        m = m.assoc(k, v);
    }
    Ok(Value::Map(m))
}

fn builtin_select_keys(args: &[Value]) -> ValueResult<Value> {
    let keys = value_to_seq(&args[1])?;
    let mut m = MapValue::empty();
    if let Value::Map(src) = &args[0] {
        for k in &keys {
            if let Some(v) = src.get(k) {
                m = m.assoc(k.clone(), v);
            }
        }
    }
    Ok(Value::Map(m))
}

fn builtin_find(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Map(m) => {
            if let Some(v) = m.get(&args[1]) {
                Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter([
                    args[1].clone(),
                    v,
                ]))))
            } else {
                Ok(Value::Nil)
            }
        }
        _ => Ok(Value::Nil),
    }
}

fn builtin_map_keys_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_map_vals_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}

// ── Atoms ─────────────────────────────────────────────────────────────────────

fn builtin_atom(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Atom(GcPtr::new(Atom::new(args[0].clone()))))
}

fn builtin_deref(args: &[Value]) -> ValueResult<Value> {
    let with_timeout = args.len() == 3;
    match &args[0] {
        Value::Atom(a) => Ok(a.get().deref()),
        Value::Var(v) => Ok(v.get().deref().unwrap_or(Value::Nil)),
        Value::Delay(d) => Ok(d.get().force()),
        Value::Agent(a) => Ok(a.get().get_state()),
        Value::Promise(p) => {
            if with_timeout {
                let timeout_ms = match &args[1] {
                    Value::Long(n) => *n as u64,
                    v => {
                        return Err(ValueError::WrongType {
                            expected: "long (timeout-ms)",
                            got: v.type_name().to_string(),
                        });
                    }
                };
                let timeout_val = args[2].clone();
                let guard = p.get().value.lock().unwrap();
                if guard.is_some() {
                    return Ok(guard.as_ref().unwrap().clone());
                }
                let (guard, _) = p
                    .get()
                    .cond
                    .wait_timeout(guard, std::time::Duration::from_millis(timeout_ms))
                    .unwrap();
                Ok(guard.as_ref().cloned().unwrap_or(timeout_val))
            } else {
                Ok(p.get().deref_blocking())
            }
        }
        Value::Future(f) => {
            if with_timeout {
                let timeout_ms = match &args[1] {
                    Value::Long(n) => *n as u64,
                    v => {
                        return Err(ValueError::WrongType {
                            expected: "long (timeout-ms)",
                            got: v.type_name().to_string(),
                        });
                    }
                };
                let timeout_val = args[2].clone();
                let guard = f.get().state.lock().unwrap();
                match &*guard {
                    FutureState::Done(v) => Ok(v.clone()),
                    FutureState::Failed(e) => Err(ValueError::Other(e.clone())),
                    FutureState::Cancelled => Err(ValueError::Other("future was cancelled".into())),
                    FutureState::Running => {
                        let (guard, _) = f
                            .get()
                            .cond
                            .wait_timeout(guard, std::time::Duration::from_millis(timeout_ms))
                            .unwrap();
                        match &*guard {
                            FutureState::Done(v) => Ok(v.clone()),
                            FutureState::Failed(e) => Err(ValueError::Other(e.clone())),
                            FutureState::Cancelled => {
                                Err(ValueError::Other("future was cancelled".into()))
                            }
                            FutureState::Running => Ok(timeout_val),
                        }
                    }
                }
            } else {
                let mut guard = f.get().state.lock().unwrap();
                loop {
                    match &*guard {
                        FutureState::Done(v) => return Ok(v.clone()),
                        FutureState::Failed(e) => return Err(ValueError::Other(e.clone())),
                        FutureState::Cancelled => {
                            return Err(ValueError::Other("future was cancelled".into()));
                        }
                        FutureState::Running => {
                            guard = f.get().cond.wait(guard).unwrap();
                        }
                    }
                }
            }
        }
        v => Err(ValueError::WrongType {
            expected: "atom, var, delay, promise, future, or agent",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_reset_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Atom(a) => Ok(a.get().reset(args[1].clone())),
        v => Err(ValueError::WrongType {
            expected: "atom",
            got: v.type_name().to_string(),
        }),
    }
}

// apply and swap! are handled specially in apply.rs; these are sentinels.
fn builtin_apply_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "apply must be invoked through the evaluator".into(),
    ))
}
fn builtin_swap_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "swap! must be invoked through the evaluator".into(),
    ))
}

// ── I/O ───────────────────────────────────────────────────────────────────────

fn print_vals(args: &[Value], sep: &str, readably: bool) -> String {
    args.iter()
        .map(|v| {
            if readably {
                format!("{}", v)
            } else {
                match v {
                    Value::Str(s) => s.get().to_string(),
                    other => format!("{}", other),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(sep)
}

fn builtin_print(args: &[Value]) -> ValueResult<Value> {
    print!("{}", print_vals(args, " ", false));
    Ok(Value::Nil)
}
fn builtin_println(args: &[Value]) -> ValueResult<Value> {
    println!("{}", print_vals(args, " ", false));
    Ok(Value::Nil)
}
fn builtin_prn(args: &[Value]) -> ValueResult<Value> {
    println!("{}", print_vals(args, " ", true));
    Ok(Value::Nil)
}
fn builtin_pr(args: &[Value]) -> ValueResult<Value> {
    print!("{}", print_vals(args, " ", true));
    Ok(Value::Nil)
}
fn builtin_pr_str(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::string(print_vals(args, " ", true)))
}
fn builtin_str(args: &[Value]) -> ValueResult<Value> {
    let s: String = args
        .iter()
        .map(|v| match v {
            Value::Nil => String::new(),
            Value::Str(s) => s.get().to_string(),
            other => {
                // Strip outer quotes for str: print without readably
                match other {
                    Value::Char(c) => c.to_string(),
                    v => format!("{}", v),
                }
            }
        })
        .collect();
    Ok(Value::string(s))
}

fn builtin_read_string(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => {
            let src = s.get().clone();
            let mut parser = cljx_reader::Parser::new(src, "<read-string>".into());
            match parser.parse_one() {
                Ok(Some(form)) => Ok(crate::eval::form_to_value(&form)),
                Ok(None) => Ok(Value::Nil),
                Err(e) => Err(ValueError::Other(e.to_string())),
            }
        }
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_spit(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    let content = match &args[1] {
        Value::Str(s) => s.get().clone(),
        v => v.to_string(),
    };
    std::fs::write(&path, &content).map_err(|e| ValueError::Other(e.to_string()))?;
    Ok(Value::Nil)
}

fn builtin_slurp(args: &[Value]) -> ValueResult<Value> {
    let path = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    let content = std::fs::read_to_string(&path).map_err(|e| ValueError::Other(e.to_string()))?;
    Ok(Value::string(content))
}

fn builtin_make_lazy_seq_sentinel(_args: &[Value]) -> ValueResult<Value> {
    // Actual work is done in apply.rs handle_make_lazy_seq.
    Err(ValueError::Other(
        "make-lazy-seq must be called from eval context".into(),
    ))
}

// ── Misc ──────────────────────────────────────────────────────────────────────

pub(crate) static GENSYM_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

fn builtin_gensym(args: &[Value]) -> ValueResult<Value> {
    let prefix = match args.first() {
        Some(Value::Str(s)) => s.get().to_string(),
        None => "G__".to_string(),
        _ => "G__".to_string(),
    };
    let n = GENSYM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Ok(Value::symbol(Symbol::simple(format!("{}{}", prefix, n))))
}

fn builtin_type(args: &[Value]) -> ValueResult<Value> {
    use crate::apply::type_tag_of;
    let tag = type_tag_of(&args[0]);
    Ok(Value::symbol(Symbol::simple(tag.as_ref())))
}

fn builtin_hash(args: &[Value]) -> ValueResult<Value> {
    use cljx_value::ClojureHash;
    Ok(Value::Long(args[0].clojure_hash() as i64))
}

fn builtin_name(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Keyword(k) => Ok(Value::string(k.get().name.as_ref().to_string())),
        Value::Symbol(s) => Ok(Value::string(s.get().name.as_ref().to_string())),
        Value::Str(s) => Ok(Value::Str(s.clone())),
        v => Err(ValueError::WrongType {
            expected: "named",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_namespace(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Keyword(k) => Ok(match &k.get().namespace {
            Some(ns) => Value::string(ns.as_ref().to_string()),
            None => Value::Nil,
        }),
        Value::Symbol(s) => Ok(match &s.get().namespace {
            Some(ns) => Value::string(ns.as_ref().to_string()),
            None => Value::Nil,
        }),
        v => Err(ValueError::WrongType {
            expected: "named",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_ex_info(args: &[Value]) -> ValueResult<Value> {
    let msg = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => format!("{}", v),
    };
    let data = args
        .get(1)
        .cloned()
        .unwrap_or(Value::Map(MapValue::empty()));
    let cause = args.get(2).cloned().unwrap_or(Value::Nil);
    let mut m = MapValue::empty();
    m = m.assoc(
        Value::keyword(Keyword::simple("message")),
        Value::string(msg),
    );
    m = m.assoc(Value::keyword(Keyword::simple("data")), data);
    if !matches!(cause, Value::Nil) {
        m = m.assoc(Value::keyword(Keyword::simple("cause")), cause);
    }
    Ok(Value::Map(m))
}

fn builtin_ex_data(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Map(m) => Ok(m
            .get(&Value::keyword(Keyword::simple("data")))
            .unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_ex_message(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Map(m) => Ok(m
            .get(&Value::keyword(Keyword::simple("message")))
            .unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_ex_cause(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Map(m) => Ok(m
            .get(&Value::keyword(Keyword::simple("cause")))
            .unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_range(args: &[Value]) -> ValueResult<Value> {
    let (start, end, step) = match args.len() {
        0 => return Err(ValueError::Other("infinite range not supported".into())),
        1 => (0i64, numeric_as_i64(&args[0])?, 1i64),
        2 => (numeric_as_i64(&args[0])?, numeric_as_i64(&args[1])?, 1i64),
        _ => (
            numeric_as_i64(&args[0])?,
            numeric_as_i64(&args[1])?,
            numeric_as_i64(&args[2])?,
        ),
    };
    if step == 0 {
        return Err(ValueError::Other("range step cannot be zero".into()));
    }
    let mut items = Vec::new();
    let mut i = start;
    while if step > 0 { i < end } else { i > end } {
        items.push(Value::Long(i));
        i = i.wrapping_add(step);
    }
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
}

fn builtin_repeat(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        1 => Err(ValueError::Other("infinite repeat not supported".into())),
        2 => {
            let n = numeric_as_i64(&args[0])? as usize;
            let v = args[1].clone();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                std::iter::repeat_n(v, n),
            ))))
        }
        _ => Err(ValueError::ArityError {
            name: "repeat".into(),
            expected: "1-2".into(),
            got: args.len(),
        }),
    }
}

fn builtin_replicate(args: &[Value]) -> ValueResult<Value> {
    let n = numeric_as_i64(&args[0])? as usize;
    let v = args[1].clone();
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(
        std::iter::repeat_n(v, n),
    ))))
}

fn builtin_symbol(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        1 => match &args[0] {
            Value::Str(s) => Ok(Value::symbol(Symbol::parse(s.get()))),
            Value::Symbol(s) => Ok(Value::Symbol(s.clone())),
            v => Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            }),
        },
        2 => {
            let ns = match &args[0] {
                Value::Str(s) => s.get().clone(),
                Value::Nil => {
                    return Ok(Value::symbol(match &args[1] {
                        Value::Str(s) => Symbol::simple(s.get().as_str()),
                        v => {
                            return Err(ValueError::WrongType {
                                expected: "string",
                                got: v.type_name().to_string(),
                            });
                        }
                    }));
                }
                v => {
                    return Err(ValueError::WrongType {
                        expected: "string",
                        got: v.type_name().to_string(),
                    });
                }
            };
            let name = match &args[1] {
                Value::Str(s) => s.get().clone(),
                v => {
                    return Err(ValueError::WrongType {
                        expected: "string",
                        got: v.type_name().to_string(),
                    });
                }
            };
            Ok(Value::symbol(Symbol::qualified(ns, name)))
        }
        n => Err(ValueError::ArityError {
            name: "symbol".into(),
            expected: "1-2".into(),
            got: n,
        }),
    }
}

fn builtin_keyword_fn(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        1 => match &args[0] {
            Value::Str(s) => Ok(Value::keyword(Keyword::parse(s.get()))),
            Value::Keyword(k) => Ok(Value::Keyword(k.clone())),
            Value::Symbol(s) => Ok(Value::keyword(Keyword::parse(&s.get().full_name()))),
            _ => Ok(Value::Nil),
        },
        2 => {
            let ns = match &args[0] {
                Value::Str(s) => s.get().clone(),
                _ => return Ok(Value::Nil),
            };
            let name = match &args[1] {
                Value::Str(s) => s.get().clone(),
                _ => return Ok(Value::Nil),
            };
            Ok(Value::keyword(Keyword::qualified(ns, name)))
        }
        n => Err(ValueError::ArityError {
            name: "keyword".into(),
            expected: "1-2".into(),
            got: n,
        }),
    }
}

fn builtin_boolean(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(is_truthy(&args[0])))
}

fn builtin_int(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_i64(&args[0])?))
}

fn builtin_long(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_i64(&args[0])?))
}

fn builtin_double_fn(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?))
}

fn builtin_char_fn(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => char::from_u32(*n as u32)
            .map(Value::Char)
            .ok_or_else(|| ValueError::Other("invalid char code".into())),
        Value::Char(c) => Ok(Value::Char(*c)),
        v => Err(ValueError::WrongType {
            expected: "integer",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_num(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(_) | Value::Double(_) => Ok(args[0].clone()),
        Value::Char(c) => Ok(Value::Long(*c as i64)),
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_format(args: &[Value]) -> ValueResult<Value> {
    // Minimal format: just use str for now.
    let fmt = match &args[0] {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "string",
                got: v.type_name().to_string(),
            });
        }
    };
    // Simple %s substitution.
    let result = fmt;
    let mut arg_idx = 1;
    let mut out = String::new();
    let mut chars = result.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('s') => {
                    if let Some(v) = args.get(arg_idx) {
                        match v {
                            Value::Str(s) => out.push_str(s.get()),
                            other => out.push_str(&format!("{}", other)),
                        }
                        arg_idx += 1;
                    }
                }
                Some('d') => {
                    if let Some(v) = args.get(arg_idx) {
                        out.push_str(&format!("{}", numeric_as_i64(v).unwrap_or(0)));
                        arg_idx += 1;
                    }
                }
                Some('%') => out.push('%'),
                Some(c2) => {
                    out.push('%');
                    out.push(c2);
                }
                None => out.push('%'),
            }
        } else {
            out.push(c);
        }
    }
    Ok(Value::string(out))
}

fn builtin_printf(args: &[Value]) -> ValueResult<Value> {
    let s = builtin_format(args)?;
    if let Value::Str(s) = s {
        print!("{}", s.get())
    }
    Ok(Value::Nil)
}

fn builtin_newline(_args: &[Value]) -> ValueResult<Value> {
    println!();
    Ok(Value::Nil)
}

fn builtin_flush(_args: &[Value]) -> ValueResult<Value> {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    Ok(Value::Nil)
}

fn builtin_stub_nil(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}

// Bit operations
fn builtin_bit_and(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])? & numeric_as_i64(&args[1])?,
    ))
}
fn builtin_bit_or(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])? | numeric_as_i64(&args[1])?,
    ))
}
fn builtin_bit_xor(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])? ^ numeric_as_i64(&args[1])?,
    ))
}
fn builtin_bit_not(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(!numeric_as_i64(&args[0])?))
}
fn builtin_bit_shl(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])? << numeric_as_i64(&args[1])?,
    ))
}
fn builtin_bit_shr(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        numeric_as_i64(&args[0])? >> numeric_as_i64(&args[1])?,
    ))
}
fn builtin_bit_ushr(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(
        ((numeric_as_i64(&args[0])? as u64) >> numeric_as_i64(&args[1])? as u64) as i64,
    ))
}

fn builtin_char_code(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Char(c) => Ok(Value::Long(*c as i64)),
        v => Err(ValueError::WrongType {
            expected: "char",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_char_at(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Str(s), Value::Long(idx)) => Ok(s
            .get()
            .chars()
            .nth(*idx as usize)
            .map(Value::Char)
            .unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_string_to_list(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => {
            let chars: Vec<Value> = s.get().chars().map(Value::Char).collect();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(chars))))
        }
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_number_to_string(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::string(n.to_string())),
        Value::Double(f) => Ok(Value::string(f.to_string())),
        v => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_string_to_number(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => {
            let radix = if let Some(Value::Long(r)) = args.get(1) {
                *r as u32
            } else {
                10
            };
            if let Ok(n) = i64::from_str_radix(s.get(), radix) {
                Ok(Value::Long(n))
            } else if radix == 10 {
                if let Ok(f) = s.get().parse::<f64>() {
                    Ok(Value::Double(f))
                } else {
                    Ok(Value::Bool(false))
                }
            } else {
                Ok(Value::Bool(false))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_floor(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.floor()))
}
fn builtin_ceil(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.ceil()))
}
fn builtin_round(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Long(numeric_as_f64(&args[0])?.round() as i64))
}
fn builtin_sqrt(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.sqrt()))
}
fn builtin_pow(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(
        numeric_as_f64(&args[0])?.powf(numeric_as_f64(&args[1])?),
    ))
}
fn builtin_log(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.ln()))
}
fn builtin_log10(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.log10()))
}
fn builtin_exp(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.exp()))
}
fn builtin_sin(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.sin()))
}
fn builtin_cos(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.cos()))
}
fn builtin_tan(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.tan()))
}
fn builtin_asin(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.asin()))
}
fn builtin_acos(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.acos()))
}
fn builtin_atan(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.atan()))
}
fn builtin_atan2(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(
        numeric_as_f64(&args[0])?.atan2(numeric_as_f64(&args[1])?),
    ))
}
fn builtin_sinh(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.sinh()))
}
fn builtin_cosh(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.cosh()))
}
fn builtin_tanh(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f64(&args[0])?.tanh()))
}
fn builtin_hypot(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(
        numeric_as_f64(&args[0])?.hypot(numeric_as_f64(&args[1])?),
    ))
}

fn builtin_rand(args: &[Value]) -> ValueResult<Value> {
    // Deterministic for testing: use a simple hash.
    let n = if args.is_empty() {
        1.0
    } else {
        numeric_as_f64(&args[0])?
    };
    Ok(Value::Double(0.5 * n)) // stub
}

fn builtin_rand_int(args: &[Value]) -> ValueResult<Value> {
    let n = numeric_as_i64(&args[0])?;
    Ok(Value::Long(n / 2)) // stub
}

fn builtin_sort(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(args.last().unwrap_or(&Value::Nil))?;
    let mut sorted = items;
    sorted.sort_by(|a, b| match (a, b) {
        (Value::Long(x), Value::Long(y)) => x.cmp(y),
        (Value::Str(x), Value::Str(y)) => x.get().cmp(y.get()),
        _ => std::cmp::Ordering::Equal,
    });
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(sorted))))
}

fn builtin_sort_by_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_group_by_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Map(MapValue::empty()))
}
fn builtin_max_key_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_min_key_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_juxt_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_fnil_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_every_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(true))
}
fn builtin_some_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_not_any_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(true))
}
fn builtin_not_every_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(false))
}
fn builtin_mapv_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Vector(GcPtr::new(PersistentVector::empty())))
}
fn builtin_filterv_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Vector(GcPtr::new(PersistentVector::empty())))
}
fn builtin_reduce_kv_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_walk_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_postwalk_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_prewalk_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_tree_seq_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}

// String functions
fn builtin_subs(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => {
            let start = numeric_as_i64(&args[1])? as usize;
            let end = if let Some(e) = args.get(2) {
                numeric_as_i64(e)? as usize
            } else {
                s.get().len()
            };
            let substr: String = s.get().chars().skip(start).take(end - start).collect();
            Ok(Value::string(substr))
        }
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_split_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Vector(GcPtr::new(PersistentVector::empty())))
}

fn builtin_join(args: &[Value]) -> ValueResult<Value> {
    let (sep, coll) = if args.len() == 1 {
        ("".to_string(), &args[0])
    } else {
        (
            match &args[0] {
                Value::Str(s) => s.get().to_string(),
                v => format!("{}", v),
            },
            &args[1],
        )
    };
    let items = value_to_seq(coll)?;
    let joined: String = items
        .iter()
        .map(|v| match v {
            Value::Str(s) => s.get().to_string(),
            other => format!("{}", other),
        })
        .collect::<Vec<_>>()
        .join(&sep);
    Ok(Value::string(joined))
}

fn builtin_trim(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => Ok(Value::string(s.get().trim().to_string())),
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_upper_case(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => Ok(Value::string(s.get().to_uppercase())),
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_lower_case(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Str(s) => Ok(Value::string(s.get().to_lowercase())),
        v => Err(ValueError::WrongType {
            expected: "string",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_starts_with(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Str(s), Value::Str(prefix)) => {
            Ok(Value::Bool(s.get().starts_with(prefix.get().as_str())))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn builtin_ends_with(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Str(s), Value::Str(suffix)) => {
            Ok(Value::Bool(s.get().ends_with(suffix.get().as_str())))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn builtin_includes(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Str(s), Value::Str(needle)) => {
            Ok(Value::Bool(s.get().contains(needle.get().as_str())))
        }
        _ => Ok(Value::Bool(false)),
    }
}

fn builtin_clojure_version(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::string("cljx-0.1.0"))
}

fn builtin_re_find_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_re_seq_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}
fn builtin_re_matches_stub(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Nil)
}

// ── Protocol & Multimethod builtins ───────────────────────────────────────────

fn builtin_satisfies_q(args: &[Value]) -> ValueResult<Value> {
    use crate::apply::type_tag_of;
    let proto = match &args[0] {
        Value::Protocol(p) => p.clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "protocol",
                got: v.type_name().to_string(),
            });
        }
    };
    let tag = type_tag_of(&args[1]);
    let impls = proto.get().impls.lock().unwrap();
    Ok(Value::Bool(impls.contains_key(tag.as_ref())))
}

fn builtin_extends_q(args: &[Value]) -> ValueResult<Value> {
    use crate::apply::resolve_type_tag;
    let proto = match &args[0] {
        Value::Protocol(p) => p.clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "protocol",
                got: v.type_name().to_string(),
            });
        }
    };
    let type_tag = match &args[1] {
        Value::Symbol(s) => resolve_type_tag(s.get().name.as_ref()),
        Value::Str(s) => resolve_type_tag(s.get().as_str()),
        Value::Keyword(k) => resolve_type_tag(k.get().name.as_ref()),
        v => {
            return Err(ValueError::WrongType {
                expected: "symbol or string",
                got: v.type_name().to_string(),
            });
        }
    };
    let impls = proto.get().impls.lock().unwrap();
    Ok(Value::Bool(impls.contains_key(type_tag.as_ref())))
}

fn builtin_prefer_method(args: &[Value]) -> ValueResult<Value> {
    let mf = match &args[0] {
        Value::MultiFn(m) => m.clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "multimethod",
                got: v.type_name().to_string(),
            });
        }
    };
    let preferred = format!("{}", args[1]);
    let over = format!("{}", args[2]);
    let mut prefers = mf.get().prefers.lock().unwrap();
    prefers.entry(preferred).or_default().push(over);
    Ok(Value::MultiFn(mf.clone()))
}

fn builtin_remove_method(args: &[Value]) -> ValueResult<Value> {
    let mf = match &args[0] {
        Value::MultiFn(m) => m.clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "multimethod",
                got: v.type_name().to_string(),
            });
        }
    };
    let key = format!("{}", args[1]);
    mf.get().methods.lock().unwrap().remove(&key);
    Ok(Value::MultiFn(mf.clone()))
}

fn builtin_methods(args: &[Value]) -> ValueResult<Value> {
    let mf = match &args[0] {
        Value::MultiFn(m) => m.clone(),
        v => {
            return Err(ValueError::WrongType {
                expected: "multimethod",
                got: v.type_name().to_string(),
            });
        }
    };
    let methods = mf.get().methods.lock().unwrap();
    let mut m = cljx_value::MapValue::empty();
    for (k, v) in methods.iter() {
        m = m.assoc(Value::string(k.clone()), v.clone());
    }
    Ok(Value::Map(m))
}

fn builtin_isa_q(args: &[Value]) -> ValueResult<Value> {
    // Stub: equality only; full hierarchy deferred.
    Ok(Value::Bool(args[0] == args[1]))
}

// ── Phase 7 — Concurrency primitives ─────────────────────────────────────────

fn builtin_compare_and_set(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Atom(a) => {
            let mut guard = a.get().value.lock().unwrap();
            if *guard == args[1] {
                *guard = args[2].clone();
                Ok(Value::Bool(true))
            } else {
                Ok(Value::Bool(false))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "atom",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_volatile(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Volatile(GcPtr::new(Volatile::new(args[0].clone()))))
}

fn builtin_vreset(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Volatile(v) => Ok(v.get().reset(args[1].clone())),
        v => Err(ValueError::WrongType {
            expected: "volatile",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_vswap_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "vswap! must be invoked through the evaluator".into(),
    ))
}

fn builtin_volatile_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Volatile(_))))
}

fn builtin_force(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Delay(d) => Ok(d.get().force()),
        other => Ok(other.clone()), // non-delay passes through
    }
}

fn builtin_realized_q(args: &[Value]) -> ValueResult<Value> {
    let realized = match &args[0] {
        Value::Delay(d) => d.get().is_realized(),
        Value::Promise(p) => p.get().is_realized(),
        Value::Future(f) => f.get().is_done(),
        Value::LazySeq(_) => false, // lazyseq is realized only after forcing
        _ => true,                  // everything else is always "realized"
    };
    Ok(Value::Bool(realized))
}

fn builtin_promise(_args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Promise(GcPtr::new(CljxPromise::new())))
}

fn builtin_deliver(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Promise(p) => {
            p.get().deliver(args[1].clone());
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "promise",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_future_done_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Future(f) => Ok(Value::Bool(f.get().is_done())),
        v => Err(ValueError::WrongType {
            expected: "future",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_future_cancelled_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Future(f) => Ok(Value::Bool(f.get().is_cancelled())),
        v => Err(ValueError::WrongType {
            expected: "future",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_future_cancel(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Future(f) => {
            let mut state = f.get().state.lock().unwrap();
            if matches!(&*state, FutureState::Running) {
                *state = FutureState::Cancelled;
                f.get().cond.notify_all();
            }
            Ok(Value::Bool(true))
        }
        v => Err(ValueError::WrongType {
            expected: "future",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_agent(args: &[Value]) -> ValueResult<Value> {
    let init = args[0].clone();
    let (tx, rx) = std::sync::mpsc::sync_channel::<AgentMsg>(1024);
    let state_arc = Arc::new(std::sync::Mutex::new(init.clone()));
    let error_arc: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));
    let worker_state = state_arc.clone();
    let worker_error = error_arc.clone();
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                AgentMsg::Update(f) => {
                    let cur = worker_state.lock().unwrap().clone();
                    match f(cur) {
                        Ok(next) => *worker_state.lock().unwrap() = next,
                        Err(e) => *worker_error.lock().unwrap() = Some(e),
                    }
                }
                AgentMsg::Shutdown => break,
            }
        }
    });
    Ok(Value::Agent(GcPtr::new(Agent {
        state: state_arc,
        error: error_arc,
        sender: std::sync::Mutex::new(tx),
    })))
}

fn builtin_await(args: &[Value]) -> ValueResult<Value> {
    for agent_val in args {
        match agent_val {
            Value::Agent(a) => {
                let (tx, rx) = std::sync::mpsc::channel::<()>();
                let sync_fn: AgentFn = Box::new(move |state| {
                    let _ = tx.send(());
                    Ok(state)
                });
                a.get()
                    .sender
                    .lock()
                    .unwrap()
                    .send(AgentMsg::Update(sync_fn))
                    .map_err(|_| ValueError::Other("await: agent is shut down".into()))?;
                rx.recv()
                    .map_err(|_| ValueError::Other("await: agent thread died".into()))?;
            }
            v => {
                return Err(ValueError::WrongType {
                    expected: "agent",
                    got: v.type_name().to_string(),
                });
            }
        }
    }
    Ok(Value::Nil)
}

fn builtin_agent_error(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Agent(a) => match a.get().get_error() {
            Some(e) => Ok(Value::string(e)),
            None => Ok(Value::Nil),
        },
        v => Err(ValueError::WrongType {
            expected: "agent",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_restart_agent(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Agent(a) => {
            a.get().clear_error();
            *a.get().state.lock().unwrap() = args[1].clone();
            Ok(args[0].clone())
        }
        v => Err(ValueError::WrongType {
            expected: "agent",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_send_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "send/send-off must be invoked through the evaluator".into(),
    ))
}

fn builtin_make_delay_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "make-delay must be invoked through the evaluator".into(),
    ))
}

// ── Records / reify ──────────────────────────────────────────────────────────

/// `(make-type-instance type-tag-str fields-map)` — low-level constructor.
/// Used by `defrecord` constructors and `reify` implementations.
fn builtin_make_type_instance(args: &[Value]) -> ValueResult<Value> {
    let type_tag = match &args[0] {
        Value::Str(s) => Arc::from(s.get().as_str()),
        Value::Symbol(s) => Arc::from(s.get().name.as_ref()),
        v => {
            return Err(ValueError::WrongType {
                expected: "string or symbol",
                got: v.type_name().to_string(),
            });
        }
    };
    let fields = match &args[1] {
        Value::Map(m) => m.clone(),
        Value::Nil => MapValue::empty(),
        v => {
            return Err(ValueError::WrongType {
                expected: "map",
                got: v.type_name().to_string(),
            });
        }
    };
    Ok(Value::TypeInstance(GcPtr::new(TypeInstance {
        type_tag,
        fields,
    })))
}

/// `(record? x)` — true if x is a TypeInstance.
fn builtin_record_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::TypeInstance(_))))
}

/// `(instance? TypeName x)` — true if x is a TypeInstance with the given type tag.
/// TypeName may be a Symbol or String.
fn builtin_instance_q(args: &[Value]) -> ValueResult<Value> {
    let expected_tag = match &args[0] {
        Value::Symbol(s) => s.get().name.clone(),
        Value::Str(s) => Arc::from(s.get().as_str()),
        _ => return Ok(Value::Bool(false)),
    };
    let result = match &args[1] {
        Value::TypeInstance(ti) => ti.get().type_tag == expected_tag,
        _ => false,
    };
    Ok(Value::Bool(result))
}
