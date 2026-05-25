//! Phase B integration tests: `^:async` dispatch, `eval_async`, and `await`.

use std::sync::Arc;

use cljrs_async::eval_async::eval_async;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

/// Build a standard environment with the async runtime registered.
fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_interp::standard_env(None, None, None);
    cljrs_async::init(&globals);
    globals
}

/// Build a standard environment *without* an async runtime.
fn sync_env() -> Arc<GlobalEnv> {
    cljrs_interp::standard_env(None, None, None)
}

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("no form")
}

/// Synchronously evaluate every form in `src`, returning the last value.
fn eval_sync(src: &str, env: &mut Env) -> Value {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in p.parse_all().expect("parse error") {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

/// Run a `!Send` future to completion on a current-thread Tokio LocalSet.
fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

#[test]
fn async_fn_call_returns_future_immediately() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn ^:async dbl [x] (* x 2))", &mut env);
        // The call returns a Future, not the computed Long, even though the
        // body produces a Long synchronously.
        let v = cljrs_interp::eval::eval(&parse_one("(dbl 21)"), &mut env).unwrap();
        assert!(matches!(v, Value::Future(_)), "expected Future, got {v:?}");
    });
}

#[test]
fn await_resolves_async_fn_result() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn ^:async dbl [x] (* x 2))", &mut env);
        let r = eval_async(&parse_one("(await (dbl 21))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(42));
    });
}

#[test]
fn await_inside_let_and_nested_async_calls() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn ^:async inc-async [x] (+ x 1))", &mut env);
        eval_sync(
            "(defn ^:async add-both [a b]
               (let [x (await (inc-async a))
                     y (await (inc-async b))]
                 (+ x y)))",
            &mut env,
        );
        let r = eval_async(&parse_one("(await (add-both 10 20))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(32));
    });
}

#[test]
fn anonymous_fn_async_metadata_is_detected() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        let r = eval_async(
            &parse_one("(let [f (fn ^:async [x] (* x 2))] (await (f 5)))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(r, Value::Long(10));
    });
}

#[test]
fn await_in_if_branch_yields() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(
            "(defn ^:async pick [x] (if (pos? x) (await (id-async x)) 0))",
            &mut env,
        );
        eval_sync("(defn ^:async id-async [x] x)", &mut env);
        let r = eval_async(&parse_one("(await (pick 7))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(7));
        let r0 = eval_async(&parse_one("(await (pick -1))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r0, Value::Long(0));
    });
}

#[test]
fn awaiting_failed_async_fn_propagates_error() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(
            "(defn ^:async boom [] (throw (ex-info \"nope\" {})))",
            &mut env,
        );
        let r = eval_async(&parse_one("(await (boom))"), &mut env).await;
        assert!(r.is_err(), "expected error, got {r:?}");
    });
}

#[test]
fn without_runtime_async_fn_runs_synchronously() {
    // No async runtime registered: `^:async` is inert and the call runs inline,
    // returning the computed value rather than a Future.
    let globals = sync_env();
    let mut env = Env::new(globals, "user");
    eval_sync("(defn ^:async dbl [x] (* x 2))", &mut env);
    let v = eval_sync("(dbl 21)", &mut env);
    assert_eq!(v, Value::Long(42));
}

#[test]
fn defn_attr_map_marks_async() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn dbl {:async true} [x] (* x 2))", &mut env);
        let v = cljrs_interp::eval::eval(&parse_one("(dbl 21)"), &mut env).unwrap();
        assert!(matches!(v, Value::Future(_)), "expected Future, got {v:?}");
    });
}
