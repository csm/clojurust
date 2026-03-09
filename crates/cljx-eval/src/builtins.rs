//! All native (Rust) built-in functions registered in `clojure.core`.

use crate::env::GlobalEnv;
use cljx_gc::GcPtr;
use cljx_value::value::SetValue;
use cljx_value::{
    Agent, AgentFn, AgentMsg, Arity, Atom, CljxCons, CljxPromise, FutureState, Keyword, MapValue,
    Namespace, NativeFn, ObjectArray, PersistentHashSet, PersistentList, PersistentVector,
    SortedSet, Symbol, TypeInstance, Value, ValueError, ValueResult, Volatile,
};
use num_bigint::{BigInt, Sign};
use num_rational::Ratio;
use num_traits::{Signed as _, ToPrimitive, Zero as _};
use std::ops::{Add, Sub};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;
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
        ("int?", Arity::Fixed(1), builtin_int_q),
        ("float?", Arity::Fixed(1), builtin_float_q),
        ("double?", Arity::Fixed(1), builtin_double_q),
        ("decimal?", Arity::Fixed(1), builtin_decimal_q),
        ("string?", Arity::Fixed(1), builtin_string_q),
        ("keyword?", Arity::Fixed(1), builtin_keyword_q),
        ("symbol?", Arity::Fixed(1), builtin_symbol_q),
        ("fn?", Arity::Fixed(1), builtin_fn_q),
        ("ifn?", Arity::Fixed(1), builtin_ifn_q),
        ("seq?", Arity::Fixed(1), builtin_seq_q),
        ("list?", Arity::Fixed(1), builtin_list_q),
        ("case=", Arity::Fixed(2), builtin_case_eq),
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
        ("ratio?", Arity::Fixed(1), builtin_ratio_q),
        ("sorted?", Arity::Fixed(1), builtin_sorted_q),
        // Collections (non-HOF)
        ("list", Arity::Variadic { min: 0 }, builtin_list),
        ("list*", Arity::Variadic { min: 1 }, builtin_list_star),
        ("vector", Arity::Variadic { min: 0 }, builtin_vector),
        ("hash-map", Arity::Variadic { min: 0 }, builtin_hash_map),
        ("array-map", Arity::Variadic { min: 0 }, builtin_array_map),
        ("hash-set", Arity::Variadic { min: 0 }, builtin_hash_set),
        ("conj", Arity::Variadic { min: 0 }, builtin_conj),
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
        ("object-array", Arity::Fixed(1), builtin_object_array),
        ("to-array", Arity::Fixed(1), builtin_to_array),
        ("to-array-2d", Arity::Fixed(1), builtin_to_array_2d),
        ("into-array", Arity::Variadic { min: 1 }, builtin_into_array),
        ("aclone", Arity::Fixed(1), builtin_aclone),
        ("alength", Arity::Fixed(1), builtin_alength),
        ("aget", Arity::Variadic { min: 2 }, builtin_aget),
        ("aset", Arity::Fixed(3), builtin_aset),
        ("amap", Arity::Fixed(1), builtin_amap_stub),
        ("areduce", Arity::Fixed(1), builtin_areduce_stub),
        (
            "aset-boolean",
            Arity::Variadic { min: 3 },
            builtin_aset_bool,
        ),
        ("aset-byte", Arity::Variadic { min: 3 }, builtin_aset_byte),
        ("aset-short", Arity::Variadic { min: 3 }, builtin_aset_short),
        ("aset-int", Arity::Variadic { min: 3 }, builtin_aset_int),
        ("aset-long", Arity::Variadic { min: 3 }, builtin_aset_long),
        (
            "aset-double",
            Arity::Variadic { min: 3 },
            builtin_aset_double,
        ),
        ("aset-float", Arity::Variadic { min: 3 }, builtin_aset_float),
        ("int-array", Arity::Variadic { min: 1 }, builtin_int_array),
        ("long-array", Arity::Variadic { min: 1 }, builtin_long_array),
        (
            "short-array",
            Arity::Variadic { min: 1 },
            builtin_short_array,
        ),
        ("byte-array", Arity::Variadic { min: 1 }, builtin_byte_array),
        (
            "float-array",
            Arity::Variadic { min: 1 },
            builtin_float_array,
        ),
        (
            "double-array",
            Arity::Variadic { min: 1 },
            builtin_double_array,
        ),
        ("char-array", Arity::Variadic { min: 1 }, builtin_char_array),
        (
            "boolean-array",
            Arity::Variadic { min: 1 },
            builtin_boolean_array,
        ),
        ("booleans", Arity::Fixed(1), builtin_identity_cast),
        ("bytes", Arity::Fixed(1), builtin_identity_cast),
        ("chars", Arity::Fixed(1), builtin_identity_cast),
        ("shorts", Arity::Fixed(1), builtin_identity_cast),
        ("ints", Arity::Fixed(1), builtin_identity_cast),
        ("longs", Arity::Fixed(1), builtin_identity_cast),
        ("floats", Arity::Fixed(1), builtin_identity_cast),
        ("doubles", Arity::Fixed(1), builtin_identity_cast),
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
        ("atom", Arity::Variadic { min: 1 }, builtin_atom),
        ("deref", Arity::Variadic { min: 1 }, builtin_deref),
        ("reset!", Arity::Fixed(2), builtin_reset_bang),
        ("get-validator", Arity::Fixed(1), builtin_get_validator),
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
        ("replicate", Arity::Fixed(2), builtin_replicate),
        ("symbol", Arity::Variadic { min: 1 }, builtin_symbol),
        ("keyword", Arity::Variadic { min: 1 }, builtin_keyword_fn),
        ("boolean", Arity::Fixed(1), builtin_boolean),
        ("int", Arity::Fixed(1), builtin_int),
        ("long", Arity::Fixed(1), builtin_long),
        ("double", Arity::Fixed(1), builtin_double_fn),
        ("float", Arity::Fixed(1), builtin_float_fn),
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
        ("sorted-set", Arity::Variadic { min: 0 }, builtin_sorted_set),
        ("sorted-map", Arity::Variadic { min: 0 }, builtin_sorted_map),
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
        // Dynamic variables (Phase 9)
        ("var-get", Arity::Fixed(1), builtin_var_get),
        ("var-set!", Arity::Fixed(2), builtin_var_set_bang),
        (
            "alter-var-root",
            Arity::Variadic { min: 2 },
            builtin_alter_var_root_sentinel,
        ),
        ("bound?", Arity::Fixed(1), builtin_bound_q),
        ("thread-bound?", Arity::Fixed(1), builtin_thread_bound_q),
        ("meta", Arity::Fixed(1), builtin_meta),
        ("with-meta", Arity::Fixed(2), builtin_with_meta),
        (
            "vary-meta",
            Arity::Variadic { min: 2 },
            builtin_vary_meta_sentinel,
        ),
        (
            "with-bindings*",
            Arity::Variadic { min: 2 },
            builtin_with_bindings_star_sentinel,
        ),
        // Namespace reflection
        ("namespace?", Arity::Fixed(1), builtin_namespace_q),
        ("ns-name", Arity::Fixed(1), builtin_ns_name),
        ("ns-interns", Arity::Fixed(1), builtin_ns_interns),
        ("ns-publics", Arity::Fixed(1), builtin_ns_interns), // alias: no private yet
        ("ns-refers", Arity::Fixed(1), builtin_ns_refers),
        ("ns-map", Arity::Fixed(1), builtin_ns_map),
        ("find-ns", Arity::Fixed(1), builtin_find_ns_sentinel),
        ("all-ns", Arity::Fixed(0), builtin_all_ns_sentinel),
        ("create-ns", Arity::Fixed(1), builtin_create_ns_sentinel),
        ("ns-aliases", Arity::Fixed(1), builtin_ns_aliases_sentinel),
        ("remove-ns", Arity::Fixed(1), builtin_remove_ns_sentinel),
        ("the-ns", Arity::Fixed(1), builtin_find_ns_sentinel), // same behaviour as find-ns for now
        (
            "alter-meta!",
            Arity::Variadic { min: 2 },
            builtin_alter_meta_bang_sentinel,
        ),
        ("ns-resolve", Arity::Fixed(2), builtin_ns_resolve_sentinel),
        ("resolve", Arity::Fixed(1), builtin_resolve_sentinel),
        // special builtins for clojurust
        ("sleep", Arity::Fixed(1), builtin_sleep),
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
pub const BOOTSTRAP_SOURCE: &str = include_str!("bootstrap.cljrs");
pub const CLOJURE_TEST_SOURCE: &str = include_str!("clojure_test.cljrs");

// ── Helper: lazy value iterator ──────────────────────────────────────────────

/// An iterator that lazily steps through any seqable `Value`, realizing
/// `LazySeq` and `Cons` cells one at a time instead of collecting into a `Vec`.
/// Finite collections (List, Vector, Set, Map, Str) are converted to a List
/// on first access, which is fine since they are already fully in memory.
struct ValueIter {
    current: Value,
}

impl ValueIter {
    fn new(v: Value) -> Self {
        ValueIter { current: v }
    }
}

impl Iterator for ValueIter {
    type Item = Value;

