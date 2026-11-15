//! Regression coverage for csm/clojurust#512: BigDecimals that are `=` but
//! differ in scale (`1M` vs `1.0M`) must hash equally, so lookups keep working
//! after a map/set grows past its linear array representation.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval_ok(src: &str) -> Value {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .unwrap_or_else(|e| panic!("{src}\n{e:?}"));
    }
    result
}

fn assert_true(src: &str) {
    assert_eq!(eval_ok(src), Value::Bool(true), "{src}");
}

#[test]
fn equal_decimals_hash_equally() {
    assert_true("(= (hash 1M) (hash 1.0M) (hash 1.000M))");
    assert_true("(= (hash 0M) (hash 0.0M) (hash -0.00M))");
    assert_true("(= (hash 100M) (hash 1E2M) (hash 100.00M))");
    assert_true("(= (hash -1.5M) (hash -1.50M))");
}

#[test]
fn map_ops_before_and_after_growth() {
    for n in [0, 20, 200] {
        assert_true(&format!(
            "(let [m (into {{1M :found}} (map (fn [i] [i i]) (range {n})))]
               (and (= :found (get m 1.0M))
                    (contains? m 1.00M)
                    (= :new (get (assoc m 1.0M :new) 1M))
                    (= (count m) (count (assoc m 1.0M :new)))
                    (not (contains? (dissoc m 1.0M) 1M))
                    (= (dec (count m)) (count (dissoc m 1.0M)))))"
        ));
    }
}

#[test]
fn sets_and_nested_keys() {
    for n in [0, 20, 200] {
        assert_true(&format!(
            "(let [s (into #{{1M}} (range {n}))
                   m (into {{[1M {{:a 2.50M}}] :found}} (map (fn [i] [i i]) (range {n})))]
               (and (contains? s 1.0M)
                    (= (count s) (count (conj s 1.0M)))
                    (not (contains? (disj s 1.0M) 1M))
                    (= :found (get m [1.0M {{:a 2.5M}}]))
                    (contains? #{{#{{1M}}}} #{{1.0M}})))"
        ));
    }
}
