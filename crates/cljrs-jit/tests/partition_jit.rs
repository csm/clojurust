//! Regression tests for issue #205 via the JIT compiler tier.
//!
//! Boots the full stdlib with JIT enabled, hammers `partition` calls until
//! the background worker emits native code, and confirms the JIT result
//! matches the interpreter result for all three arities.

#![cfg(not(feature = "no-gc"))]

use cljrs_eval::{Env, eval};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (std::sync::Arc<cljrs_env::env::GlobalEnv>, Env) {
    cljrs_jit::init();
    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_stdlib::standard_env();
    // Wait for the compiler-namespace background load.
    while !globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_str(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = eval(&form, env).expect("eval error");
    }
    result
}

fn assert_partition_jit(expr: &str, expected: &str, env: &mut Env) {
    // Wrap in a defn and call it many times to trigger JIT compilation.
    eval_str(
        &format!("(defn __partition_test_fn [] (mapv vec {}))", expr),
        env,
    );
    let mut last = Value::Nil;
    for _ in 0..60 {
        last = eval_str("(__partition_test_fn)", env);
    }
    let normalised = eval_str("(pr-str (__partition_test_fn))", env);
    match normalised {
        Value::Str(s) => assert_eq!(
            s.get().as_str(),
            expected,
            "JIT partition mismatch for {}",
            expr
        ),
        other => panic!("pr-str returned non-string: {:?}", other),
    }
    let _ = last;
}

#[test]
fn jit_partition_2arg() {
    let (_globals, mut env) = make_env();
    assert_partition_jit("(partition 2 [1 2 3 4])", "[[1 2] [3 4]]", &mut env);
}

#[test]
fn jit_partition_3arg() {
    let (_globals, mut env) = make_env();
    assert_partition_jit("(partition 2 1 [1 2 3 4])", "[[1 2] [2 3] [3 4]]", &mut env);
}

#[test]
fn jit_partition_4arg() {
    let (_globals, mut env) = make_env();
    assert_partition_jit(
        "(partition 2 2 (repeat nil) [1 2 3])",
        "[[1 2] [3 nil]]",
        &mut env,
    );
}
