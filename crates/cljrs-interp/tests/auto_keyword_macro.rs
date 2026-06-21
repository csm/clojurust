//! Regression tests for issue #209: `::keyword` must resolve to the caller's
//! namespace at the point of macroexpansion, not the macro's definition
//! namespace.
//!
//! In Clojure, `::kw` is resolved at READ time.  cljrs keeps `AutoKeyword`
//! nodes in the AST and resolves them lazily; before this fix, macros received
//! the unresolved `AutoKeyword` and re-resolved it against the macro's own
//! namespace (`clojure.core`) rather than the caller's.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::{Keyword, Value};

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `src` in namespace `ns` (via `(ns ...)` switch) and return the
/// last value.  The env starts in `user` so clojure.core is available.
fn eval_in_ns(ns: &str, src: &str) -> Value {
    let (_, mut env) = make_env();
    // Switch to the target namespace so ::kw resolves against it.
    let ns_switch = format!("(ns {ns})");
    let mut parser = Parser::new(ns_switch, "<setup>".to_string());
    for form in parser.parse_all().expect("parse ns") {
        cljrs_interp::eval::eval(&form, &mut env).expect("eval ns");
    }
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

fn kw(ns: &str, name: &str) -> Value {
    Value::keyword(Keyword::qualified(ns, name))
}

// ── Core regression: issue #209 ──────────────────────────────────────────────

#[test]
fn auto_kw_resolves_to_caller_ns_through_if_not() {
    // The original failing case from the issue report.
    let result = eval_in_ns(
        "my.app",
        r#"(let [v ::error] (if-not (= v ::error) :WRONG :right))"#,
    );
    assert_eq!(
        result,
        Value::keyword(Keyword::simple("right")),
        "if-not must not re-resolve ::error against clojure.core"
    );
}

#[test]
fn auto_kw_bound_value_is_namespace_qualified() {
    // Binding ::error in let produces :my.app/error, not :error.
    let result = eval_in_ns("my.app", r#"(let [v ::error] v)"#);
    assert_eq!(result, kw("my.app", "error"));
}

#[test]
fn auto_kw_equality_holds_through_if_not() {
    // Both sides of the comparison must resolve to the same qualified keyword.
    let result = eval_in_ns(
        "acme.core",
        r#"(let [v ::status] (if-not (= v ::status) false true))"#,
    );
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn plain_if_auto_kw_still_works() {
    // Plain `if` was already correct; ensure we haven't regressed it.
    let result = eval_in_ns(
        "my.app",
        r#"(let [v ::error] (if (= v ::error) :right :WRONG))"#,
    );
    assert_eq!(result, Value::keyword(Keyword::simple("right")));
}

#[test]
fn explicit_qualified_kw_works_through_if_not() {
    // Explicit :my.app/error must also be fine (was already working).
    let result = eval_in_ns(
        "my.app",
        r#"(let [v ::error] (if-not (= v :my.app/error) :WRONG :right))"#,
    );
    assert_eq!(result, Value::keyword(Keyword::simple("right")));
}

#[test]
fn auto_kw_through_when_not() {
    // when-not is also built on if-not; exercise that path.
    let result = eval_in_ns("my.app", r#"(let [v ::ok] (when-not (= v ::ok) :WRONG))"#);
    // (when-not <truthy-cond>) → nil
    assert_eq!(result, Value::Nil);
}

#[test]
fn auto_kw_different_namespaces_are_not_equal() {
    // ::error in ns1 != ::error in ns2.
    let r1 = eval_in_ns("ns1", r#"(let [v ::error] v)"#);
    let r2 = eval_in_ns("ns2", r#"(let [v ::error] v)"#);
    assert_eq!(r1, kw("ns1", "error"));
    assert_eq!(r2, kw("ns2", "error"));
    assert_ne!(r1, r2);
}

#[test]
fn auto_kw_equality_with_direct_let() {
    // Sanity check: even without a macro, both sides of = resolve identically.
    let result = eval_in_ns("my.app", r#"(= ::error ::error)"#);
    assert_eq!(result, Value::Bool(true));
}
