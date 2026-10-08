//! Regression tests: a local binding shadows a core macro at call position.
//!
//! cljrs defines `doc` (and friends) as macros in bootstrap.cljrs, loaded into
//! clojure.core. Before this fix, `resolve_macro` looked the head symbol up in
//! the global namespace without consulting the local frames, so
//! (let [doc (fn [x] :local)] (doc 1)) expanded clojure.core/doc on the
//! literal argument and answered its return value (nil), never calling the
//! local. Clojure semantics: locals shadow macros — the compiler checks the
//! local scope before macroexpanding a call.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_src(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn let_local_shadows_core_macro() {
    let result = eval_src("(let [doc (fn [x] :local)] (doc 1))");
    assert_eq!(result, Value::keyword(cljrs_value::Keyword::parse("local")));
}

#[test]
fn fn_param_shadows_core_macro() {
    let result = eval_src("((fn [doc] (doc 1)) (fn [x] :param))");
    assert_eq!(result, Value::keyword(cljrs_value::Keyword::parse("param")));
}

#[test]
fn local_named_like_macro_holding_a_var_calls_the_var() {
    // The exact shape that surfaced this: a let-bound var resolved by
    // ns-resolve must be CALLED, not macroexpanded away.
    let result = eval_src(
        "(defn document [& xs] (count xs)) \
         (let [doc (ns-resolve (find-ns 'user) 'document)] (doc :a :b :c))",
    );
    assert_eq!(result, Value::Long(3));
}

#[test]
fn core_macro_still_expands_without_a_local() {
    // doc expands and runs (its answer for these inputs is nil; the point is
    // expansion happened rather than an unbound-symbol error).
    let result = eval_src("(doc 'some-undefined-symbol)");
    assert_eq!(result, Value::Nil);
    // A macro whose expansion is observable in the value, not just in not
    // erroring: when expands to an if.
    let result = eval_src("(when true :expanded)");
    assert_eq!(
        result,
        Value::keyword(cljrs_value::Keyword::parse("expanded"))
    );
}

// ── macroexpand_all: the form's own binding forms are its scope ─────────────
//
// `macroexpand_all` expands a form ahead of its evaluation (lowering, AOT,
// the nREPL `macroexpand-all` op), so the locals that shadow a macro are the
// ones the form itself binds. These tests read the expansion directly; in
// each, `m` is a macro that expands to `:macro` whatever it is given.

use cljrs_reader::form::{Form, FormKind};

/// A form as text, for the kinds these tests use.
fn show(form: &Form) -> String {
    let seq = |open: &str, items: &[Form], close: &str| {
        let items: Vec<String> = items.iter().map(show).collect();
        format!("{open}{}{close}", items.join(" "))
    };
    match &form.kind {
        FormKind::Nil => "nil".to_string(),
        FormKind::Int(n) => n.to_string(),
        FormKind::Str(s) => format!("{s:?}"),
        FormKind::Symbol(s) => s.clone(),
        FormKind::Keyword(k) => format!(":{k}"),
        FormKind::List(items) => seq("(", items, ")"),
        FormKind::Vector(items) => seq("[", items, "]"),
        FormKind::Map(items) => seq("{", items, "}"),
        FormKind::Set(items) => seq("#{", items, "}"),
        other => panic!("show: unexpected form {other:?}"),
    }
}

/// `src`, fully expanded inside the bindings `locals`.
fn expanded_in(locals: &[&str], src: &str) -> String {
    let (_, mut env) = make_env();
    let define = Parser::new(
        "(defmacro m [& _] :macro)".to_string(),
        "<test>".to_string(),
    )
    .parse_all()
    .expect("parse error");
    cljrs_runtime::interp::eval::eval(&define[0], &mut env).expect("eval error");
    let form = Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse error")
        .remove(0);
    let locals: Vec<Arc<str>> = locals.iter().map(|l| Arc::from(*l)).collect();
    let expanded = cljrs_runtime::interp::macros::macroexpand_all_in(&form, &mut env, &locals)
        .expect("expand error");
    show(&expanded)
}

/// `src`, fully expanded at the top level.
fn expanded(src: &str) -> String {
    expanded_in(&[], src)
}

/// Assert that `src` expands to itself.
fn assert_unchanged(src: &str) {
    assert_eq!(expanded(src), src);
}

#[test]
fn a_macro_call_expands_wherever_nothing_binds_its_name() {
    assert_eq!(expanded("(m 1)"), ":macro");
    assert_eq!(expanded("(f (m 1))"), "(f :macro)");
    assert_eq!(
        expanded("[(m 1) {:k (m 1)} #{(m 1)}]"),
        "[:macro {:k :macro} #{:macro}]"
    );
    assert_eq!(expanded("()"), "()");
}

#[test]
fn a_quoted_form_is_not_expanded() {
    assert_unchanged("(quote (m 1))");
}

#[test]
fn the_locals_a_form_stands_in_shadow_a_macro() {
    assert_eq!(expanded_in(&["m"], "(m 1)"), "(m 1)");
    assert_eq!(expanded_in(&["x", "m"], "(f (m 1))"), "(f (m 1))");
    assert_eq!(expanded_in(&["x"], "(m 1)"), ":macro");
    // A qualified symbol is never a local.
    assert_eq!(expanded_in(&["m"], "(user/m 1)"), ":macro");
}

