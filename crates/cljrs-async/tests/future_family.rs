//! `future` — a task on this isolate's executor.
//!
//! Cooperative, not parallel: every Clojure value is `!Send`, so the body runs
//! on the same thread as its caller and advances only while that thread is
//! awaiting. The tests below pin exactly that contract, including the parts
//! that differ from the JVM.

use std::sync::Arc;

use cljrs_async::eval_async::eval_async;
use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals
}

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// Evaluate every form through the async tree-walker, returning the last value.
async fn eval_all(src: &str, env: &mut Env) -> Value {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in p.parse_all().expect("parse error") {
        result = eval_async(&form, env).await.expect("eval error");
    }
    result
}

async fn eval_err(src: &str, env: &mut Env) -> String {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut last = Err(String::new());
    for form in p.parse_all().expect("parse error") {
        last = eval_async(&form, env).await.map_err(|e| format!("{e:?}"));
        if last.is_err() {
            break;
        }
    }
    last.expect_err("expected an error")
}

fn run(src: &str) -> String {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        format!("{}", eval_all(src, &mut env).await)
    })
}

#[test]
fn future_returns_a_future_and_await_yields_its_value() {
    assert_eq!(run("(future? (future 41))"), "true");
    assert_eq!(run("(await (future (+ 1 41)))"), "42");
}

#[test]
fn the_body_does_not_run_before_the_caller_yields() {
    // The JVM would have another thread run this immediately; here the task
    // only advances once the caller awaits.
    assert_eq!(
        run(
            "(def log (atom [])) (let [f (future (swap! log conj :ran) :v)] [@log (await f) @log])"
        ),
        "[[] :v [:ran]]"
    );
}

#[test]
fn future_done_tracks_the_task() {
    assert_eq!(
        run("(let [f (future 1)] [(future-done? f) (await f) (future-done? f)])"),
        "[false 1 true]"
    );
}

#[test]
fn realized_answers_for_a_future() {
    assert_eq!(
        run("(let [f (future 1)] [(realized? f) (await f) (realized? f)])"),
        "[false 1 true]"
    );
}

#[test]
fn cancelling_a_running_future_makes_await_raise() {
    assert_eq!(
        run("(let [f (future :late)] \
             [(future-cancel f) (future-cancelled? f) (future-done? f) \
              (try (await f) (catch Exception e :raised))])"),
        "[true true true :raised]"
    );
}

#[test]
fn cancelling_a_settled_future_reports_false() {
    assert_eq!(
        run("(let [f (future 1)] (await f) [(future-cancel f) (future-cancelled? f)])"),
        "[false false]"
    );
}

#[test]
fn a_throwing_body_propagates_through_await() {
    assert_eq!(
        run(
            "(try (await (future (throw (ex-info \"boom\" {})))) (catch Exception e (ex-message e)))"
        ),
        "\"boom\""
    );
}

#[test]
fn futures_interleave_with_each_other() {
    // Two tasks, both outstanding, both resolved by awaiting in turn.
    assert_eq!(
        run("(let [a (future 1) b (future 2)] [(await a) (await b)])"),
        "[1 2]"
    );
}

#[test]
fn the_future_predicates_reject_other_values() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        for src in [
            "(future-done? 1)",
            "(future-cancel :x)",
            "(future-cancelled? [])",
        ] {
            let err = eval_err(src, &mut env).await;
            assert!(
                err.contains("future"),
                "{src} should report a type error naming future, got: {err}"
            );
        }
        assert_eq!(
            format!("{}", eval_all("(future? 1)", &mut env).await),
            "false"
        );
    });
}
