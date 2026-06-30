//! Regression tests for auto-gensym tokenization: `x#` inside a syntax-quote
//! must read as a single symbol token (the trailing `#` is part of the
//! symbol, not a `#`-dispatch reader macro), and every occurrence of the same
//! `x#` within one syntax-quote must expand to the same generated symbol.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_src(src: &str) -> Value {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn auto_gensym_symbol_tokenizes_as_one_token() {
    // Before the fix, `x#` would lex as `x` followed by a stray `#`
    // dispatch token, which fails to parse inside `(let [x# 1] x#)`.
    let result = eval_src("(count `(let [x# 1] x#))");
    assert_eq!(result, Value::Long(3));
}

#[test]
fn auto_gensym_is_consistent_within_one_syntax_quote() {
    // Every occurrence of `x#` in the same syntax-quote must read out to the
    // same generated symbol.
    let result = eval_src("(let [expanded `(let [x# 1] x#)] (= (nth (nth expanded 1) 0) (nth expanded 2)))");
    assert_eq!(result, Value::Bool(true));
}

#[test]
fn auto_gensym_differs_across_syntax_quotes() {
    // Two separate syntax-quote forms generate distinct gensyms even for the
    // same base name `x#`.
    let result = eval_src(
        "(let [a (nth (nth `(let [x# 1] x#) 1) 0) b (nth (nth `(let [x# 1] x#) 1) 0)] (= a b))",
    );
    assert_eq!(result, Value::Bool(false));
}

#[test]
fn auto_gensym_via_defmacro_hygiene() {
    // Classic hygienic-macro pattern: a macro-introduced `x#` binding must
    // not capture a caller's `x`.
    let result = eval_src(
        r#"
        (defmacro my-or [a b]
          `(let [x# ~a] (if x# x# ~b)))
        (let [x 100]
          (my-or false x))
        "#,
    );
    assert_eq!(result, Value::Long(100));
}