    fn next(&mut self) -> Option<Value> {
        loop {
            match &self.current {
                Value::Nil => return None,
                Value::WithMeta(inner, _) => {
                    self.current = inner.as_ref().clone();
                }
                Value::LazySeq(ls) => {
                    self.current = ls.get().realize();
                }
                Value::Cons(c) => {
                    let cell = c.get();
                    let head = cell.head.clone();
                    self.current = cell.tail.clone();
                    return Some(head);
                }
                Value::List(l) => {
                    return if let Some(first) = l.get().first() {
                        let head = first.clone();
                        self.current = Value::List(GcPtr::new((*l.get().rest()).clone()));
                        Some(head)
                    } else {
                        None
                    };
                }
                Value::Vector(v) => {
                    let items: Vec<Value> = v.get().iter().cloned().collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::Set(s) => {
                    let items: Vec<Value> = s.iter().cloned().collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::Map(m) => {
                    let mut pairs = Vec::new();
                    m.for_each(|k, v| {
                        pairs.push(Value::Vector(GcPtr::new(PersistentVector::from_iter([
                            k.clone(),
                            v.clone(),
                        ]))));
                    });
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(pairs)));
                }
                Value::Str(s) => {
                    let chars: Vec<Value> = s.get().chars().map(Value::Char).collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(chars)));
                }
                Value::ObjectArray(a) => {
                    let items = a.get().0.lock().unwrap().clone();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::IntArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Long(*v as i64))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::LongArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Long(*v))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::ShortArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Long(*v as i64))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::ByteArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Long(*v as i64))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::FloatArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Double(*v as f64))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::DoubleArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Double(*v))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::BooleanArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Bool(*v))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                Value::CharArray(a) => {
                    let items: Vec<Value> = a
                        .get()
                        .lock()
                        .unwrap()
                        .iter()
                        .map(|v| Value::Char(*v))
                        .collect();
                    self.current = Value::List(GcPtr::new(PersistentList::from_iter(items)));
                }
                _ => return None,
            }
        }
    }
}

// ── Helper: value to sequence vector (eager — use only when random access is needed) ──

fn value_to_seq(v: &Value) -> ValueResult<Vec<Value>> {
    Ok(ValueIter::new(v.clone()).collect())
}

