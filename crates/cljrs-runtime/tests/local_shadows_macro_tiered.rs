//! A local binding shadows a macro at call position once the function that
//! holds the call is IR-promoted, too.
//!
//! Lowering macro-expands a function body ahead of time, outside the frames
//! the tree-walker would have had: the function's own parameters are not
//! bound, and whatever the *caller* happens to have bound is. So the locals in
//! scope have to come from the body's own binding forms and from the function
//! being lowered, never from the environment that triggered the lowering.
//!
//! This file is its own binary so it can flip the process-wide eager-lowering
//! switch, and drives each function far past the warm threshold so the tier is
//! genuinely entered.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env(mode: ExecutionMode) -> (Arc<GlobalEnv>, Env) {
    // Process-wide, and the reason this test is its own binary.
    cljrs_runtime::tiered::force_eager_lowering();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `body` after defining the macro `m`, which expands to `:macro`
/// whatever its arguments, and the function `local`, which answers `:local`.
/// `body` is evaluated 3000 times; the result is the set of answers, printed.
///
/// The tree-walker is the reference: the promoted function must answer what
/// it answers.
fn answers(defs: &str, body: &str) -> String {
    let src = format!(
        "(defmacro m [& _] :macro)
         (defn local [& _] :local)
         {defs}
         (pr-str (set (map (fn [_] {body}) (range 3000))))"
    );
    let walked = answers_in(ExecutionMode::TreeWalk, &src);
    let promoted = answers_in(ExecutionMode::TieredNoJit, &src);
    assert_eq!(promoted, walked, "promoted (left), tree-walked (right)");
    promoted
}

fn answers_in(mode: ExecutionMode, src: &str) -> String {
    let (_globals, mut env) = make_env(mode);
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

#[test]
fn a_parameter_shadows_a_macro() {
    assert_eq!(answers("(defn f [m] (m 1))", "(f local)"), "#{:local}");
}

#[test]
fn a_rest_parameter_shadows_a_macro() {
    // A rest parameter is a list, which is not a function: what matters is
    // that the call is made, not expanded.
    assert_eq!(
        answers(
            "(defn f [& m] (try (if (= :macro (m 0)) :macro :called)
                                (catch Exception _ :called)))",
            "(f 1)"
        ),
        "#{:called}"
    );
}

#[test]
fn a_destructured_parameter_shadows_a_macro() {
    assert_eq!(
        answers("(defn f [{:keys [m]}] (m 1))", "(f {:m local})"),
        "#{:local}"
    );
    assert_eq!(
        answers("(defn f [[_ m]] (m 1))", "(f [0 local])"),
        "#{:local}"
    );
}

#[test]
fn a_let_binding_shadows_a_macro() {
    assert_eq!(
        answers("(defn f [g] (let [m g] (m 1)))", "(f local)"),
        "#{:local}"
    );
    assert_eq!(
        answers("(defn f [g] (let [{:keys [m]} {:m g}] (m 1)))", "(f local)"),
        "#{:local}"
    );
}

#[test]
fn a_let_binding_is_not_in_scope_of_its_own_value() {
    // `(m 1)` in the value position is still the macro; the body's is the local.
    assert_eq!(
        answers(
            "(defn f [g] (let [m (if (= :macro (m 1)) g nil)] (m 1)))",
            "(f local)"
        ),
        "#{:local}"
    );
}

#[test]
fn a_loop_binding_shadows_a_macro() {
    assert_eq!(
        answers("(defn f [g] (loop [m g] (m 1)))", "(f local)"),
        "#{:local}"
    );
}

#[test]
fn a_letfn_binding_shadows_a_macro() {
    assert_eq!(
        answers("(defn f [] (letfn [(m [x] :local)] (m 1)))", "(f)"),
        "#{:local}"
    );
}

#[test]
fn a_closed_over_local_shadows_a_macro() {
    assert_eq!(
        answers("(defn mk [m] (fn [] (m 1))) (def g (mk local))", "(g)"),
        "#{:local}"
    );
}

#[test]
fn a_fn_name_shadows_a_macro() {
    assert_eq!(
        answers(
            "(defn f [] ((fn m [n] (if (= n 0) :local (m 0))) 1))",
            "(f)"
        ),
        "#{:local}"
    );
}

#[test]
fn a_caught_exception_binding_shadows_a_macro() {
    assert_eq!(
        answers(
            "(defn f [] (try (throw (ex-info \"x\" {})) (catch Exception m (ex-message m))))",
            "(f)"
        ),
        "#{\"x\"}"
    );
    // Called as a function the exception is not callable, so the call throws
    // rather than expanding the macro.
    assert_eq!(
        answers(
            "(defn f [] (try (try (throw (ex-info \"x\" {})) (catch Exception m (m 1)))
                             (catch Exception _ :not-callable)))",
            "(f)"
        ),
        "#{:not-callable}"
    );
}

#[test]
fn the_macro_still_expands_where_nothing_shadows_it() {
    assert_eq!(answers("(defn f [g] (m 1))", "(f local)"), "#{:macro}");
    // The shadowing binding has ended.
    assert_eq!(
        answers("(defn f [g] (let [m g] nil) (m 1))", "(f local)"),
        "#{:macro}"
    );
    // A qualified symbol is never a local.
    assert_eq!(answers("(defn f [m] (user/m 1))", "(f local)"), "#{:macro}");
    // Inside collection literals.
    assert_eq!(
        answers("(defn f [g] [(m 1) {:k (m 1)} #{(m 1)}])", "(f local)"),
        "#{[:macro {:k :macro} #{:macro}]}"
    );
}

#[test]
fn a_callers_local_does_not_shadow_a_macro_in_the_callee() {
    // `g` is lowered while a caller frame binds `m`. That frame is not in
    // scope in `g`'s body.
    assert_eq!(
        answers("(defn g [] (m 1)) (defn f [m] (g))", "(f local)"),
        "#{:macro}"
    );
    assert_eq!(
        answers("(defn g [] (m 1))", "(let [m local] (g))"),
        "#{:macro}"
    );
}
