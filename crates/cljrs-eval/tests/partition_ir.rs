//! Regression tests for issue #205 via the IR interpreter tier.
//!
//! Verifies that `partition` 3-arg and 4-arg calls are correctly lowered by
//! the ANF pass to `KnownFn::Partition3` / `KnownFn::Partition4` and that the
//! IR interpreter executes them correctly.

use std::sync::Arc;

use cljrs_eval::Env;
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<cljrs_env::env::GlobalEnv>, Env) {
    let globals = cljrs_eval::standard_env();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_str(src: &str) -> Value {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

fn assert_partition(expr: &str, expected: &str) {
    let normalised = eval_str(&format!("(pr-str (mapv vec {}))", expr));
    match normalised {
        Value::Str(s) => assert_eq!(
            s.get().as_str(),
            expected,
            "partition mismatch for {}",
            expr
        ),
        other => panic!("pr-str returned non-string: {:?}", other),
    }
}

// ── 2-arg (regression: must stay correct after the fix) ─────────────────────

#[test]
fn ir_partition_2arg_basic() {
    assert_partition("(partition 2 [1 2 3 4])", "[[1 2] [3 4]]");
}

// ── 3-arg ────────────────────────────────────────────────────────────────────

#[test]
fn ir_partition_3arg_overlapping() {
    assert_partition("(partition 2 1 [1 2 3 4])", "[[1 2] [2 3] [3 4]]");
}

#[test]
fn ir_partition_3arg_skipping() {
    assert_partition("(partition 2 3 [1 2 3 4 5 6])", "[[1 2] [4 5]]");
}

// ── 4-arg ────────────────────────────────────────────────────────────────────

#[test]
fn ir_partition_4arg_pads_last_chunk() {
    // The exact example from the issue
    assert_partition("(partition 2 2 (repeat nil) [1 2 3])", "[[1 2] [3 nil]]");
}

#[test]
fn ir_partition_4arg_no_padding_needed() {
    assert_partition("(partition 2 2 [99 99] [1 2 3 4])", "[[1 2] [3 4]]");
}