fn numeric_as_f64(v: &Value) -> ValueResult<f64> {
    match v {
        Value::Long(n) => Ok(*n as f64),
        Value::Double(f) => Ok(*f),
        Value::BigInt(n) => Ok(n.get().to_f64().unwrap_or(f64::INFINITY)),
        Value::BigDecimal(d) => Ok(d.get().to_f64().unwrap_or(f64::INFINITY)),
        Value::Ratio(r) => Ok(r.get().to_f64().unwrap_or(f64::NAN)),
        _ => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_f32(v: &Value) -> ValueResult<f32> {
    let n = numeric_as_f64(v)?;
    Ok(n as f32)
}

fn numeric_as_i8(v: &Value) -> ValueResult<i8> {
    let v = numeric_as_f64(v)?;
    Ok(v as i8)
}

fn numeric_as_i16(v: &Value) -> ValueResult<i16> {
    let v = numeric_as_f64(v)?;
    Ok(v as i16)
}

fn numeric_as_i32(v: &Value) -> ValueResult<i32> {
    let v = numeric_as_f64(v)?;
    Ok(v as i32)
}

fn numeric_as_i64(v: &Value) -> ValueResult<i64> {
    match v {
        Value::Long(n) => Ok(*n),
        Value::Double(f) => {
            if f64::is_infinite(*f) || f64::is_nan(*f) {
                Err(ValueError::Other(
                    "cannot convert non-number to i64".to_string(),
                ))
            } else {
                Ok(*f as i64)
            }
        }
        Value::Char(c) => Ok(*c as i64),
        Value::BigInt(n) => n
            .get()
            .to_i64()
            .ok_or_else(|| ValueError::Other("BigInt too large for i64".into())),
        Value::Ratio(r) => r
            .get()
            .floor()
            .to_i64()
            .ok_or_else(|| ValueError::Other("cannot convert ratio".into())),
        Value::Bool(b) => Ok(*b as i64),
        _ => Err(ValueError::WrongType {
            expected: "integer",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_bigint(v: &Value) -> ValueResult<num_bigint::BigInt> {
    use num_bigint::BigInt;
    match v {
        Value::Long(n) => Ok(BigInt::from(*n)),
        Value::BigInt(n) => Ok(n.get().clone()),
        Value::Ratio(r) => Ok(r.get().to_integer()),
        _ => Err(ValueError::WrongType {
            expected: "integer",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_bigdecimal(v: &Value) -> ValueResult<bigdecimal::BigDecimal> {
    use bigdecimal::BigDecimal;
    match v {
        Value::Long(n) => Ok(BigDecimal::from(*n)),
        Value::BigInt(n) => Ok(BigDecimal::from(n.get().clone())),
        Value::BigDecimal(d) => Ok(d.get().clone()),
        Value::Double(f) => Ok(BigDecimal::try_from(*f).unwrap_or_else(|_| BigDecimal::from(0))),
        Value::Ratio(r) => {
            let numer = BigDecimal::from(r.get().numer().clone());
            let denom = BigDecimal::from(r.get().denom().clone());
            Ok(numer / denom)
        }
        _ => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn numeric_as_ratio(v: &Value) -> ValueResult<num_rational::Ratio<num_bigint::BigInt>> {
    use num_bigint::BigInt;
    use num_rational::Ratio;
    match v {
        Value::Long(n) => Ok(Ratio::from(BigInt::from(*n))),
        Value::BigInt(n) => Ok(Ratio::from(n.get().clone())),
        Value::Ratio(r) => Ok(r.get().clone()),
        _ => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn is_truthy(v: &Value) -> bool {
    !matches!(v, Value::Nil | Value::Bool(false))
}

// ── Arithmetic ────────────────────────────────────────────────────────────────

/// Determine the numeric "category" for type promotion.
/// Double > BigDecimal > Ratio > BigInt > Long.
/// Double is contagious; otherwise widen to the broadest non-Double type.
#[derive(PartialEq, Eq, PartialOrd, Ord, Clone, Copy)]
enum NumCat {
    Long,
    BigInt,
    Ratio,
    BigDecimal,
    Double,
}

fn num_category(v: &Value) -> ValueResult<NumCat> {
    match v {
        Value::Long(_) => Ok(NumCat::Long),
        Value::BigInt(_) => Ok(NumCat::BigInt),
        Value::Ratio(_) => Ok(NumCat::Ratio),
        Value::BigDecimal(_) => Ok(NumCat::BigDecimal),
        Value::Double(_) => Ok(NumCat::Double),
        _ => Err(ValueError::WrongType {
            expected: "number",
            got: v.type_name().to_string(),
        }),
    }
}

fn widest_category(args: &[Value]) -> ValueResult<NumCat> {
    let mut cat = NumCat::Long;
    for v in args {
        let c = num_category(v)?;
        if c > cat {
            cat = c;
        }
    }
    Ok(cat)
}

/// Simplify a Ratio: if denominator is 1, return Long or BigInt.
/// If `preserve_bigint` is true, integer results stay as BigInt (for when an operand was BigInt).
fn simplify_ratio_with(r: num_rational::Ratio<num_bigint::BigInt>, preserve_bigint: bool) -> Value {
    if r.is_integer() {
        let n = r.to_integer();
        if preserve_bigint {
            Value::BigInt(GcPtr::new(n))
        } else {
            match n.to_i64() {
                Some(l) => Value::Long(l),
                None => Value::BigInt(GcPtr::new(n)),
            }
        }
    } else {
        Value::Ratio(GcPtr::new(r))
    }
}

fn simplify_ratio(r: num_rational::Ratio<num_bigint::BigInt>) -> Value {
    simplify_ratio_with(r, false)
}

/// Simplify a BigInt: if it fits in i64, return Long.
/// Used only when Long arithmetic overflows — NOT when BigInt was explicitly requested.
fn simplify_bigint(n: num_bigint::BigInt) -> Value {
    match n.to_i64() {
        Some(l) => Value::Long(l),
        None => Value::BigInt(GcPtr::new(n)),
    }
}

fn builtin_add(args: &[Value]) -> ValueResult<Value> {
    let cat = widest_category(args)?;
    match cat {
        NumCat::Double => {
            let mut sum = 0.0f64;
            for v in args {
                sum += numeric_as_f64(v)?;
            }
            Ok(Value::Double(sum))
        }
        NumCat::BigDecimal => {
            let mut sum = bigdecimal::BigDecimal::from(0);
            for v in args {
                sum += numeric_as_bigdecimal(v)?;
            }
            Ok(Value::BigDecimal(GcPtr::new(sum)))
        }
        NumCat::Ratio => {
            let mut sum = num_rational::Ratio::from(num_bigint::BigInt::from(0));
            for v in args {
                sum += numeric_as_ratio(v)?;
            }
            Ok(simplify_ratio(sum))
        }
        NumCat::BigInt => {
            let mut sum = num_bigint::BigInt::from(0);
            for v in args {
                sum += numeric_as_bigint(v)?;
            }
            Ok(Value::BigInt(GcPtr::new(sum)))
        }
        NumCat::Long => {
            let mut sum: i64 = 0;
            for v in args {
                let n = numeric_as_i64(v)?;
                match sum.checked_add(n) {
                    Some(s) => sum = s,
                    None => {
                        // Overflow: promote to BigInt for remaining args
                        let mut big = num_bigint::BigInt::from(sum) + num_bigint::BigInt::from(n);
                        for v2 in &args[args.iter().position(|x| std::ptr::eq(x, v)).unwrap() + 1..]
                        {
                            big += numeric_as_bigint(v2)?;
                        }
                        return Ok(simplify_bigint(big));
                    }
                }
            }
            Ok(Value::Long(sum))
        }
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
            Value::Long(n) => match n.checked_neg() {
                Some(r) => Ok(Value::Long(r)),
                None => Ok(Value::BigInt(GcPtr::new(-num_bigint::BigInt::from(*n)))),
            },
            Value::Double(f) => Ok(Value::Double(-f)),
            Value::BigInt(n) => Ok(simplify_bigint(-n.get().clone())),
            Value::BigDecimal(d) => Ok(Value::BigDecimal(GcPtr::new(-d.get().clone()))),
            Value::Ratio(r) => Ok(simplify_ratio(-r.get().clone())),
            v => Err(ValueError::WrongType {
                expected: "number",
                got: v.type_name().to_string(),
            }),
        };
    }
    let cat = widest_category(args)?;
    match cat {
        NumCat::Double => {
            let mut result = numeric_as_f64(&args[0])?;
            for v in &args[1..] {
                result -= numeric_as_f64(v)?;
            }
            Ok(Value::Double(result))
        }
        NumCat::BigDecimal => {
            let mut result = numeric_as_bigdecimal(&args[0])?;
            for v in &args[1..] {
                result -= numeric_as_bigdecimal(v)?;
            }
            Ok(Value::BigDecimal(GcPtr::new(result)))
        }
        NumCat::Ratio => {
            let mut result = numeric_as_ratio(&args[0])?;
            for v in &args[1..] {
                result -= numeric_as_ratio(v)?;
            }
            Ok(simplify_ratio(result))
        }
        NumCat::BigInt => {
            let mut result = numeric_as_bigint(&args[0])?;
            for v in &args[1..] {
                result -= numeric_as_bigint(v)?;
            }
            Ok(Value::BigInt(GcPtr::new(result)))
        }
        NumCat::Long => {
            let mut result = numeric_as_i64(&args[0])?;
            for v in &args[1..] {
                let n = numeric_as_i64(v)?;
                match result.checked_sub(n) {
                    Some(r) => result = r,
                    None => {
                        let mut big =
                            num_bigint::BigInt::from(result) - num_bigint::BigInt::from(n);
                        for v2 in &args[args.iter().position(|x| std::ptr::eq(x, v)).unwrap() + 1..]
                        {
                            big -= numeric_as_bigint(v2)?;
                        }
                        return Ok(simplify_bigint(big));
                    }
                }
            }
            Ok(Value::Long(result))
        }
    }
}

fn builtin_mul(args: &[Value]) -> ValueResult<Value> {
    let cat = widest_category(args)?;
    match cat {
        NumCat::Double => {
            let mut result = 1.0f64;
            for v in args {
                result *= numeric_as_f64(v)?;
            }
            Ok(Value::Double(result))
        }
        NumCat::BigDecimal => {
            let mut result = bigdecimal::BigDecimal::from(1);
            for v in args {
                result *= numeric_as_bigdecimal(v)?;
            }
            Ok(Value::BigDecimal(GcPtr::new(result)))
        }
        NumCat::Ratio => {
            let mut result = num_rational::Ratio::from(num_bigint::BigInt::from(1));
            for v in args {
                result *= numeric_as_ratio(v)?;
            }
            Ok(simplify_ratio(result))
        }
        NumCat::BigInt => {
            let mut result = num_bigint::BigInt::from(1);
            for v in args {
                result *= numeric_as_bigint(v)?;
            }
            Ok(Value::BigInt(GcPtr::new(result)))
        }
        NumCat::Long => {
            let mut result: i64 = 1;
            for v in args {
                let n = numeric_as_i64(v)?;
                match result.checked_mul(n) {
                    Some(r) => result = r,
                    None => {
                        let mut big =
                            num_bigint::BigInt::from(result) * num_bigint::BigInt::from(n);
                        for v2 in &args[args.iter().position(|x| std::ptr::eq(x, v)).unwrap() + 1..]
                        {
                            big *= numeric_as_bigint(v2)?;
                        }
                        return Ok(simplify_bigint(big));
                    }
                }
            }
            Ok(Value::Long(result))
        }
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
        // (/ x) => 1/x — produce a ratio for integers
        return builtin_div(&[Value::Long(1), args[0].clone()]);
    }
    let cat = widest_category(args)?;
    match cat {
        NumCat::Double => {
            let mut result = numeric_as_f64(&args[0])?;
            for v in &args[1..] {
                result /= numeric_as_f64(v)?;
            }
            Ok(Value::Double(result))
        }
        NumCat::BigDecimal => {
            let mut result = numeric_as_bigdecimal(&args[0])?;
            for v in &args[1..] {
                let d = numeric_as_bigdecimal(v)?;
                if d.is_zero() {
                    return Err(ValueError::Other("divide by zero".into()));
                }
                result = result / d;
            }
            Ok(Value::BigDecimal(GcPtr::new(result)))
        }
        _ => {
            // For Long, BigInt, Ratio: use Ratio arithmetic to get exact results
            let preserve_bigint = cat == NumCat::BigInt;
            let mut result = numeric_as_ratio(&args[0])?;
            for v in &args[1..] {
                let d = numeric_as_ratio(v)?;
                if d.is_zero() {
                    return Err(ValueError::Other("divide by zero".into()));
                }
                result /= d;
            }
            Ok(simplify_ratio_with(result, preserve_bigint))
        }
    }
}

fn builtin_mod(args: &[Value]) -> ValueResult<Value> {
    // Clojure mod: result has same sign as divisor.
    use num_bigint::BigInt;
    match (&args[0], &args[1]) {
        // NaN in either position → throw
        (Value::Double(f), _) if f.is_nan() => Err(ValueError::Other("mod of NaN".into())),
        (_, Value::Double(f)) if f.is_nan() => Err(ValueError::Other("mod by NaN".into())),
        // Infinity as dividend → throw
        (Value::Double(f), _) if f.is_infinite() => {
            Err(ValueError::Other("mod of infinity".into()))
        }
        // Infinity as divisor → NaN
        (_, Value::Double(f)) if f.is_infinite() => Ok(Value::Double(f64::NAN)),
        // Double in either → double mod
        (_, _) if matches!(&args[0], Value::Double(_)) || matches!(&args[1], Value::Double(_)) => {
            let a = numeric_as_f64(&args[0])?;
            let b = numeric_as_f64(&args[1])?;
            if b == 0.0 {
                return Err(ValueError::Other("mod by zero".into()));
            }
            let r = a % b;
            let result = if (r > 0.0 && b < 0.0) || (r < 0.0 && b > 0.0) {
                r + b
            } else {
                r
            };
            Ok(Value::Double(result))
        }
        // BigDecimal in either → BigDecimal mod
        (Value::BigDecimal(_), _) | (_, Value::BigDecimal(_)) => {
            let a = numeric_as_bigdecimal(&args[0])?;
            let b = numeric_as_bigdecimal(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("mod by zero".into()));
            }
            let r = &a % &b;
            let result =
                if r.is_negative() && b.is_negative() || (r.is_negative() && b.is_positive()) {
                    r + &b
                } else {
                    r
                };
            Ok(Value::BigDecimal(GcPtr::new(result)))
        }
        // Ratio in either → ratio mod, result may be BigInt if integer
        (Value::Ratio(_), _) | (_, Value::Ratio(_)) => {
            let a = numeric_as_ratio(&args[0])?;
            let b = numeric_as_ratio(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("mod by zero".into()));
            }
            let r = &a % &b;
            let result = if (r > num_rational::Ratio::from(BigInt::from(0i64))
                && b < num_rational::Ratio::from(BigInt::from(0i64)))
                || (r < num_rational::Ratio::from(BigInt::from(0i64))
                    && b > num_rational::Ratio::from(BigInt::from(0i64)))
            {
                r + &b
            } else {
                r
            };
            if result.is_integer() {
                Ok(Value::BigInt(GcPtr::new(result.to_integer())))
            } else {
                Ok(Value::Ratio(GcPtr::new(result)))
            }
        }
        // BigInt in either → BigInt mod
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => {
            let a = numeric_as_bigint(&args[0])?;
            let b = numeric_as_bigint(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("mod by zero".into()));
            }
            let r = &a % &b;
            let result = if (r > BigInt::from(0i64) && b < BigInt::from(0i64))
                || (r < BigInt::from(0i64) && b > BigInt::from(0i64))
            {
                r + &b
            } else {
                r
            };
            Ok(Value::BigInt(GcPtr::new(result)))
        }
        // Long × Long → Long
        _ => {
            let a = numeric_as_i64(&args[0])?;
            let b = numeric_as_i64(&args[1])?;
            if b == 0 {
                return Err(ValueError::Other("mod by zero".into()));
            }
            Ok(Value::Long(((a % b) + b) % b))
        }
    }
}

fn builtin_rem(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Double(f), _) if f.is_nan() => Err(ValueError::Other("rem of NaN".into())),
        (_, Value::Double(f)) if f.is_nan() => Err(ValueError::Other("rem by NaN".into())),
        (Value::Double(f), _) if f.is_infinite() => {
            Err(ValueError::Other("rem of infinity".into()))
        }
        (_, Value::Double(f)) if f.is_infinite() => Ok(Value::Double(f64::NAN)),
        (_, _) if matches!(&args[0], Value::Double(_)) || matches!(&args[1], Value::Double(_)) => {
            let a = numeric_as_f64(&args[0])?;
            let b = numeric_as_f64(&args[1])?;
            if b == 0.0 {
                return Err(ValueError::Other("rem by zero".into()));
            }
            Ok(Value::Double(a % b))
        }
        (Value::BigDecimal(_), _) | (_, Value::BigDecimal(_)) => {
            let a = numeric_as_bigdecimal(&args[0])?;
            let b = numeric_as_bigdecimal(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("rem by zero".into()));
            }
            Ok(Value::BigDecimal(GcPtr::new(&a % &b)))
        }
        (Value::Ratio(_), _) | (_, Value::Ratio(_)) => {
            let a = numeric_as_ratio(&args[0])?;
            let b = numeric_as_ratio(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("rem by zero".into()));
            }
            let r = &a % &b;
            if r.is_integer() {
                Ok(Value::BigInt(GcPtr::new(r.to_integer())))
            } else {
                Ok(Value::Ratio(GcPtr::new(r)))
            }
        }
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => {
            let a = numeric_as_bigint(&args[0])?;
            let b = numeric_as_bigint(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("rem by zero".into()));
            }
            Ok(Value::BigInt(GcPtr::new(&a % &b)))
        }
        _ => {
            let a = numeric_as_i64(&args[0])?;
            let b = numeric_as_i64(&args[1])?;
            if b == 0 {
                return Err(ValueError::Other("rem by zero".into()));
            }
            Ok(Value::Long(a % b))
        }
    }
}

fn builtin_quot(args: &[Value]) -> ValueResult<Value> {
    match (&args[0], &args[1]) {
        (Value::Double(f), _) if f.is_nan() => Err(ValueError::Other("quot of NaN".into())),
        (_, Value::Double(f)) if f.is_nan() => Err(ValueError::Other("quot by NaN".into())),
        (_, _) if matches!(&args[0], Value::Double(_)) || matches!(&args[1], Value::Double(_)) => {
            let a = numeric_as_f64(&args[0])?;
            let b = numeric_as_f64(&args[1])?;
            if b == 0.0 {
                return Err(ValueError::Other("quot by zero".into()));
            }
            Ok(Value::Double((a / b).trunc()))
        }
        (Value::BigDecimal(_), _) | (_, Value::BigDecimal(_)) => {
            let a = numeric_as_bigdecimal(&args[0])?;
            let b = numeric_as_bigdecimal(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("quot by zero".into()));
            }
            let q = &a / &b;
            Ok(Value::BigDecimal(GcPtr::new(q.with_scale(0))))
        }
        (Value::Ratio(_), _) | (_, Value::Ratio(_)) => {
            let a = numeric_as_ratio(&args[0])?;
            let b = numeric_as_ratio(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("quot by zero".into()));
            }
            let q = &a / &b;
            Ok(Value::BigInt(GcPtr::new(q.to_integer())))
        }
        (Value::BigInt(_), _) | (_, Value::BigInt(_)) => {
            let a = numeric_as_bigint(&args[0])?;
            let b = numeric_as_bigint(&args[1])?;
            if b.is_zero() {
                return Err(ValueError::Other("quot by zero".into()));
            }
            Ok(Value::BigInt(GcPtr::new(&a / &b)))
        }
        _ => {
            let a = numeric_as_i64(&args[0])?;
            let b = numeric_as_i64(&args[1])?;
            if b == 0 {
                return Err(ValueError::Other("quot by zero".into()));
            }
            Ok(Value::Long(a / b))
        }
    }
}

fn builtin_inc(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Long(n.wrapping_add(1))),
        Value::Double(f) => Ok(Value::Double(f + 1.0)),
        Value::Ratio(r) => Ok(Value::Ratio(GcPtr::new(
            r.get().add(Ratio::new(BigInt::from(1), BigInt::from(1))),
        ))),
        Value::BigInt(i) => Ok(Value::BigInt(GcPtr::new(i.get().add(1)))),
        Value::BigDecimal(d) => Ok(Value::BigDecimal(GcPtr::new(d.get().add(1)))),
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
        Value::Ratio(r) => Ok(Value::Ratio(GcPtr::new(
            r.get().sub(Ratio::new(BigInt::from(1), BigInt::from(1))),
        ))),
        Value::BigInt(i) => Ok(Value::BigInt(GcPtr::new(i.get().sub(1)))),
        Value::BigDecimal(d) => Ok(Value::BigDecimal(GcPtr::new(d.get().sub(1)))),
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
        Value::Long(n) => Ok(Value::Long(n.wrapping_abs())),
        Value::Double(f) => Ok(Value::Double(f.abs())),
        Value::BigInt(b) => Ok(Value::BigInt(GcPtr::new(b.get().abs()))),
        Value::BigDecimal(d) => Ok(Value::BigDecimal(GcPtr::new(d.get().abs()))),
        Value::Ratio(r) => Ok(Value::Ratio(GcPtr::new(r.get().abs()))),
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
    let cat = widest_category(&[a.clone(), b.clone()])?;
    let r = match cat {
        NumCat::Double => {
            let x = numeric_as_f64(a)?;
            let y = numeric_as_f64(b)?;
            x.partial_cmp(&y).unwrap_or(std::cmp::Ordering::Equal)
        }
        NumCat::BigDecimal => {
            let x = numeric_as_bigdecimal(a)?;
            let y = numeric_as_bigdecimal(b)?;
            x.cmp(&y)
        }
        _ => {
            // Long, BigInt, Ratio — compare as ratios for exact precision
            let x = numeric_as_ratio(a)?;
            let y = numeric_as_ratio(b)?;
            x.cmp(&y)
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
        Value::Nil => return Err(ValueError::Other("expected number, got nil".into())),
        Value::Long(n) => *n == 0,
        Value::Double(f) => *f == 0.0,
        Value::Ratio(r) => r.get().numer().is_zero(),
        Value::BigInt(i) => i.get().is_zero(),
        Value::BigDecimal(d) => d.get().is_zero(),
        _ => {
            return Err(ValueError::WrongType {
                expected: "number",
                got: args[0].type_name().to_string(),
            });
        }
    };
    Ok(Value::Bool(zero))
}
fn builtin_pos_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Long(n) => *n > 0,
        Value::Double(f) => *f > 0.0,
        Value::Ratio(r) => r.get().numer().is_positive(),
        Value::BigInt(i) => i.get().sign() == Sign::Plus,
        Value::BigDecimal(d) => d.get().sign() == Sign::Plus,
        _ => false,
    }))
}
fn builtin_neg_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match &args[0] {
        Value::Nil => return Err(ValueError::Other("expected number, got nil".into())),
        Value::Long(n) => *n < 0,
        Value::Double(f) => *f < 0.0,
        Value::Ratio(r) => !r.get().numer().is_positive(),
        Value::BigInt(i) => i.get().sign() == Sign::Minus,
        Value::BigDecimal(d) => d.get().sign() == Sign::Minus,
        _ => {
            return Err(ValueError::WrongType {
                expected: "number",
                got: args[0].type_name().to_string(),
            });
        }
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

fn builtin_int_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Long(_))))
}

