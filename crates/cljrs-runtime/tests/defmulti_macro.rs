//! `defmulti` and `defmethod` are Clojure macros over `multi-fn` and
//! `add-method`.
//!
//! Neither surface form needed the interpreter. `eval_defmulti` parsed an
//! optional docstring and a flat `:default val` pair, then built a `MultiFn` and
//! interned it — but `def` already interns, and nothing in minting a `MultiFn`
//! reads the environment. `eval_defmethod` looked the multimethod up with
//! `lookup_in_ns(current_ns, name)`, passing the WHOLE symbol string, so
//! `(defmethod other.ns/m ...)` could not find a multimethod that plainly
//! existed — the same defect PR #354 fixed for protocols named in an impl
//! position. Naming the multimethod by symbol in the expansion resolves it
//! through the ordinary rules instead, aliases included.
//!
//! These tests pin the grammar, that fix, and the two primitives.

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

fn eval_pr(src: &str) -> String {
    let (_globals, mut env) = make_env();
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

// ── the grammar ──────────────────────────────────────────────────────────────

#[test]
fn dispatch_and_default() {
    let src = "(defmulti area :shape)
               (defmethod area :square [s] (* (:side s) (:side s)))
               (defmethod area :default [_] :unknown)
               (pr-str [(area {:shape :square :side 3}) (area {:shape :blob})])";
    assert_eq!(eval_pr(src), "[9 :unknown]");
}

#[test]
fn a_docstring_is_not_read_as_the_dispatch_fn() {
    let src = "(defmulti area \"the area of a shape\" :shape)
               (defmethod area :square [s] (* (:side s) (:side s)))
               (pr-str (area {:shape :square :side 4}))";
    assert_eq!(eval_pr(src), "16");
}

#[test]
fn the_default_dispatch_value_can_be_chosen() {
    let src = "(defmulti f identity :default :other)
               (defmethod f 1 [x] [:one x])
               (defmethod f :other [x] [:fallback x])
               (pr-str [(f 1) (f 99)])";
    assert_eq!(eval_pr(src), "[[:one 1] [:fallback 99]]");
}

/// `:default nil` names nil as the fallback dispatch value, which is not the
/// same as leaving `:default` out — so presence has to be tested, not
/// truthiness.
#[test]
fn nil_is_a_usable_default_dispatch_value() {
    let src = "(defmulti f identity :default nil)
               (defmethod f nil [x] [:fallback x])
               (pr-str (f 99))";
    assert_eq!(eval_pr(src), "[:fallback 99]");
}

// ── the qualified-name fix ───────────────────────────────────────────────────

#[test]
fn defmethod_finds_a_multimethod_in_another_namespace() {
    let src = "(ns other.ns)
               (defmulti describe :kind)
               (ns user)
               (alias 'o 'other.ns)
               (defmethod o/describe :thing [x] [:described (:kind x)])
               (pr-str (o/describe {:kind :thing}))";
    assert_eq!(eval_pr(src), "[:described :thing]");
}

#[test]
fn defmethod_on_a_name_that_is_not_a_multimethod_errors() {
    let (_globals, mut env) = make_env();
    let src = "(def notamulti 42)
               (defmethod notamulti :x [_] 1)";
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
    }
    let err = last.expect_err("registering a method on a non-multimethod must fail");
    assert!(
        format!("{err:?}").contains("multimethod"),
        "expected a multimethod type error, got {err:?}"
    );
}

// ── what the forms bind and return ───────────────────────────────────────────

#[test]
fn defmulti_returns_the_var() {
    assert_eq!(eval_pr("(pr-str (defmulti m identity))"), "#'user/m");
}

#[test]
fn the_name_keeps_its_metadata() {
    let src = "(defmulti ^{:marker true} m identity)
               (pr-str (:marker (meta (var m))))";
    assert_eq!(eval_pr(src), "true");
}

// ── the primitives ───────────────────────────────────────────────────────────

#[test]
fn multi_fn_and_add_method_are_callable_on_their_own() {
    let src = "(def m (multi-fn 'm :shape))
               (add-method m :square (fn [s] (:side s)))
               (pr-str (m {:shape :square :side 7}))";
    assert_eq!(eval_pr(src), "7");
}

/// `add-method` and `remove-method` must key the method table identically, or
/// a method registered through one would be unreachable by the other.
#[test]
fn add_method_and_remove_method_agree_on_the_key() {
    let src = "(defmulti area :shape)
               (defmethod area :square [_] :square-area)
               (pr-str [(count (methods area))
                        (do (remove-method area :square) (count (methods area)))])";
    assert_eq!(eval_pr(src), "[1 0]");
}

#[test]
fn multi_fn_defaults_its_default_dispatch_value() {
    let src = "(def m (multi-fn 'm identity))
               (add-method m :default (fn [_] :fell-through))
               (pr-str (m :anything))";
    assert_eq!(eval_pr(src), ":fell-through");
}
