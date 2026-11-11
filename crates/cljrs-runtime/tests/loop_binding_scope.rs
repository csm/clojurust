//! Issue #477 — `loop` binds like `let`, and `recur` rebinds in parallel.
//!
//! On initial entry each initializer must see the loop-locals bound before it,
//! not a shadowed outer binding: `(let [a 40] (loop [a 2 b (inc a)] [a b]))` is
//! `[2 3]`.  The tree-walker, the async evaluator and IR lowering all used to
//! evaluate every initializer against the enclosing scope (`[2 41]`).
//!
//! On `recur` every argument is evaluated against the current iteration before
//! any loop-local is rebound, so `(recur b a)` swaps.  The Tier-1 IR
//! interpreter used to resolve a loop header's phis one at a time, so a swap
//! read the first phi's new value (`[3 3]`).
//!
//! Each program runs through the tree-walker and the IR tier.  This file flips
//! the process-wide eager-lowering switch, so it lives in its own test binary.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env(mode: cljrs_runtime::ExecutionMode) -> (Arc<GlobalEnv>, Env) {
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

fn eval_pr(mode: cljrs_runtime::ExecutionMode, src: &str) -> String {
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

/// Run `body` directly in the tree-walker, and inside a function called often
/// enough to be lowered, in the IR tier; both must produce `expected`.
fn assert_both_tiers(body: &str, expected: &str) {
    assert_eq!(
        eval_pr(
            cljrs_runtime::ExecutionMode::TreeWalk,
            &format!("(pr-str {body})")
        ),
        expected,
        "tree-walking interpreter"
    );
    let tiered = format!(
        "(defn f [] {body})
         (dotimes [_ 50] (f))
         (pr-str (f))"
    );
    assert_eq!(
        eval_pr(cljrs_runtime::ExecutionMode::TieredNoJit, &tiered),
        expected,
        "IR tier"
    );
}

#[test]
fn later_initializer_sees_earlier_loop_local() {
    assert_both_tiers("(let [a 40] (loop [a 2 b (inc a)] [a b]))", "[2 3]");
}

#[test]
fn later_initializer_sees_loop_local_without_outer_binding() {
    assert_both_tiers("(loop [loop-a 2 b loop-a] b)", "2");
}

#[test]
fn loop_star_binds_sequentially_too() {
    assert_both_tiers("(let [a 40] (loop* [a 2 b (inc a)] [a b]))", "[2 3]");
}

#[test]
fn destructured_pattern_is_visible_to_later_initializer() {
    assert_both_tiers("(let [a 40] (loop [[a] [2] b (inc a)] [a b]))", "[2 3]");
}

#[test]
fn closure_in_later_initializer_captures_loop_local() {
    assert_both_tiers("(let [a 40] (loop [a 2 f (fn [] a)] (f)))", "2");
}

#[test]
fn recur_rebinds_in_parallel() {
    // The issue's regression test: entry binds `b` to 3, the first `recur`
    // swaps `a` and `b`.
    assert_both_tiers(
        "(let [a 40]
           (loop [a 2 b (inc a) swap? true]
             (if swap?
               (recur b a false)
               [a b])))",
        "[3 2]",
    );
}

#[test]
fn recur_rotates_three_locals() {
    assert_both_tiers(
        "(loop [a 1 b 2 c 3 n 2]
           (if (pos? n)
             (recur b c a (dec n))
             [a b c]))",
        "[3 1 2]",
    );
}