fn builtin_double_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Double(_))))
}

fn builtin_decimal_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::BigDecimal(_))))
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
fn builtin_ifn_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::Fn(_)
            | Value::NativeFunction(_)
            | Value::Macro(_)
            | Value::Keyword(_)
            | Value::Map(_)
            | Value::Set(_)
            | Value::Vector(_)
            | Value::Symbol(_)
    )))
}
fn builtin_seq_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0],
        Value::List(_) | Value::Cons(_)
    )))
}

fn builtin_list_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0].unwrap_meta(), Value::List(_))))
}

/// Type-strict equality for `case`: numbers only match if they are the same numeric type.
/// e.g. 1 (Long) != 1.0 (Double), 3.0 (Double) != 3.0M (BigDecimal).
/// Long and BigInt ARE considered equivalent (matching Clojure JVM behavior).
fn builtin_case_eq(args: &[Value]) -> ValueResult<Value> {
    let a = args[0].unwrap_meta();
    let b = args[1].unwrap_meta();
    let same_numeric_type = match (a, b) {
        // Long and BigInt are interchangeable in case (Clojure JVM behavior)
        (Value::Long(_) | Value::BigInt(_), Value::Long(_) | Value::BigInt(_)) => true,
        (Value::Double(_), Value::Double(_)) => true,
        (Value::BigDecimal(_), Value::BigDecimal(_)) => true,
        (Value::Ratio(_), Value::Ratio(_)) => true,
        // Non-numeric types: fall through to regular equality
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
        _ => {
            // Non-numeric: use regular equality
            return Ok(Value::Bool(args[0] == args[1]));
        }
    };
    Ok(Value::Bool(same_numeric_type && args[0] == args[1]))
}
fn builtin_map_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0].unwrap_meta(), Value::Map(_))))
}
fn builtin_vector_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(
        args[0].unwrap_meta(),
        Value::Vector(_)
    )))
}
fn builtin_set_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0].unwrap_meta(), Value::Set(_))))
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
    let empty = match args[0].unwrap_meta() {
        Value::Nil => true,
        Value::List(l) => l.get().is_empty(),
        Value::Vector(v) => v.get().is_empty(),
        Value::Map(m) => m.count() == 0,
        Value::Set(s) => s.is_empty(),
        Value::Str(s) => s.get().is_empty(),
        Value::LazySeq(s) => {
            let realized = s.get().realize();
            return builtin_empty_q(&[realized])
        }
        Value::Cons(c) => matches!(c.get().head, Value::Nil),
        _ => return Err(ValueError::WrongType {
            expected: "seqable",
            got: args[0].type_name().to_string()
        }),
    };
    Ok(Value::Bool(empty))
}
fn builtin_even_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Bool(n % 2 == 0)),
        Value::BigInt(n) => Ok(Value::Bool(!n.get().bit(0))),
        _ => Err(ValueError::WrongType {
            expected: "int",
            got: args[0].type_name().to_string(),
        }),
    }
}
fn builtin_odd_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => Ok(Value::Bool(n % 2 != 0)),
        Value::BigInt(n) => Ok(Value::Bool(n.get().bit(0))),
        _ => Err(ValueError::WrongType {
            expected: "int",
            got: args[0].type_name().to_string(),
        }),
    }
}

fn builtin_ratio_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Ratio(_))))
}

fn builtin_sorted_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(
        matches!(&args[0], Value::Map(MapValue::Sorted(_)))
            || matches!(&args[0], Value::Set(SetValue::Sorted(_))),
    ))
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
    items.extend(ValueIter::new(last.clone()));
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

fn builtin_array_map(args: &[Value]) -> ValueResult<Value> {
    if !args.len().is_multiple_of(2) {
        return Err(ValueError::OddMap { count: args.len() });
    }
    // Build as a regular map — starts as ArrayMap, promotes to HashMap if >8 entries.
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
    Ok(Value::Set(SetValue::Hash(GcPtr::new(set))))
}

