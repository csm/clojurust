//! `:cljrs` is a reader-conditional feature key of this runtime, alongside
//! `:rust`.
//!
//! Portable libraries name each dialect by its own name (`:clj`, `:cljs`,
//! `:cljr`, `:lpy`, ...), and malli keys its clojurust branches `:cljrs`.
//! Only `:rust` used to select, so `#?(:clj [...] :cljrs [...])` with no
//! `:default` read as nothing. When that conditional held a `deftype` field
//! vector, loading `malli.impl.regex` failed with "deftype requires a field
//! vector as second arg", even though the deftype grammar itself was fine.

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

/// Evaluate `src` and return the last value rendered with `pr-str`.
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

#[test]
fn cljrs_key_selects_the_platform_branch() {
    assert_eq!(
        eval_pr("(pr-str #?(:clj :jvm :cljrs :here :default :fallback))"),
        ":here"
    );
}

#[test]
fn cljrs_key_selects_in_quoted_data() {
    assert_eq!(eval_pr("(pr-str '#?(:clj [a] :cljrs [b c]))"), "[b c]");
}

#[test]
fn cljrs_key_splices() {
    assert_eq!(
        eval_pr("(pr-str [1 #?@(:clj [2] :cljrs [3 4]) 5])"),
        "[1 3 4 5]"
    );
}

/// Clauses are tried in source order, as in Clojure: `:default` always
/// matches, so an earlier `:default` wins over a later platform key.
#[test]
fn default_before_platform_key_wins() {
    assert_eq!(
        eval_pr("(pr-str #?(:default :generic :cljrs :specific))"),
        ":generic"
    );
    assert_eq!(
        eval_pr("(pr-str '#?(:default :generic :rust :specific))"),
        ":generic"
    );
}

/// With several `:default` clauses the first one wins, not the last.
#[test]
fn first_default_wins() {
    assert_eq!(
        eval_pr("(pr-str #?(:clj :jvm :default :first :default :second))"),
        ":first"
    );
    assert_eq!(eval_pr("(pr-str [#?@(:default [1] :default [2])])"), "[1]");
}

/// The shape from `malli.impl.regex`: a deftype whose field vector, mutable
/// fields included, is chosen by a reader conditional with no `:default`.
#[test]
fn deftype_field_vector_from_cljrs_reader_conditional() {
    let src = r#"
        (defprotocol ICache (bump! [c]))
        (deftype ^:private Cache
          #?(:clj  [^:unsynchronized-mutable values, ^:unsynchronized-mutable ^long size]
             :cljrs [^:unsynchronized-mutable values, ^:unsynchronized-mutable size])
          ICache
          (bump! [_] (set! size (inc size)) size))
        (let [c (->Cache [] 0)]
          (bump! c)
          (pr-str (bump! c)))
    "#;
    assert_eq!(eval_pr(src), "2");
}
