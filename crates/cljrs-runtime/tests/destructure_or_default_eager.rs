//! Issue #363 — an `:or` destructuring default is evaluated *eagerly*, in both
//! tiers.
//!
//! `destructure` expands a symbol carrying an `:or` entry to
//! `(get m :k default)`.  `get` is an ordinary function, so its third argument
//! is evaluated whether or not the key is present: on the JVM a side-effecting
//! default fires on every call, even one that supplies the key.
//!
//! The tree-walker does that (`interp/destructure.rs`); the IR lowerer used to
//! emit the default into the `then` arm of a nil-check branch, so it fired only
//! on a miss.  Since a function tiers up partway through a run — at a point set
//! by a background worker — that made the *number* of times a default fired
//! nondeterministic.  These tests run the same programs through both tiers and
//! demand the same answer, so the two cannot drift apart again.
//!
//! This file flips the process-wide eager-lowering switch, so it lives in its
//! own test binary; the tree-walk cases below are unaffected by it, since
//! `ExecutionMode::TreeWalk` never consults the IR path at all.

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
        // The type name, not `{:?}`: which type came back instead of a string
        // is what the assertion is about, and a `Value` may hold a secret.
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

/// Run `src` through the tree-walker and through the IR tier and assert both
/// produce `expected` — the tier split in #363 is exactly a disagreement here.
fn assert_both_tiers(src: &str, expected: &str) {
    assert_eq!(
        eval_pr(cljrs_runtime::ExecutionMode::TreeWalk, src),
        expected,
        "tree-walking interpreter"
    );
    assert_eq!(
        eval_pr(cljrs_runtime::ExecutionMode::TieredNoJit, src),
        expected,
        "IR tier"
    );
}

#[test]
fn a_present_key_still_evaluates_its_default() {
    // The discriminating case: the key is supplied on every call, so a lazy
    // lowering never runs the default and reports 0.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{:keys [x] :or {x (do (swap! counter inc) 1)}}] x)
         (dotimes [_ 200] (f {:x 99}))
         (pr-str @counter)",
        "200",
    );
}

#[test]
fn a_present_key_still_wins_over_its_default() {
    // Eager evaluation must not change which value is *bound*.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{:keys [x] :or {x (do (swap! counter inc) 1)}}] x)
         (pr-str (mapv f (repeat 200 {:x 99})))",
        &format!("[{}]", vec!["99"; 200].join(" ")),
    );
}

#[test]
fn a_missing_key_binds_the_default() {
    assert_both_tiers(
        "(defn f [{:keys [x] :or {x 7}}] x)
         (pr-str (mapv f (repeat 200 {})))",
        &format!("[{}]", vec!["7"; 200].join(" ")),
    );
}

#[test]
fn a_default_evaluates_once_per_call_not_once_per_lowering() {
    // A miss and a hit interleaved: 200 calls, 200 evaluations either way.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{:keys [x] :or {x (do (swap! counter inc) 1)}}] x)
         (dotimes [i 200] (f (if (even? i) {:x 99} {})))
         (pr-str @counter)",
        "200",
    );
}

#[test]
fn strs_and_syms_defaults_are_eager_too() {
    // `:strs` and `:syms` share `lower_with_default`; pin them alongside.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{:strs [x] :or {x (do (swap! counter inc) 1)}}] x)
         (dotimes [_ 200] (f {\"x\" 99}))
         (pr-str @counter)",
        "200",
    );
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{:syms [x] :or {x (do (swap! counter inc) 1)}}] x)
         (dotimes [_ 200] (f {'x 99}))
         (pr-str @counter)",
        "200",
    );
}

#[test]
fn an_explicit_binding_pairs_default_is_eager_too() {
    // `{a :x}` rather than `:keys` — the other `apply_default_if_nil` site.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [{a :x :or {a (do (swap! counter inc) 1)}}] a)
         (dotimes [_ 200] (f {:x 99}))
         (pr-str @counter)",
        "200",
    );
}

#[test]
fn a_let_destructuring_default_is_eager_too() {
    // The same lowering serves `let*`, reached here through a hot function.
    assert_both_tiers(
        "(def counter (atom 0))
         (defn f [m] (let [{:keys [x] :or {x (do (swap! counter inc) 1)}} m] x))
         (dotimes [_ 200] (f {:x 99}))
         (pr-str @counter)",
        "200",
    );
}