fn builtin_conj(args: &[Value]) -> ValueResult<Value> {
    if args.is_empty() {
        return Ok(Value::Vector(GcPtr::new(PersistentVector::empty())));
    }
    let meta = args[0].get_meta().cloned();
    let mut result = args[0].unwrap_meta().clone();
    for v in &args[1..] {
        result = match result {
            Value::Nil => Value::List(GcPtr::new(PersistentList::from_iter([v.clone()]))),
            Value::List(l) => {
                let tail_clone: std::sync::Arc<PersistentList> =
                    std::sync::Arc::new((*l.get()).clone());
                Value::List(GcPtr::new(PersistentList::cons(v.clone(), tail_clone)))
            }
            Value::Vector(vec) => Value::Vector(GcPtr::new(vec.get().conj(v.clone()))),
            Value::Set(s) => Value::Set(s.conj(v.clone())),
            Value::Map(m) => {
                // v can be a [key val] pair or another map.
                match v.unwrap_meta() {
                    Value::Map(other) => {
                        let mut merged = m;
                        other.for_each(|k, val| {
                            merged = merged.assoc(k.clone(), val.clone());
                        });
                        Value::Map(merged)
                    }
                    _ => {
                        let pair = value_to_seq(v)?;
                        if pair.len() != 2 {
                            return Err(ValueError::Other(
                                "conj on map requires [key val] pairs".into(),
                            ));
                        }
                        Value::Map(m.assoc(pair[0].clone(), pair[1].clone()))
                    }
                }
            }
            // Conj onto lazy seq / cons: prepend like a list.
            Value::LazySeq(_) | Value::Cons(_) => Value::Cons(GcPtr::new(CljxCons {
                head: v.clone(),
                tail: result,
            })),
            _ => {
                return Err(ValueError::WrongType {
                    expected: "collection",
                    got: result.type_name().to_string(),
                });
            }
        };
    }
    Ok(match meta {
        Some(m) => result.with_meta(m),
        None => result,
    })
}

fn builtin_assoc(args: &[Value]) -> ValueResult<Value> {
    if args.len() < 3 || !(args.len() - 1).is_multiple_of(2) {
        return Err(ValueError::Other(
            "assoc requires map followed by key-value pairs".into(),
        ));
    }
    // Capture metadata from the input to preserve on the result.
    let meta = args[0].get_meta().cloned();
    let coll = args[0].unwrap_meta();

    let apply_meta = |v: Value| -> Value {
        match meta {
            Some(ref m) => v.with_meta(m.clone()),
            None => v,
        }
    };

    // assoc on a TypeInstance: update field(s), return new TypeInstance.
    if let Value::TypeInstance(ti) = coll {
        let mut fields = ti.get().fields.clone();
        for pair in args[1..].chunks(2) {
            fields = fields.assoc(pair[0].clone(), pair[1].clone());
        }
        return Ok(apply_meta(Value::TypeInstance(GcPtr::new(
            cljx_value::TypeInstance {
                type_tag: ti.get().type_tag.clone(),
                fields,
            },
        ))));
    }
    let mut result = match coll {
        Value::Nil => MapValue::empty(),
        Value::Map(m) => m.clone(),
        Value::Vector(_) => {
            // assoc on vector: (assoc v idx val)
            let mut result = coll.clone();
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
            return Ok(apply_meta(result));
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
    Ok(apply_meta(Value::Map(result)))
}

fn builtin_dissoc(args: &[Value]) -> ValueResult<Value> {
    let meta = args[0].get_meta().cloned();
    match args[0].unwrap_meta() {
        Value::Nil => Ok(Value::Nil),
        Value::Map(m) => {
            let mut result = m.clone();
            for k in &args[1..] {
                result = result.dissoc(k);
            }
            let v = Value::Map(result);
            Ok(match meta {
                Some(m) => v.with_meta(m),
                None => v,
            })
        }
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_get(args: &[Value]) -> ValueResult<Value> {
    let default = args.get(2).cloned().unwrap_or(Value::Nil);
    match args[0].unwrap_meta() {
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
            if s.contains(&args[1]) {
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
    let default = args.get(2).cloned().unwrap_or(Value::Nil);
    for k in ValueIter::new(args[1].clone()) {
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
    let v = args[0].unwrap_meta();
    match v {
        Value::LazySeq(_) | Value::Cons(_) => {
            // Walk and count elements lazily (linear time, no Vec alloc).
            let n = ValueIter::new(v.clone()).count();
            return Ok(Value::Long(n as i64));
        }
        _ => {}
    }
    let n = match v {
        Value::Nil => 0,
        Value::List(l) => l.get().count(),
        Value::Vector(v) => v.get().count(),
        Value::Map(m) => m.count(),
        Value::Set(s) => s.count(),
        Value::Str(s) => s.get().chars().count(),
        Value::TypeInstance(ti) => ti.get().fields.count(),
        _ => {
            return Err(ValueError::WrongType {
                expected: "collection",
                got: v.type_name().to_string(),
            });
        }
    };
    Ok(Value::Long(n as i64))
}

fn builtin_seq(args: &[Value]) -> ValueResult<Value> {
    match args[0].unwrap_meta() {
        Value::LazySeq(ls) => {
            // Realize the lazy seq then apply seq to the result.
            let realized = ls.get().realize();
            if let Some(err) = crate::apply::take_lazy_seq_error() {
                return Err(ValueError::Other(err));
            }
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
            if s.is_empty() {
                Ok(Value::Nil)
            } else {
                let items: Vec<Value> = s.iter().cloned().collect();
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
        Value::ObjectArray(a) => {
            let array = a.get().0.lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(array))))
            }
        }
        Value::BooleanArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|b| Value::Bool(*b)),
                ))))
            }
        }
        Value::ByteArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|i| Value::Long(*i as i64)),
                ))))
            }
        }
        Value::ShortArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|i| Value::Long(*i as i64)),
                ))))
            }
        }
        Value::IntArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|i| Value::Long(*i as i64)),
                ))))
            }
        }
        Value::LongArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|i| Value::Long(*i)),
                ))))
            }
        }
        Value::CharArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|i| Value::Char(*i)),
                ))))
            }
        }
        Value::FloatArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|f| Value::Double(*f as f64)),
                ))))
            }
        }
        Value::DoubleArray(a) => {
            let array = a.get().lock().unwrap().clone();
            if array.is_empty() {
                Ok(Value::Nil)
            } else {
                Ok(Value::List(GcPtr::new(PersistentList::from_iter(
                    array.iter().map(|f| Value::Double(*f)),
                ))))
            }
        }
        v => Err(ValueError::WrongType {
            expected: "seqable",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_first(args: &[Value]) -> ValueResult<Value> {
    match args[0].unwrap_meta() {
        Value::LazySeq(ls) => {
            let v = ls.get().realize();
            if let Some(err) = crate::apply::take_lazy_seq_error() {
                return Err(ValueError::Other(err));
            }
            builtin_first(&[v])
        }
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
        Value::Set(s) => Ok(s.iter().next().cloned().unwrap_or(Value::Nil)),
        Value::Str(s) => Ok(s
            .get()
            .chars()
            .next()
            .map(Value::Char)
            .unwrap_or(Value::Nil)),
        _ => Ok(Value::Nil),
    }
}

fn builtin_rest(args: &[Value]) -> ValueResult<Value> {
    match args[0].unwrap_meta() {
        Value::LazySeq(ls) => {
            let v = ls.get().realize();
            if let Some(err) = crate::apply::take_lazy_seq_error() {
                return Err(ValueError::Other(err));
            }
            builtin_rest(&[v])
        }
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
        Value::Map(m) => {
            let items: Vec<Value> = m
                .iter()
                .skip(1)
                .map(|(k, v)| {
                    Value::Vector(GcPtr::new(PersistentVector::from_iter([
                        k.clone(),
                        v.clone(),
                    ])))
                })
                .collect();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
        }
        Value::Set(s) => {
            let items: Vec<Value> = s.iter().skip(1).cloned().collect();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(items))))
        }
        Value::Str(s) => {
            let items: Vec<Value> = s.get().chars().skip(1).map(Value::Char).collect();
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
        Value::Map(m) => {
            let kvs = m
                .iter()
                .map(|e| {
                    Value::Vector(GcPtr::new(PersistentVector::from_iter([
                        e.0.clone(),
                        e.1.clone(),
                    ])))
                })
                .collect::<Vec<_>>();
            let tail = PersistentList::from_iter(kvs.iter().cloned());
            let new_list = PersistentList::cons(head, Arc::new(tail));
            Ok(Value::List(GcPtr::new(new_list)))
        }
        Value::Set(s) => {
            let tail = PersistentList::from_iter(s.iter().cloned());
            let new_list = PersistentList::cons(head, std::sync::Arc::new(tail));
            Ok(Value::List(GcPtr::new(new_list)))
        }
        Value::Str(s) => {
            let tail = PersistentList::from_iter(s.get().chars().map(Value::Char));
            let new_list = PersistentList::cons(head, Arc::new(tail));
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
        Value::LazySeq(_) | Value::Cons(_) => Ok(ValueIter::new(args[0].clone())
            .nth(idx)
            .or(default)
            .unwrap_or(Value::Nil)),
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
    match &args[0] {
        Value::Char(_) | Value::Long(_) | Value::Double(_) => Err(ValueError::WrongType {
            expected: "seq",
            got: args[0].type_name().to_string(),
        }),
        _ => {
            let items = value_to_seq(&args[0])?;
            let reversed: Vec<Value> = items.into_iter().rev().collect();
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(reversed))))
        }
    }
}