#[test]
fn a_parameter_is_in_scope_in_its_own_arity_only() {
    assert_unchanged("(fn [m] (m 1))");
    assert_unchanged("(fn* [m] (m 1))");
    assert_unchanged("(fn [x & m] (m 1))");
    assert_eq!(
        expanded("(fn ([m] (m 1)) ([x] (m 1)))"),
        "(fn ([m] (m 1)) ([x] :macro))"
    );
    assert_eq!(
        expanded("(do (fn [m] (m 1)) (m 1))"),
        "(do (fn [m] (m 1)) :macro)"
    );
    assert_eq!(expanded("(fn [x] (m 1))"), "(fn [x] :macro)");
}

#[test]
fn a_function_name_is_in_scope_in_every_arity() {
    assert_unchanged("(fn m ([x] (m 1)) ([x y] (m 1)))");
    assert_unchanged("(defn m [x] (m 1))");
    assert_unchanged("(defn- m [x] (m 1))");
    assert_eq!(
        expanded("(defn f \"doc\" {:a (m 1)} [m] (m 1))"),
        "(defn f \"doc\" {:a :macro} [m] (m 1))"
    );
    assert_eq!(
        expanded("(defmacro g [x] (m 1))"),
        "(defmacro g [x] :macro)"
    );
}

#[test]
fn a_destructuring_pattern_binds_every_name_in_it() {
    for pattern in [
        "[_ m]",
        "[_ & m]",
        "[_ :as m]",
        "[[m]]",
        "{m :k}",
        "{[m] :k}",
        "{:keys [m]}",
        "{:keys [:m]}",
        "{:keys [a/m]}",
        "{:keys [:a/m]}",
        "{:a/keys [m]}",
        "{:strs [m]}",
        "{:syms [m]}",
        "{:as m}",
    ] {
        assert_unchanged(&format!("(fn [{pattern}] (m 1))"));
        assert_unchanged(&format!("(let [{pattern} x] (m 1))"));
    }
}

#[test]
fn what_a_pattern_does_not_bind_does_not_shadow() {
    // `&` separates; an `:or` key names a binding made elsewhere.
    for pattern in ["[x & y]", "{x :k :or {m 1}}", "{:keys [x] :other m}"] {
        assert_eq!(
            expanded(&format!("(fn [{pattern}] (m 1))")),
            format!("(fn [{pattern}] :macro)")
        );
    }
    // A default under `:or` is an expression, expanded in the enclosing scope.
    assert_eq!(
        expanded("(fn [{x :k :or {x (m 1)}}] x)"),
        "(fn [{x :k :or {x :macro}}] x)"
    );
}

#[test]
fn a_let_binding_is_in_scope_after_its_own_value() {
    assert_eq!(
        expanded("(let [x (m 1) m x y (m 1)] (m 1))"),
        "(let [x :macro m x y (m 1)] (m 1))"
    );
    assert_eq!(
        expanded("(let* [m (m 1)] (m 1))"),
        "(let* [m :macro] (m 1))"
    );
    assert_eq!(
        expanded("(loop [m (m 1)] (m 1))"),
        "(loop [m :macro] (m 1))"
    );
    assert_eq!(
        expanded("(loop* [m (m 1)] (m 1))"),
        "(loop* [m :macro] (m 1))"
    );
    assert_eq!(
        expanded("(do (let [m 1] (m 1)) (m 1))"),
        "(do (let [m 1] (m 1)) :macro)"
    );
    // A malformed binding vector is left for the evaluator to refuse.
    assert_eq!(expanded("(let [m] (m 1))"), "(let [m] (m 1))");
    assert_eq!(expanded("(let x (m 1))"), "(let x :macro)");
}

#[test]
fn letfn_names_are_in_scope_in_every_function_and_the_body() {
    assert_unchanged("(letfn [(m [x] (g x)) (g [x] (m x))] (m 1))");
    assert_eq!(
        expanded("(letfn [(g [x] (m x)) (h ([m] (m 1)) ([x] (m 1)))] (m 1))"),
        "(letfn [(g [x] :macro) (h ([m] (m 1)) ([x] :macro))] :macro)"
    );
    assert_eq!(
        expanded("(do (letfn [(m [x] x)] (m 1)) (m 1))"),
        "(do (letfn [(m [x] x)] (m 1)) :macro)"
    );
    // Malformed specs are left for the evaluator to refuse.
    assert_eq!(expanded("(letfn [() x] (m 1))"), "(letfn [() x] :macro)");
    assert_eq!(expanded("(letfn x (m 1))"), "(letfn x :macro)");
}

#[test]
fn a_caught_exception_is_in_scope_in_its_handler() {
    assert_unchanged("(try (f) (catch Exception m (m 1)))");
    assert_eq!(
        expanded("(try (m 1) (catch Exception e (m 1)) (finally (m 1)))"),
        "(try :macro (catch Exception e :macro) (finally :macro))"
    );
    assert_eq!(
        expanded("(do (try (f) (catch Exception m (m 1))) (m 1))"),
        "(do (try (f) (catch Exception m (m 1))) :macro)"
    );
    // Too short to bind anything.
    assert_eq!(expanded("(catch Exception)"), "(catch Exception)");
}
