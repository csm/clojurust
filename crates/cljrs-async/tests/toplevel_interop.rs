//! `(.method target)` at the top level, under the async evaluator.
//!
//! `session::eval_form` routes every top-level form through
//! `eval_async` whenever the async driver is installed, which is the default
//! for the CLI. `eval_call_async` evaluated its head with `eval` before
//! checking for the interop sugar, and `.toUpperCase` is not a symbol that
//! resolves to anything — so a bare `(.method x)` failed with
//! `Unable to resolve symbol: .toUpperCase` while the identical form inside a
//! `defn` worked, because a function body reaches the sync evaluator or the
//! IR tier instead.
//!
//! These drive the async path directly, which is where the divergence was.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn globals() -> Arc<GlobalEnv> {
    cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals()
}

/// Evaluate `src` the way the CLI does: every top-level form through
/// `eval_async`, on a `LocalSet`.
fn eval_async_src(src: &str) -> Result<Value, String> {
    let globals = globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        let mut result = Value::Nil;
        for form in &forms {
            result = cljrs_async::eval_async::eval_async(form, &mut env)
                .await
                .map_err(|e| format!("{e:?}"))?;
        }
        Ok(result)
    })
}

fn value_of(src: &str) -> Value {
    eval_async_src(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

#[test]
fn a_top_level_method_call_dispatches() {
    assert_eq!(
        value_of(r#"(.toUpperCase "ab")"#),
        Value::string("AB".to_string())
    );
}

#[test]
fn a_top_level_method_call_passes_its_arguments() {
    assert_eq!(
        value_of(r#"(.substring "hello" 1 3)"#),
        Value::string("el".to_string())
    );
}

#[test]
fn a_top_level_field_read_dispatches() {
    assert_eq!(
        value_of("(do (deftype T [x]) (.-x (->T 7)))"),
        Value::Long(7)
    );
}

#[test]
fn the_target_is_evaluated_not_taken_literally() {
    assert_eq!(
        value_of(r#"(let [s "ab"] (.toUpperCase s))"#),
        Value::string("AB".to_string())
    );
}

#[test]
fn a_method_call_with_no_target_says_so() {
    let err = eval_async_src("(.toUpperCase)").expect_err("no target");
    assert!(
        err.contains("requires a target object"),
        "unhelpful error: {err}"
    );
}

#[test]
fn an_unknown_method_reports_the_type_not_the_symbol() {
    // The old failure mode was `Unable to resolve symbol: .nope`, which points
    // at the head rather than at the target that cannot answer it.
    let err = eval_async_src(r#"(.nope "ab")"#).expect_err("unknown method");
    assert!(
        err.contains("not supported") && !err.contains("resolve symbol"),
        "unhelpful error: {err}"
    );
}