fn builtin_concat(args: &[Value]) -> ValueResult<Value> {
    let mut out = Vec::new();
    for arg in args {
        out.extend(ValueIter::new(arg.clone()));
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
        Value::List(_) => Ok(Value::Nil),
        Value::Set(_) => Ok(Value::Nil),
        Value::Vector(_) => Ok(Value::Nil),
        v => Err(ValueError::WrongType {
            expected: "map",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_contains_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(match args[0].unwrap_meta() {
        Value::Map(m) => m.contains_key(&args[1]),
        Value::Set(s) => s.contains(&args[1]),
        Value::Vector(v) => {
            if let Value::Long(idx) = &args[1] {
                *idx >= 0 && (*idx as usize) < v.get().count()
            } else if let Value::Nil = &args[1] {
                false
            } else {
                return Err(ValueError::WrongType {
                    expected: "int",
                    got: args[1].type_name().to_string(),
                });
            }
        }
        Value::Str(s) => {
            if let Value::Long(idx) = &args[1] {
                *idx >= 0 && (*idx as usize) < s.get().len()
            } else {
                return Err(ValueError::WrongType {
                    expected: "int",
                    got: args[1].type_name().to_string(),
                });
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
    let mut result = args[0].clone();
    for item in ValueIter::new(args[1].clone()) {
        result = match result {
            Value::Nil => Value::List(GcPtr::new(PersistentList::from_iter([item]))),
            Value::List(l) => {
                let tail = std::sync::Arc::new((*l.get()).clone());
                Value::List(GcPtr::new(PersistentList::cons(item, tail)))
            }
            Value::Vector(v) => Value::Vector(GcPtr::new(v.get().conj(item))),
            Value::Set(s) => Value::Set(s.conj(item)),
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
    let meta = args[0].get_meta().cloned();
    let apply_meta = |v: Value| -> Value {
        match meta {
            Some(ref m) => v.with_meta(m.clone()),
            None => v,
        }
    };
    Ok(apply_meta(match &args[0] {
        Value::List(_) => Value::List(GcPtr::new(PersistentList::empty())),
        Value::Vector(_) => Value::Vector(GcPtr::new(PersistentVector::empty())),
        Value::Map(_) => Value::Map(MapValue::empty()),
        Value::Set(_) => Value::Set(SetValue::Hash(GcPtr::new(PersistentHashSet::empty()))),
        Value::LazySeq(_) => Value::List(GcPtr::new(PersistentList::empty())),
        Value::Nil => Value::Nil,
        _ => Value::Nil,
    }))
}

fn builtin_vec(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::List(_)
        | Value::Set(_)
        | Value::Vector(_)
        | Value::Map(_)
        | Value::LazySeq(_)
        | Value::Queue(_)
        | Value::Str(_)
        | Value::ObjectArray(_)
        | Value::IntArray(_)
        | Value::LongArray(_)
        | Value::ShortArray(_)
        | Value::ByteArray(_)
        | Value::FloatArray(_)
        | Value::DoubleArray(_)
        | Value::BooleanArray(_)
        | Value::CharArray(_)
        | Value::Nil => Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(
            ValueIter::new(args[0].clone()),
        )))),

        _ => Err(ValueError::WrongType {
            expected: "seq",
            got: args[0].type_name().to_string(),
        }),
    }
}

/// `(object-array size-or-coll)` — if given a number, creates a vector of nils;
/// if given a collection, converts it to a vector.
fn builtin_object_array(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Long(n) => {
            let size = *n as usize;
            Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(
                vec![Value::Nil; size],
            ))))
        }
        Value::Double(f) => {
            let size = *f as usize;
            Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(
                vec![Value::Nil; size],
            ))))
        }
        _ => {
            let v: Vec<Value> = ValueIter::new(args[0].clone()).collect();
            Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(v))))
        }
    }
}

/// `(to-array coll)` — converts any collection to an object array.
fn builtin_to_array(args: &[Value]) -> ValueResult<Value> {
    let v: Vec<Value> = ValueIter::new(args[0].clone()).collect();
    Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(v))))
}

/// `(to-array-2d coll)` — converts a collection of collections to an array of arrays.
fn builtin_to_array_2d(args: &[Value]) -> ValueResult<Value> {
    let outer: Vec<Value> = ValueIter::new(args[0].clone())
        .map(|inner| {
            let v: Vec<Value> = ValueIter::new(inner).collect();
            Value::ObjectArray(GcPtr::new(ObjectArray::new(v)))
        })
        .collect();
    Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(outer))))
}

/// `(into-array coll)` or `(into-array type coll)` — converts to an object array (type arg ignored).
fn builtin_into_array(args: &[Value]) -> ValueResult<Value> {
    let coll = if args.len() >= 2 { &args[1] } else { &args[0] };
    let v: Vec<Value> = ValueIter::new(coll.clone()).collect();
    Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(v))))
}

/// `(aclone arr)` — clone an array.
fn builtin_aclone(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::ObjectArray(a) => {
            let cloned = a.get().0.lock().unwrap().clone();
            Ok(Value::ObjectArray(GcPtr::new(ObjectArray::new(cloned))))
        }
        Value::IntArray(a) => Ok(Value::IntArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::LongArray(a) => Ok(Value::LongArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::ShortArray(a) => Ok(Value::ShortArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::ByteArray(a) => Ok(Value::ByteArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::FloatArray(a) => Ok(Value::FloatArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::DoubleArray(a) => Ok(Value::DoubleArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::BooleanArray(a) => Ok(Value::BooleanArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        Value::CharArray(a) => Ok(Value::CharArray(GcPtr::new(Mutex::new(
            a.get().lock().unwrap().clone(),
        )))),
        _ => Err(ValueError::WrongType {
            expected: "array",
            got: args[0].type_name().to_string(),
        }),
    }
}

/// `(alength arr)` — length of an array.
fn builtin_alength(args: &[Value]) -> ValueResult<Value> {
    let len = match &args[0] {
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
            return Err(ValueError::WrongType {
                expected: "array",
                got: args[0].type_name().to_string(),
            });
        }
    };
    Ok(Value::Long(len as i64))
}

/// `(aget arr idx & idxs)` — get element from an array, supports nested access.
fn builtin_aget(args: &[Value]) -> ValueResult<Value> {
    let mut current = args[0].clone();
    for idx_val in &args[1..] {
        let idx = numeric_as_i64(idx_val)? as usize;
        current = match &current {
            Value::ObjectArray(a) => {
                let guard = a.get().0.lock().unwrap();
                guard.get(idx).cloned().unwrap_or(Value::Nil)
            }
            Value::IntArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Long(*v as i64))
                .unwrap_or(Value::Nil),
            Value::LongArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Long(*v))
                .unwrap_or(Value::Nil),
            Value::ShortArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Long(*v as i64))
                .unwrap_or(Value::Nil),
            Value::ByteArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Long(*v as i64))
                .unwrap_or(Value::Nil),
            Value::FloatArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Double(*v as f64))
                .unwrap_or(Value::Nil),
            Value::DoubleArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Double(*v))
                .unwrap_or(Value::Nil),
            Value::BooleanArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Bool(*v))
                .unwrap_or(Value::Nil),
            Value::CharArray(a) => a
                .get()
                .lock()
                .unwrap()
                .get(idx)
                .map(|v| Value::Char(*v))
                .unwrap_or(Value::Nil),
            _ => {
                return Err(ValueError::WrongType {
                    expected: "array",
                    got: current.type_name().to_string(),
                });
            }
        };
    }
    Ok(current)
}

/// `(aset arr idx val)` — set element in an array (mutates in place, returns the value set).
fn builtin_aset(args: &[Value]) -> ValueResult<Value> {
    let idx = numeric_as_i64(&args[1])? as usize;
    let newval = args[2].clone();
    match &args[0] {
        Value::ObjectArray(a) => {
            let mut guard = a.get().0.lock().unwrap();
            if idx >= guard.len() {
                return Err(ValueError::IndexOutOfBounds {
                    idx,
                    count: guard.len(),
                });
            }
            guard[idx] = newval.clone();
            Ok(newval)
        }
        Value::IntArray(_) => builtin_aset_int(args),
        Value::LongArray(_) => builtin_aset_long(args),
        Value::ShortArray(_) => builtin_aset_short(args),
        Value::ByteArray(_) => builtin_aset_byte(args),
        Value::FloatArray(_) => builtin_aset_float(args),
        Value::DoubleArray(_) => builtin_aset_double(args),
        Value::BooleanArray(_) => builtin_aset_bool(args),
        // TODO CharArray, need aset-char builtin
        _ => Err(ValueError::WrongType {
            expected: "array",
            got: args[0].type_name().to_string(),
        }),
    }
}

// amap and areduce are macros in Clojure; stubs for now.
fn builtin_amap_stub(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "amap is a macro — not yet implemented".into(),
    ))
}
fn builtin_areduce_stub(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::Other(
        "areduce is a macro — not yet implemented".into(),
    ))
}

/// Helper: build a typed array. `(xxx-array size init-or-coll)` or `(xxx-array size-or-coll)`.
/// `coerce` converts each element to the target type.
fn make_typed_array<T: Clone>(
    args: &[Value],
    default: T,
    coerce: fn(&Value) -> ValueResult<T>,
    value_builder: fn(Vec<T>) -> Value,
) -> ValueResult<Value> {
    match args.len() {
        1 => {
            // Single arg: size (numeric) or collection
            match &args[0] {
                Value::Long(n) => {
                    let size = *n as usize;
                    let vec: Vec<T> = Vec::from_iter(std::iter::repeat_n(default, size));
                    Ok(value_builder(vec))
                }
                _ => {
                    let vec: Vec<T> = ValueIter::new(args[0].clone())
                        .map(|v| coerce(&v))
                        .collect::<ValueResult<Vec<T>>>()?;
                    Ok(value_builder(vec))
                }
            }
        }
        2 => {
            // Two args: (xxx-array size init-coll)
            let size = numeric_as_i64(&args[0])? as usize;
            let items: ValueResult<Vec<T>> = ValueIter::new(args[1].clone())
                .map(|v| coerce(&v))
                .collect();
            let mut vec: Vec<T> = items?;
            vec.resize(size, default);
            Ok(value_builder(vec))
        }
        _ => Err(ValueError::ArityError {
            name: "typed-array".into(),
            expected: "1 or 2".into(),
            got: args.len(),
        }),
    }
}

fn coerce_to_char_native(v: &Value) -> ValueResult<char> {
    match v {
        Value::Char(c) => Ok(*c),
        Value::Long(n) => char::from_u32(*n as u32)
            .ok_or_else(|| ValueError::Other(format!("invalid char code point: {n}"))),
        _ => Err(ValueError::WrongType {
            expected: "char",
            got: v.type_name().to_string(),
        }),
    }
}

