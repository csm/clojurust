//! Clojure-level tests for the cross-isolate channel surface (Phase B2):
//! `isolate-chan`, `isolate-put!`, `isolate-poll!`, `isolate-take!`.

use std::sync::Arc;

use cljrs_async::eval_async::eval_async;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_env::error::EvalError;
use cljrs_reader::Parser;
use cljrs_value::Value;

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_interp::standard_env(None, None, None);
    cljrs_async::init(&globals);
    globals
}

const REQUIRE_ISOLATE: &str = "(require '[clojure.core.async :refer [isolate-chan isolate-put! isolate-poll! isolate-take!]])";

/// Build a `user` env with the isolate-channel builtins referred in.
fn user_env(globals: Arc<GlobalEnv>) -> Env {
    let mut env = Env::new(globals, "user");
    eval_sync(REQUIRE_ISOLATE, &mut env);
    env
}

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("no form")
}

/// Evaluate every form in `src`, returning the last value (panicking on error).
fn eval_sync(src: &str, env: &mut Env) -> Value {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in p.parse_all().expect("parse error") {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

/// Evaluate a single form, returning the `Result` so error paths can be tested.
#[allow(clippy::result_large_err)] // mirrors cljrs_interp::eval::eval's own signature
fn try_eval(src: &str, env: &mut Env) -> Result<Value, EvalError> {
    cljrs_interp::eval::eval(&parse_one(src), env)
}

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

#[test]
fn isolate_chan_returns_tx_rx_pair() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        let v = eval_sync("(count (isolate-chan))", &mut env);
        assert_eq!(v, Value::Long(2));
    });
}

#[test]
fn put_then_poll_roundtrips_a_value() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        let v = eval_sync(
            "(let [[tx rx] (isolate-chan)]
               (isolate-put! tx {:a 1 :b [2 3 \"four\"]})
               (isolate-poll! rx))",
            &mut env,
        );
        let expected = eval_sync("{:a 1 :b [2 3 \"four\"]}", &mut env);
        assert_eq!(v, expected);
    });
}

#[test]
fn put_returns_true_poll_empty_returns_nil() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        assert_eq!(
            eval_sync(
                "(let [[tx _] (isolate-chan)] (isolate-put! tx 42))",
                &mut env
            ),
            Value::Bool(true)
        );
        assert_eq!(
            eval_sync("(let [[_ rx] (isolate-chan)] (isolate-poll! rx))", &mut env),
            Value::Nil
        );
    });
}

#[test]
fn fifo_order_across_multiple_puts() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        let v = eval_sync(
            "(let [[tx rx] (isolate-chan)]
               (isolate-put! tx 1)
               (isolate-put! tx 2)
               (isolate-put! tx 3)
               [(isolate-poll! rx) (isolate-poll! rx) (isolate-poll! rx) (isolate-poll! rx)])",
            &mut env,
        );
        let expected = eval_sync("[1 2 3 nil]", &mut env);
        assert_eq!(v, expected);
    });
}

#[test]
fn non_shareable_value_is_located_error_at_put_site() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        let result = try_eval(
            "(let [[tx _] (isolate-chan)] (isolate-put! tx (atom 1)))",
            &mut env,
        );
        let err = result.expect_err("expected a located CloneError at the put site");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("isolate-put!") && msg.contains("isolate boundary"),
            "error should be located at the put site and mention the boundary, got: {msg}"
        );
    });
}

#[test]
fn isolate_take_resolves_a_put_value() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = user_env(globals);
        // Stash the channel ends in vars so both forms see the same channel.
        eval_sync("(def ch (isolate-chan))", &mut env);
        eval_sync("(isolate-put! (first ch) [:hello 99])", &mut env);
        let r = eval_async(&parse_one("(await (isolate-take! (second ch)))"), &mut env)
            .await
            .unwrap();
        let expected = eval_sync("[:hello 99]", &mut env);
        assert_eq!(r, expected);
    });
}
