//! Regression tests for issue #205: `partition` 3-arg and 4-arg arities.

use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_interp::standard_env(None, None, None);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `src` in a fresh env and return the result.
fn eval_str(src: &str) -> Value {
    let (_, mut env) = make_env();
    eval_in(src, &mut env)
}

fn eval_in(src: &str, env: &mut Env) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

/// Evaluate `expr`, materialise the result to a Clojure vector-of-vectors with
/// `(mapv vec ...)`, then compare the pr-str against `expected`.
fn assert_partition(expr: &str, expected: &str) {
    let normalised = eval_str(&format!("(pr-str (mapv vec {}))", expr));
    match normalised {
        Value::Str(s) => assert_eq!(
            s.get().as_str(),
            expected,
            "partition result mismatch for {}",
            expr
        ),
        other => panic!("pr-str returned non-string: {:?}", other),
    }
}

// ── 2-arg: (partition n coll) ────────────────────────────────────────────────

#[test]
fn partition_2arg_basic() {
    assert_partition("(partition 2 [1 2 3 4])", "[[1 2] [3 4]]");
}

#[test]
fn partition_2arg_drops_incomplete_tail() {
    assert_partition("(partition 2 [1 2 3])", "[[1 2]]");
}

#[test]
fn partition_2arg_empty() {
    assert_partition("(partition 2 [])", "[]");
}

// ── 3-arg: (partition n step coll) ──────────────────────────────────────────

#[test]
fn partition_3arg_overlapping() {
    // (partition 2 1 [1 2 3 4]) => ((1 2) (2 3) (3 4))
    assert_partition("(partition 2 1 [1 2 3 4])", "[[1 2] [2 3] [3 4]]");
}

#[test]
fn partition_3arg_step_equals_n() {
    // Same as 2-arg when step == n
    assert_partition("(partition 2 2 [1 2 3 4])", "[[1 2] [3 4]]");
}

#[test]
fn partition_3arg_skipping() {
    // (partition 2 3 [1 2 3 4 5 6]) => ((1 2) (4 5))
    assert_partition("(partition 2 3 [1 2 3 4 5 6])", "[[1 2] [4 5]]");
}

#[test]
fn partition_3arg_drops_incomplete_tail() {
    // (partition 3 1 [1 2 3 4]) => ((1 2 3) (2 3 4))
    assert_partition("(partition 3 1 [1 2 3 4])", "[[1 2 3] [2 3 4]]");
}

// ── 4-arg: (partition n step pad coll) ──────────────────────────────────────

#[test]
fn partition_4arg_pads_last_chunk() {
    // (partition 2 2 [99] [1 2 3]) => ((1 2) (3 99))
    assert_partition("(partition 2 2 [99] [1 2 3])", "[[1 2] [3 99]]");
}

#[test]
fn partition_4arg_repeat_nil_pad() {
    // The example from the issue: (partition 2 2 (repeat nil) [1 2 3])
    assert_partition(
        "(partition 2 2 (repeat nil) [1 2 3])",
        "[[1 2] [3 nil]]",
    );
}

#[test]
fn partition_4arg_no_padding_needed() {
    // Even-length coll: pad never used
    assert_partition("(partition 2 2 [99 99] [1 2 3 4])", "[[1 2] [3 4]]");
}

#[test]
fn partition_4arg_overlapping_with_pad() {
    // (partition 3 2 [0 0] [1 2 3 4 5])
    // start=0: [1 2 3] full; start=2: [3 4 5] full; start=4: [5] + [0 0] => [5 0 0]
    assert_partition(
        "(partition 3 2 [0 0] [1 2 3 4 5])",
        "[[1 2 3] [3 4 5] [5 0 0]]",
    );
}
