//! Regression coverage for unused locals preventing closure lowering (#487).

mod common;

use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval(env: &mut Env, source: &str) -> Value {
    common::eval_in(env, source).expect(source)
}

fn captures(value: &Value) -> Vec<String> {
    let Value::Fn(f) = value else {
        panic!("expected function, got {value:?}");
    };
    let mut names: Vec<_> = f
        .get()
        .closed_over_names
        .iter()
        .map(ToString::to_string)
        .collect();
    names.sort();
    names
}

#[test]
fn unused_locals_do_not_become_captures() {
    let (_, mut env) = common::fresh_env();
    for source in [
        "(let [x 100] (fn [v] (+ v 1)))",
        "(let [x 100] (fn [] 1))",
        "(let [x 100] (fn [x] x))",
        "(let [x 100] (fn [& x] x))",
        "(let [x 100] (fn x [] x))",
        "(let [x 100] (fn [] 'x))",
        "(let [x 100] (fn [] (quote x)))",
        "(let [x 100] (fn [] (let [x 1] x)))",
        "(let [x 100] (fn [] (loop [x 1] x)))",
        "(let [x 100] (fn [] (fn [x] x)))",
        "(let [x 100] (fn [] (fn ([x] x) ([x y] (+ x y)))))",
    ] {
        let f = eval(&mut env, source);
        assert!(captures(&f).is_empty(), "{source}: {:?}", captures(&f));
    }
}

#[test]
fn captures_are_the_union_of_arities_and_include_nested_references() {
    let (_, mut env) = common::fresh_env();
    let f = eval(
        &mut env,
        "(let [x 10 y 20 unused 99] (fn ([] x) ([v] (+ y v))))",
    );
    assert_eq!(captures(&f), ["x", "y"]);
    env.globals.intern(&env.current_ns, "f".into(), f);
    assert_eq!(eval(&mut env, "[(f) (f 2)]"), eval(&mut env, "[10 22]"));

    let f = eval(&mut env, "(let [x 10 unused 99] (fn [] (fn [v] (+ x v))))");
    assert_eq!(captures(&f), ["x"]);
    env.globals.intern(&env.current_ns, "f".into(), f);
    assert_eq!(eval(&mut env, "((f) 2)"), Value::Long(12));
}

#[test]
fn initializer_sees_outer_binding_before_shadowing_it() {
    let (_, mut env) = common::fresh_env();
    let f = eval(
        &mut env,
        "(let [x 10 unused 99] (fn [] (let [x (inc x)] x)))",
    );
    assert_eq!(captures(&f), ["x"]);
    env.globals.intern(&env.current_ns, "f".into(), f);
    assert_eq!(eval(&mut env, "(f)"), Value::Long(11));
    let f = eval(&mut env, "(let [x 1] (let [x 2] (fn [] x)))");
    assert_eq!(captures(&f), ["x"]);
    env.globals.intern(&env.current_ns, "f".into(), f);
    assert_eq!(eval(&mut env, "(f)"), Value::Long(2));
}

#[test]
fn macro_expansion_keeps_implicit_locals_and_stays_lazy() {
    let (_, mut env) = common::fresh_env();
    eval(
        &mut env,
        "(def expansions (atom 0))
         (defmacro hidden-x [] (swap! expansions inc) 'x)
         (defmacro env-x [] (get &env 'x))
         (def f (let [x 42] (fn [] (hidden-x))))
         (def g (let [x 43] (fn [] (env-x))))",
    );
    assert_eq!(eval(&mut env, "@expansions"), Value::Long(0));
    assert_eq!(eval(&mut env, "(f)"), Value::Long(42));
    assert_eq!(eval(&mut env, "(g)"), Value::Long(43));
    assert_eq!(eval(&mut env, "@expansions"), Value::Long(1));
    // A macro call nested in a reader anonymous function needs the same
    // protection, as does a macro supplied as a runtime parameter.
    assert_eq!(
        eval(&mut env, "(let [x 44] (((fn [] #(hidden-x)))))"),
        Value::Long(44)
    );
    assert_eq!(
        eval(&mut env, "(let [x 45] ((fn [m] (m)) hidden-x))"),
        Value::Long(45)
    );
}

#[test]
fn destructuring_defaults_and_mutual_local_functions_still_work() {
    let (_, mut env) = common::fresh_env();
    assert_eq!(
        eval(&mut env, "(let [x 42] ((fn [{:keys [v] :or {v x}}] v) {}))"),
        Value::Long(42)
    );
    assert_eq!(
        eval(
            &mut env,
            "(let [x 42]
               (letfn [(a [n] (if (= n 0) x (b (- n 1))))
                       (b [n] (if (= n 0) x (a (- n 1))))]
                 (a 3)))",
        ),
        Value::Long(42)
    );
}
