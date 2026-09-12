//! `defprotocol` is a Clojure macro over the `protocol*` primitive.
//!
//! The Rust special form did four things: parse the body grammar (docstring,
//! flat `:option value` pairs, method specs), derive each method's arity from
//! its parameter vector, mint the `Protocol`, and intern the protocol var plus
//! one `ProtocolFn` var per method. Only the third needs the interpreter, and
//! only for one reason: `Protocol.ns` is what qualifies a method name for
//! extend-via-metadata dispatch, and a builtin fn cannot see the environment.
//!
//! So `protocol*` keeps that, `protocol-fn` reads a method's arity back out of
//! the protocol that already states it, and the grammar moves to Clojure.
//! These tests pin the grammar and the two properties the split could break:
//! the namespace the protocol is minted in, and the arity carried into each
//! dispatch fn.

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

// ── the body grammar ─────────────────────────────────────────────────────────

#[test]
fn a_docstring_is_not_read_as_a_method() {
    let src = "(defprotocol IShow \"how a thing shows itself\" (-show [x]))
               (extend-protocol IShow Long (-show [n] (str \"L:\" n)))
               (pr-str (-show 42))";
    assert_eq!(eval_pr(src), "\"L:42\"");
}

#[test]
fn option_pairs_may_sit_among_the_method_specs() {
    let src = "(defprotocol IShow
                 (-show [x])
                 :extend-via-metadata true
                 (-tag [x]))
               (extend-protocol IShow Long (-show [n] :shown) (-tag [n] :tagged))
               (pr-str [(-show 1) (-tag 1)])";
    assert_eq!(eval_pr(src), "[:shown :tagged]");
}

/// A method's declared arity reaches its dispatch fn: `-show` is declared at
/// two arguments, so a one-argument call must be rejected on arity rather than
/// dispatched.
#[test]
fn the_declared_arity_reaches_the_dispatch_fn() {
    let (_globals, mut env) = make_env();
    let src = "(defprotocol IPair (-pair [x y]))
               (extend-protocol IPair Long (-pair [x y] [x y]))
               (-pair 1)";
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
    }
    let err = last.expect_err("a one-argument call to a two-argument method must fail");
    assert!(
        format!("{err:?}").contains("rity"),
        "expected an arity error, got {err:?}"
    );
}

#[test]
fn a_variadic_method_spec_accepts_extra_arguments() {
    let src = "(defprotocol IJoin (-join [x & more]))
               (extend-protocol IJoin Long (-join [x & more] (apply str x more)))
               (pr-str (-join 1 2 3))";
    assert_eq!(eval_pr(src), "\"123\"");
}

// ── the namespace the protocol is minted in ──────────────────────────────────
//
// `Protocol.ns` is the whole reason `protocol*` stays a special form: it
// qualifies the method symbol that extend-via-metadata dispatch looks up in a
// value's metadata. If the macro minted the protocol in the wrong namespace the
// type-tag path would still work and only this would break.

#[test]
fn extend_via_metadata_dispatches_on_the_qualified_method_symbol() {
    let src = "(defprotocol IShow :extend-via-metadata true (-show [x]))
               (extend-protocol IShow Long (-show [n] :by-tag))
               (pr-str [(-show 1)
                        (-show (with-meta [1] {'user/-show (fn [v] :by-meta)}))])";
    assert_eq!(eval_pr(src), "[:by-tag :by-meta]");
}

#[test]
fn a_protocol_minted_in_another_ns_carries_that_ns() {
    let src = "(ns mini.proto)
               (defprotocol IThing :extend-via-metadata true (-describe [this]))
               (ns user)
               (alias 'mp 'mini.proto)
               (pr-str (mp/-describe
                         (with-meta [] {'mini.proto/-describe (fn [_] :from-meta)})))";
    assert_eq!(eval_pr(src), ":from-meta");
}

#[test]
fn extend_via_metadata_stays_off_unless_asked_for() {
    let (_globals, mut env) = make_env();
    let src = "(defprotocol IPlain (-p [x]))
               (extend-protocol IPlain Long (-p [n] :tagged))
               (-p (with-meta [] {'user/-p (fn [_] :from-meta)}))";
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
    }
    let err = last.expect_err("a metadata impl must not be consulted for a plain protocol");
    assert!(
        format!("{err:?}").contains("No implementation"),
        "expected a missing-implementation error, got {err:?}"
    );
}

// ── what defprotocol binds ───────────────────────────────────────────────────

#[test]
fn defprotocol_returns_the_protocol_var() {
    let src = "(pr-str (defprotocol IShow (-show [x])))";
    assert_eq!(eval_pr(src), "#'user/IShow");
}

#[test]
fn the_name_keeps_its_metadata() {
    let src = "(defprotocol ^{:marker true} IShow (-show [x]))
               (pr-str (:marker (meta (var IShow))))";
    assert_eq!(eval_pr(src), "true");
}

// ── the primitive ────────────────────────────────────────────────────────────

#[test]
fn protocol_star_mints_a_protocol_that_extend_can_be_handed() {
    let src = "(def P (protocol* IShow [{:name \"-show\" :min-arity 1 :variadic false}]))
               (def -show (protocol-fn P \"-show\"))
               (extend 'Long P {:-show (fn [n] :shown)})
               (pr-str (-show 1))";
    assert_eq!(eval_pr(src), ":shown");
}

/// `protocol-fn` reads the arity out of the protocol rather than taking it
/// again: the spec vector is the single place a method's arity is stated.
#[test]
fn protocol_fn_reads_the_arity_from_the_protocol() {
    let (_globals, mut env) = make_env();
    let src = "(def P (protocol* IPair [{:name \"-pair\" :min-arity 2 :variadic false}]))
               (def -pair (protocol-fn P \"-pair\"))
               (extend 'Long P {:-pair (fn [x y] [x y])})
               (-pair 1)";
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
    }
    let err = last.expect_err("the protocol's own min-arity must gate the call");
    assert!(
        format!("{err:?}").contains("rity"),
        "expected an arity error, got {err:?}"
    );
}

#[test]
fn protocol_fn_refuses_a_method_the_protocol_does_not_declare() {
    let (_globals, mut env) = make_env();
    let src = "(def P (protocol* IShow [{:name \"-show\" :min-arity 1 :variadic false}]))
               (protocol-fn P \"-nope\")";
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut last = Ok(Value::Nil);
    for form in forms {
        last = cljrs_runtime::interp::eval::eval(&form, &mut env);
    }
    let err = last.expect_err("an undeclared method must not silently produce a dispatch fn");
    assert!(
        format!("{err:?}").contains("-nope"),
        "expected the method name in the error, got {err:?}"
    );
}
