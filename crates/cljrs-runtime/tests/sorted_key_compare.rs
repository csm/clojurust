//! Sorted collections order keys by the same policy as `compare` (issue #514).
//!
//! Sorted maps and sets used to order keys with an internal Rust `Ord` that
//! compared maps and sets by size alone and treated every pair of functions as
//! equal, so distinct keys silently overwrote each other.  They now use
//! `cljrs_value::try_compare`, the function behind `compare`: incomparable
//! keys are an error on construction and on every incremental update, and the
//! original collection is left intact.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn try_eval(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result =
            cljrs_runtime::interp::eval::eval(&form, &mut env).map_err(|e| format!("{e:?}"))?;
    }
    Ok(result)
}

fn eval(src: &str) -> Value {
    try_eval(src).unwrap_or_else(|e| panic!("{src}: {e}"))
}

fn eval_str(src: &str) -> String {
    let src = format!("(pr-str {src})");
    match eval(&src) {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected string, got {other:?}"),
    }
}

fn assert_err(src: &str) {
    if let Ok(v) = try_eval(src) {
        panic!("{src}: expected an error, got {v:?}");
    }
}

// ── The reported cases ───────────────────────────────────────────────────────

#[test]
fn distinct_same_size_maps_are_rejected_as_sorted_keys() {
    let maps = "(def a (hash-map :a 1)) (def b (hash-map :b 2))";
    assert_eq!(eval(&format!("{maps} (= a b)")), Value::Bool(false));
    assert_err(&format!("{maps} (sorted-set a b)"));
    assert_err(&format!("{maps} (sorted-map a :first b :second)"));
    assert_err("(compare {:a 1} {:b 2})");
}

#[test]
fn distinct_functions_are_rejected_as_sorted_keys() {
    assert_err("(sorted-set (fn [] 1) (fn [] 2))");
    assert_err("(sorted-map (fn [] 1) 1 (fn [] 2) 2)");
    assert_err("(sorted-set (atom 1) (atom 2))");
}

#[test]
fn same_size_sets_are_rejected_as_sorted_keys() {
    assert_err("(sorted-set #{1} #{2})");
    assert_err("(sorted-map #{1} :x #{2} :y)");
}

// ── Incremental updates follow the same policy ───────────────────────────────

#[test]
fn incremental_updates_reject_incomparable_keys() {
    assert_err("(conj (sorted-set {:a 1}) {:b 2})");
    assert_err("(conj (sorted-set 1 2) :k)");
    assert_err("(assoc (sorted-map 1 :a) \"x\" :b)");
    assert_err("(conj (sorted-map 1 :a) [\"x\" :b])");
    assert_err("(conj (sorted-map 1 :a) {\"x\" :b})");
    assert_err("(into (sorted-set 1) [2 :k])");
    assert_err("(into (sorted-map 1 :a) [[2 :b] [:k :c]])");
    assert_err("(assoc-in (sorted-map 1 {}) [:k] 2)");
    assert_err("(reduce conj (sorted-set) [(fn [] 1) (fn [] 2)])");
}

#[test]
fn failed_update_leaves_previous_collection_intact() {
    let src = r#"
        (def s (sorted-set 3 1 2))
        (def m (sorted-map 2 :b 1 :a))
        (def s-err (try (conj s :k) (catch Exception _ :caught)))
        (def m-err (try (assoc m "x" :c) (catch Exception _ :caught)))
        [(pr-str s) (pr-str m) s-err m-err (count s) (count m)]
    "#;
    let result = eval(src);
    match result {
        Value::Vector(v) => {
            let items: Vec<Value> = v.get().iter().cloned().collect();
            assert_eq!(items[0], Value::string("#{1 2 3}".to_string()));
            assert_eq!(items[1], Value::string("{1 :a, 2 :b}".to_string()));
            assert_eq!(
                items[2],
                Value::keyword(cljrs_value::Keyword::simple("caught"))
            );
            assert_eq!(
                items[3],
                Value::keyword(cljrs_value::Keyword::simple("caught"))
            );
            assert_eq!(items[4], Value::Long(3));
            assert_eq!(items[5], Value::Long(2));
        }
        other => panic!("expected vector, got {other:?}"),
    }
}

// ── Supported keys keep working, and agree with `compare` ────────────────────

#[test]
fn supported_keys_sort_like_compare() {
    assert_eq!(eval_str("(sorted-set 3 1 2)"), "#{1 2 3}");
    assert_eq!(eval_str("(sorted-set \"b\" \"a\")"), "#{\"a\" \"b\"}");
    assert_eq!(eval_str("(sorted-set :b :a/z :a)"), "#{:a :b :a/z}");
    assert_eq!(
        eval_str("(sorted-set [1 2] [1] [0 9])"),
        "#{[0 9] [1] [1 2]}"
    );
    assert_eq!(eval_str("(sorted-set 2 1/2 1.5 nil)"), "#{nil 1/2 1.5 2}");
    assert_eq!(eval_str("(sorted-map :b 2 :a 1)"), "{:a 1, :b 2}");
    assert_eq!(eval_str("(conj (sorted-set 3 1) 2)"), "#{1 2 3}");
    assert_eq!(eval_str("(assoc (sorted-map 3 :c) 1 :a)"), "{1 :a, 3 :c}");
    assert_eq!(eval_str("(into (sorted-map) {:b 2 :a 1})"), "{:a 1, :b 2}");
    assert_eq!(
        eval("(sorted? (into (sorted-map) [[:b 2] [:a 1]]))"),
        Value::Bool(true)
    );
}

#[test]
fn sorted_order_agrees_with_compare() {
    let src = r#"
        (every? (fn [g]
                  (= (seq (apply sorted-set g)) (seq (sort compare g))))
                [[3 -1 2/3 1.5 nil 100000000000000000000N]
                 [[1 2] [1] [0 9] [] nil]
                 ["b" "a" "" nil]
                 [:k :a/k :a :b/a nil]
                 ['sym 'a/b 'a nil]
                 [\z \a nil]])
    "#;
    assert_eq!(eval(src), Value::Bool(true));
}

#[test]
fn lookups_of_incomparable_keys_find_nothing() {
    assert_eq!(eval("(get (sorted-map 1 :a) \"x\")"), Value::Nil);
    assert_eq!(eval("(contains? (sorted-set 1 2) :k)"), Value::Bool(false));
    assert_eq!(eval_str("(dissoc (sorted-map 1 :a) :k)"), "{1 :a}");
    assert_eq!(eval_str("(disj (sorted-set 1 2) :k)"), "#{1 2}");
}

#[test]
fn compare_sees_through_metadata() {
    assert_eq!(
        eval("(compare (with-meta [1] {:m true}) [2])"),
        Value::Long(-1)
    );
    assert_eq!(
        eval("(count (sorted-set (with-meta [1] {:m 1}) [1] [2]))"),
        Value::Long(2)
    );
}