// aset methods
fn builtin_aset_bool(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::BooleanArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = is_truthy(&args[2]);
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Bool(newval))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "boolean-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_byte(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::ByteArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_i8(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval as i64))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "byte-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_int(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::IntArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_i32(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval as i64))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "int-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_short(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::ShortArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_i16(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval as i64))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "short-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_long(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::LongArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_i64(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "long-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_double(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::DoubleArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_f64(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval as i64))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "double-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_aset_float(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        3 => match &args[0] {
            Value::FloatArray(b) => {
                let mut v = b.get().lock().unwrap();
                let index = numeric_as_i64(&args[1])? as usize;
                let newval = numeric_as_f32(&args[2])?;
                if index >= v.len() {
                    Err(ValueError::IndexOutOfBounds {
                        idx: index,
                        count: v.len(),
                    })
                } else {
                    v[index] = newval;
                    Ok(Value::Long(newval as i64))
                }
            }
            _ => Err(ValueError::WrongType {
                expected: "float-array",
                got: args[0].type_name().to_string(),
            }),
        },
        _ => Err(ValueError::Unsupported),
    }
}

fn builtin_int_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0, numeric_as_i32, |v| {
        Value::IntArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_long_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0i64, numeric_as_i64, |v| {
        Value::LongArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_short_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0i16, numeric_as_i16, |v| {
        Value::ShortArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_byte_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0i8, numeric_as_i8, |v| {
        Value::ByteArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_float_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0f32, numeric_as_f32, |v| {
        Value::FloatArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_double_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(args, 0f64, numeric_as_f64, |v| {
        Value::DoubleArray(GcPtr::new(Mutex::new(v)))
    })
}

fn builtin_char_array(args: &[Value]) -> ValueResult<Value> {
    match args.len() {
        1 => match &args[0] {
            Value::Long(n) => {
                let size = *n as usize;
                let vec = Vec::from_iter(std::iter::repeat_n('\0', size));
                Ok(Value::CharArray(GcPtr::new(Mutex::new(vec))))
            }
            Value::Str(s) => {
                let vec = Vec::from_iter(s.get().chars());
                Ok(Value::CharArray(GcPtr::new(Mutex::new(vec))))
            }
            _ => make_typed_array(args, '\0', coerce_to_char_native, |v| {
                Value::CharArray(GcPtr::new(Mutex::new(v)))
            }),
        },
        2 => make_typed_array(args, '\0', coerce_to_char_native, |v| {
            Value::CharArray(GcPtr::new(Mutex::new(v)))
        }),
        _ => Err(ValueError::ArityError {
            name: "char-array".into(),
            expected: "1 or 2".into(),
            got: args.len(),
        }),
    }
}

fn builtin_boolean_array(args: &[Value]) -> ValueResult<Value> {
    make_typed_array(
        args,
        false,
        |v| Ok(is_truthy(v)),
        |v| Value::BooleanArray(GcPtr::new(Mutex::new(v))),
    )
}

/// `(booleans x)`, `(ints x)`, etc. — type hint casts, identity in our runtime.
fn builtin_identity_cast(args: &[Value]) -> ValueResult<Value> {
    Ok(args[0].clone())
}

fn builtin_set_fn(args: &[Value]) -> ValueResult<Value> {
    if !matches!(
        args[0],
        Value::List(_)
            | Value::Set(_)
            | Value::Map(_)
            | Value::LazySeq(_)
            | Value::Vector(_)
            | Value::Str(_)
            | Value::Nil
    ) {
        return Err(ValueError::WrongType {
            expected: "seq",
            got: args[0].type_name().to_string(),
        });
    }
    let set = ValueIter::new(args[0].clone()).fold(PersistentHashSet::empty(), |s, v| s.conj(v));
    Ok(Value::Set(SetValue::Hash(GcPtr::new(set))))
}

fn builtin_disj(args: &[Value]) -> ValueResult<Value> {
    let meta = args[0].get_meta().cloned();
    let apply_meta = |v: Value| -> Value {
        match meta {
            Some(ref m) => v.with_meta(m.clone()),
            None => v,
        }
    };
    match args[0].unwrap_meta() {
        Value::Set(s) => {
            let mut result = s.clone();
            for k in &args[1..] {
                result = result.disj(k);
            }
            Ok(apply_meta(Value::Set(result)))
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
            let len = v.get().count();
            // NaN converts to 0 via (int) cast in Clojure
            let start_i = match &args[1] {
                Value::Double(f) if f.is_nan() => 0,
                _ => numeric_as_i64(&args[1])?,
            };
            let end_i = if let Some(e) = args.get(2) {
                match e {
                    Value::Double(f) if f.is_nan() => 0,
                    _ => numeric_as_i64(e)?,
                }
            } else {
                len as i64
            };
            if start_i < 0
                || end_i < 0
                || (start_i as usize) > len
                || (end_i as usize) > len
                || start_i > end_i
            {
                return Err(ValueError::Other(format!(
                    "subvec index out of range: start={}, end={}, count={}",
                    start_i, end_i, len
                )));
            }
            let items: Vec<Value> = v
                .get()
                .iter()
                .skip(start_i as usize)
                .take((end_i - start_i) as usize)
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
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in ValueIter::new(args[0].clone()) {
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
    let mut m = MapValue::empty();
    for v in ValueIter::new(args[0].clone()) {
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
    if args.is_empty() {
        return Ok(Value::List(GcPtr::new(PersistentList::empty())));
    }
    // Step through all seqs in lockstep using first/rest, stopping when any is exhausted.
    let mut seqs: Vec<Value> = args.to_vec();
    let mut out = Vec::new();
    loop {
        // First pass: decompose all seqs into (first, rest). If any is empty, stop.
        let mut firsts = Vec::with_capacity(seqs.len());
        let mut rests = Vec::with_capacity(seqs.len());
        for seq in &seqs {
            match seq_first_rest(seq)? {
                Some((first, rest)) => {
                    firsts.push(first);
                    rests.push(rest);
                }
                None => {
                    return Ok(if out.is_empty() {
                        Value::List(GcPtr::new(PersistentList::empty()))
                    } else {
                        Value::List(GcPtr::new(PersistentList::from_iter(out)))
                    });
                }
            }
        }
        out.extend(firsts);
        seqs = rests;
    }
}

fn builtin_interpose(args: &[Value]) -> ValueResult<Value> {
    let sep = &args[0];
    let mut out = Vec::new();
    for (i, v) in ValueIter::new(args[1].clone()).enumerate() {
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
    let mut ks = args[0].clone();
    let mut vs = args[1].clone();
    let mut m = MapValue::empty();
    loop {
        let Some((k, ks_rest)) = seq_first_rest(&ks)? else {
            break;
        };
        let Some((v, vs_rest)) = seq_first_rest(&vs)? else {
            break;
        };
        m = m.assoc(k, v);
        ks = ks_rest;
        vs = vs_rest;
    }
    Ok(Value::Map(m))
}

/// Step one element from a sequence lazily. Returns `(first, rest)` or `None` if empty.
fn seq_first_rest(v: &Value) -> ValueResult<Option<(Value, Value)>> {
    match v {
        Value::Nil => Ok(None),
        Value::LazySeq(ls) => seq_first_rest(&ls.get().realize()),
        Value::Cons(c) => {
            let cell = c.get();
            Ok(Some((cell.head.clone(), cell.tail.clone())))
        }
        Value::List(l) => match l.get().first() {
            None => Ok(None),
            Some(first) => {
                let rest = l.get().rest();
                Ok(Some((
                    first.clone(),
                    Value::List(GcPtr::new((*rest).clone())),
                )))
            }
        },
        Value::Vector(vec) => {
            let mut iter = vec.get().iter();
            match iter.next() {
                None => Ok(None),
                Some(first) => {
                    let rest = PersistentVector::from_iter(iter.cloned());
                    Ok(Some((first.clone(), Value::Vector(GcPtr::new(rest)))))
                }
            }
        }
        Value::Set(s) => {
            let mut iter = s.iter();
            match iter.next() {
                None => Ok(None),
                Some(first) => {
                    let rest: Vec<Value> = iter.cloned().collect();
                    Ok(Some((
                        first.clone(),
                        Value::List(GcPtr::new(PersistentList::from_iter(rest))),
                    )))
                }
            }
        }
        Value::Map(m) => {
            let mut pairs = Vec::new();
            m.for_each(|k, v| {
                pairs.push(Value::Vector(GcPtr::new(PersistentVector::from_iter([
                    k.clone(),
                    v.clone(),
                ]))));
            });
            if pairs.is_empty() {
                Ok(None)
            } else {
                let first = pairs.remove(0);
                Ok(Some((
                    first,
                    Value::List(GcPtr::new(PersistentList::from_iter(pairs))),
                )))
            }
        }
        Value::Str(s) => {
            let mut chars = s.get().chars();
            match chars.next() {
                None => Ok(None),
                Some(ch) => {
                    let rest: Vec<Value> = chars.map(Value::Char).collect();
                    Ok(Some((
                        Value::Char(ch),
                        Value::List(GcPtr::new(PersistentList::from_iter(rest))),
                    )))
                }
            }
        }
        _ => Err(ValueError::WrongType {
            expected: "seq",
            got: v.type_name().to_string(),
        }),
    }
}

fn builtin_select_keys(args: &[Value]) -> ValueResult<Value> {
    let mut m = MapValue::empty();
    if let Value::Map(src) = &args[0] {
        for k in ValueIter::new(args[1].clone()) {
            if let Some(v) = src.get(&k) {
                m = m.assoc(k, v);
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
    // Actual option parsing (meta/validator) is handled in apply.rs handle_atom_call.
    // This fallback is only hit by direct Value-level calls (e.g. tests via apply).
    Ok(Value::Atom(GcPtr::new(Atom::new(args[0].clone()))))
}

fn builtin_get_validator(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Atom(a) => Ok(a.get().get_validator().unwrap_or(Value::Nil)),
        v => Err(ValueError::WrongType {
            expected: "atom",
            got: v.type_name().to_string(),
        }),
    }
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
    use cljx_value::value::PrintValue;
    args.iter()
        .map(|v| {
            if readably {
                format!("{}", v)
            } else {
                match v {
                    Value::Str(s) => s.get().to_string(),
                    Value::Char(c) => c.to_string(),
                    other => format!("{}", PrintValue(other)),
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
    use cljx_value::value::PrintValue;
    let s: String = args
        .iter()
        .map(|v| match v {
            Value::Nil => String::new(),
            Value::Str(s) => s.get().to_string(),
            Value::Char(c) => c.to_string(),
            other => format!("{}", PrintValue(other)),
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
            Value::Keyword(kw) => Ok(Value::symbol(Symbol::parse(&kw.get().full_name()))),
            Value::Symbol(s) => Ok(Value::Symbol(s.clone())),
            Value::Var(v) => Ok(Value::symbol(Symbol::parse(&v.get().full_name()))),
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

fn builtin_float_fn(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Double(numeric_as_f32(&args[0])? as f64))
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

fn value_compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Long(x), Value::Long(y)) => x.cmp(y),
        (Value::Str(x), Value::Str(y)) => x.get().cmp(y.get()),
        (Value::Keyword(x), Value::Keyword(y)) => x.get().full_name().cmp(&y.get().full_name()),
        _ => std::cmp::Ordering::Equal,
    }
}

fn builtin_sort(args: &[Value]) -> ValueResult<Value> {
    let items = value_to_seq(args.last().unwrap_or(&Value::Nil))?;
    let mut sorted = items;
    sorted.sort_by(value_compare);
    Ok(Value::List(GcPtr::new(PersistentList::from_iter(sorted))))
}

fn builtin_sorted_set(args: &[Value]) -> ValueResult<Value> {
    let set = SortedSet::from_iter(args.iter().cloned());
    Ok(Value::Set(SetValue::Sorted(GcPtr::new(set))))
}

fn builtin_sorted_map(args: &[Value]) -> ValueResult<Value> {
    if !args.len().is_multiple_of(2) {
        return Err(ValueError::OddMap { count: args.len() });
    }
    let sm = cljx_value::SortedMap::from_pairs(
        args.chunks(2)
            .map(|pair| (pair[0].clone(), pair[1].clone())),
    );
    Ok(Value::Map(MapValue::Sorted(GcPtr::new(sm))))
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
            let len = s.get().chars().count();
            let start_i = numeric_as_i64(&args[1])?;
            let end_i = if let Some(e) = args.get(2) {
                numeric_as_i64(e)?
            } else {
                len as i64
            };
            if start_i < 0
                || end_i < 0
                || (start_i as usize) > len
                || (end_i as usize) > len
                || start_i > end_i
            {
                return Err(ValueError::Other(format!(
                    "String index out of range: start={}, end={}, length={}",
                    start_i, end_i, len
                )));
            }
            let substr: String = s
                .get()
                .chars()
                .skip(start_i as usize)
                .take((end_i - start_i) as usize)
                .collect();
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
    let joined: String = ValueIter::new(coll.clone())
        .map(|v| match &v {
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

// ── Dynamic variables (Phase 9) ───────────────────────────────────────────────

/// `(var-get v)` — return the current value of a var (dynamic bindings respected).
fn builtin_var_get(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => Ok(crate::dynamics::deref_var(vp).unwrap_or(Value::Nil)),
        v => Err(ValueError::WrongType {
            expected: "var",
            got: v.type_name().to_string(),
        }),
    }
}

/// `(var-set! v val)` — set the thread-local binding for `v`; if none, set root.
fn builtin_var_set_bang(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => {
            let val = args[1].clone();
            if !crate::dynamics::set_thread_local(vp, val.clone()) {
                vp.get().bind(val.clone());
            }
            Ok(val)
        }
        v => Err(ValueError::WrongType {
            expected: "var",
            got: v.type_name().to_string(),
        }),
    }
}

/// Sentinel — `alter-var-root` is intercepted in `eval_call` because it needs env.
fn builtin_alter_var_root_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "alter-var-root sentinel should not be called directly".to_string(),
    })
}

/// `(bound? v)` — true if var has any binding (thread-local or root).
fn builtin_bound_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => Ok(Value::Bool(crate::dynamics::deref_var(vp).is_some())),
        _ => Ok(Value::Bool(false)),
    }
}

/// `(thread-bound? v)` — true if var has a thread-local binding on this thread.
fn builtin_thread_bound_q(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => Ok(Value::Bool(crate::dynamics::is_thread_bound(vp))),
        _ => Ok(Value::Bool(false)),
    }
}

/// `(meta x)` — return the metadata map of a var, or nil.
fn builtin_meta(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => Ok(vp.get().get_meta().unwrap_or(Value::Nil)),
        Value::Atom(a) => Ok(a.get().get_meta().unwrap_or(Value::Nil)),
        Value::WithMeta(_, meta) => Ok(meta.as_ref().clone()),
        _ => Ok(Value::Nil),
    }
}

/// `(with-meta v m)` — attach metadata to a value.
fn builtin_with_meta(args: &[Value]) -> ValueResult<Value> {
    match &args[0] {
        Value::Var(vp) => {
            vp.get().set_meta(args[1].clone());
            Ok(args[0].clone())
        }
        _ => Ok(args[0].clone().with_meta(args[1].clone())),
    }
}

/// Sentinel — `vary-meta` is intercepted in `eval_call` because it needs env.
fn builtin_vary_meta_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "vary-meta sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `with-bindings*` is intercepted in `eval_call` (needs env).
fn builtin_with_bindings_star_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "with-bindings* sentinel should not be called directly".to_string(),
    })
}

// ── Namespace reflection ──────────────────────────────────────────────────────

fn ns_from_arg(v: &Value) -> ValueResult<&cljx_gc::GcPtr<Namespace>> {
    match v {
        Value::Namespace(ns) => Ok(ns),
        other => Err(ValueError::WrongType {
            expected: "namespace",
            got: other.type_name().to_string(),
        }),
    }
}

/// `(namespace? x)` — true if x is a Namespace value.
fn builtin_namespace_q(args: &[Value]) -> ValueResult<Value> {
    Ok(Value::Bool(matches!(args[0], Value::Namespace(_))))
}

/// `(ns-name ns)` — return the name of a namespace as a Symbol.
fn builtin_ns_name(args: &[Value]) -> ValueResult<Value> {
    let ns = ns_from_arg(&args[0])?;
    let name = ns.get().name.clone();
    Ok(Value::Symbol(cljx_gc::GcPtr::new(Symbol {
        namespace: None,
        name,
    })))
}

/// `(ns-interns ns)` — map of unqualified Symbol → Var for all interned vars.
fn builtin_ns_interns(args: &[Value]) -> ValueResult<Value> {
    let ns = ns_from_arg(&args[0])?;
    let interns = ns.get().interns.lock().unwrap();
    let mut m = MapValue::empty();
    for (name, var) in interns.iter() {
        let sym = Value::Symbol(cljx_gc::GcPtr::new(Symbol {
            namespace: None,
            name: name.clone(),
        }));
        m = m.assoc(sym, Value::Var(var.clone()));
    }
    Ok(Value::Map(m))
}

/// `(ns-refers ns)` — map of Symbol → Var for all referred vars.
fn builtin_ns_refers(args: &[Value]) -> ValueResult<Value> {
    let ns = ns_from_arg(&args[0])?;
    let refers = ns.get().refers.lock().unwrap();
    let mut m = MapValue::empty();
    for (name, var) in refers.iter() {
        let sym = Value::Symbol(cljx_gc::GcPtr::new(Symbol {
            namespace: None,
            name: name.clone(),
        }));
        m = m.assoc(sym, Value::Var(var.clone()));
    }
    Ok(Value::Map(m))
}

/// `(ns-map ns)` — map of Symbol → Var for all visible names (interns + refers).
/// Interns take priority over refers on name collision.
fn builtin_ns_map(args: &[Value]) -> ValueResult<Value> {
    let ns = ns_from_arg(&args[0])?;
    let mut m = MapValue::empty();
    // refers first (lower priority)
    {
        let refers = ns.get().refers.lock().unwrap();
        for (name, var) in refers.iter() {
            let sym = Value::Symbol(cljx_gc::GcPtr::new(Symbol {
                namespace: None,
                name: name.clone(),
            }));
            m = m.assoc(sym, Value::Var(var.clone()));
        }
    }
    // interns override
    {
        let interns = ns.get().interns.lock().unwrap();
        for (name, var) in interns.iter() {
            let sym = Value::Symbol(cljx_gc::GcPtr::new(Symbol {
                namespace: None,
                name: name.clone(),
            }));
            m = m.assoc(sym, Value::Var(var.clone()));
        }
    }
    Ok(Value::Map(m))
}

/// Sentinel — `find-ns` / `the-ns` need GlobalEnv; intercepted in `eval_call`.
fn builtin_find_ns_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "find-ns sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `all-ns` needs GlobalEnv; intercepted in `eval_call`.
fn builtin_all_ns_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "all-ns sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `create-ns` needs GlobalEnv; intercepted in `eval_call`.
fn builtin_create_ns_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "create-ns sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `ns-aliases` needs GlobalEnv (to look up target ns); intercepted in `eval_call`.
fn builtin_ns_aliases_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "ns-aliases sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `remove-ns` needs GlobalEnv; intercepted in `eval_call`.
fn builtin_remove_ns_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "remove-ns sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `alter-meta!` needs apply_value; intercepted in `eval_call`.
fn builtin_alter_meta_bang_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "alter-meta! sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `ns-resolve` needs GlobalEnv; intercepted in `eval_call`.
fn builtin_ns_resolve_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "ns-resolve sentinel should not be called directly".to_string(),
    })
}

/// Sentinel — `resolve` needs GlobalEnv + current ns; intercepted in `eval_call`.
fn builtin_resolve_sentinel(_args: &[Value]) -> ValueResult<Value> {
    Err(ValueError::WrongType {
        expected: "intercepted",
        got: "resolve sentinel should not be called directly".to_string(),
    })
}

/// Sleep -- pause current thread for N ms
fn builtin_sleep(args: &[Value]) -> ValueResult<Value> {
    sleep(Duration::from_millis(
        i64::max(0, numeric_as_i64(&args[0])?) as u64,
    ));
    Ok(Value::Nil)
}
