//! Tests for the `<?` error-propagation family and the `clojure.rust.error`
//! helpers (`error?`, `ok?`, `throw-err`).
//!
//! The model: failures ride *in band* as error values on channels; `<?`/`<??`
//! take a value and throw it if it is an error (otherwise return it), and
//! `go-try` is the boundary that catches a throw and re-delivers it as an
//! in-band error on its result channel — the CSP analogue of Rust's `?`.

use std::sync::Arc;

use cljrs_async::eval_async::eval_async;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_reader::Parser;
use cljrs_value::Value;

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_interp::standard_env(None, None, None);
    cljrs_async::init(&globals);
    globals
}

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("no form")
}

fn eval_sync(src: &str, env: &mut Env) -> Value {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in p.parse_all().expect("parse error") {
        result = cljrs_interp::eval::eval(&form, env).expect("eval error");
    }
    result
}

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// Require core.async (with the `<?` family) and the error helpers.
const REQ: &str = "(require '[clojure.core.async :refer [chan offer! take! put! close! <!! <? <?? go go-try]]) \
                   (require '[clojure.rust.error :refer [error? ok? throw-err]])";

#[test]
fn error_predicates_and_throw_err() {
    let globals = async_env();
    let mut env = Env::new(globals, "user");
    eval_sync(REQ, &mut env);
    // An error value (caught exception) vs ordinary values.
    eval_sync(
        "(def e (try (throw (ex-info \"x\" {})) (catch Exception ex ex)))",
        &mut env,
    );
    assert_eq!(eval_sync("(error? e)", &mut env), Value::Bool(true));
    assert_eq!(eval_sync("(error? 5)", &mut env), Value::Bool(false));
    assert_eq!(eval_sync("(error? nil)", &mut env), Value::Bool(false));
    assert_eq!(eval_sync("(ok? 5)", &mut env), Value::Bool(true));
    assert_eq!(eval_sync("(ok? nil)", &mut env), Value::Bool(false));
    assert_eq!(eval_sync("(ok? e)", &mut env), Value::Bool(false));
    // throw-err passes non-errors through untouched.
    assert_eq!(eval_sync("(throw-err 5)", &mut env), Value::Long(5));
    // throw-err throws an error value, catchable with the message intact.
    assert_eq!(
        eval_sync(
            "(= \"x\" (try (throw-err e) (catch Exception ex (ex-message ex))))",
            &mut env
        ),
        Value::Bool(true)
    );
}

#[test]
fn blocking_take_returns_value_or_throws() {
    let globals = async_env();
    let mut env = Env::new(globals, "user");
    eval_sync(REQ, &mut env);
    eval_sync("(def ch (chan 2))", &mut env);
    // A normal value comes back unchanged.
    eval_sync("(offer! ch 42)", &mut env);
    assert_eq!(eval_sync("(<?? ch)", &mut env), Value::Long(42));
    // An in-band error value is thrown (and is catchable with its message).
    eval_sync(
        "(offer! ch (try (throw (ex-info \"boom\" {})) (catch Exception e e)))",
        &mut env,
    );
    assert_eq!(
        eval_sync(
            "(= \"boom\" (try (<?? ch) (catch Exception e (ex-message e))))",
            &mut env
        ),
        Value::Bool(true)
    );
}

#[test]
fn async_take_returns_value() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQ, &mut env);
        eval_sync("(def out (chan 1))", &mut env);
        eval_sync("(offer! out 21)", &mut env);
        let r = eval_async(&parse_one("(<? out)"), &mut env).await.unwrap();
        assert_eq!(r, Value::Long(21));
    });
}

#[test]
fn go_try_delivers_result_then_propagates_via_take() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQ, &mut env);
        // Success: go-try's result channel yields the body value; <? returns it.
        eval_sync("(defn ^:async ok-run [] (<? (go-try 42)))", &mut env);
        let r = eval_async(&parse_one("(await (ok-run))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(42));

        // A nil body just closes the channel; <? sees nil (not an error).
        eval_sync("(defn ^:async nil-run [] (<? (go-try nil)))", &mut env);
        let r = eval_async(&parse_one("(await (nil-run))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Nil);
    });
}

#[test]
fn go_try_catches_throw_and_propagates_as_error() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQ, &mut env);
        // A throw inside go-try is caught and re-delivered in band; <? re-throws
        // it, so awaiting the fn surfaces an error carrying the message.
        eval_sync(
            "(defn ^:async err-run [] (<? (go-try (throw (ex-info \"boom\" {})))))",
            &mut env,
        );
        let r = eval_async(&parse_one("(await (err-run))"), &mut env).await;
        assert!(r.is_err(), "expected an error, got {r:?}");
        let msg = format!("{:?}", r.unwrap_err());
        assert!(
            msg.contains("boom"),
            "error should carry the message: {msg}"
        );
    });
}

#[test]
fn awaited_throw_preserves_ex_data() {
    // Fidelity: a throw that crosses an `await` boundary must keep its ex-data
    // and ex-message (previously the error was stringified into a bare message,
    // losing both). We await a throwing ^:async fn at top level and inspect the
    // resulting error value directly — note we do NOT wrap the await in a
    // try/catch, because eval_async delegates try to the synchronous evaluator,
    // whose blocking await would deadlock the single LocalSet thread (the
    // documented async try/catch limitation).
    use cljrs_value::ValueError;

    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQ, &mut env);
        eval_sync(
            "(defn ^:async boom [] (throw (ex-info \"nope\" {:code 42})))",
            &mut env,
        );
        let err = eval_async(&parse_one("(await (boom))"), &mut env)
            .await
            .expect_err("awaiting a throwing async fn should error");

        // The error must be a re-raised Thrown value (not a stringified Runtime
        // error), and that value must be the original Value::Error with its
        // ex-message and ex-data intact.
        let thrown = match err {
            cljrs_env::error::EvalError::Thrown(v) => v,
            other => panic!("expected EvalError::Thrown, got {other:?}"),
        };
        let exc = match &thrown {
            Value::Error(e) => e.get(),
            other => panic!("expected a Value::Error, got {other:?}"),
        };
        assert_eq!(exc.message(), "nope", "ex-message must survive");
        let data = exc.data().expect("ex-data must survive");
        // :code -> 42 in the preserved data map.
        let code = data
            .get(&Value::keyword(cljrs_value::Keyword::simple("code")))
            .expect("ex-data should contain :code");
        assert_eq!(code, Value::Long(42), "ex-data value must survive");
        // Sanity: the variant really is a thrown exception, not WrongType etc.
        assert!(matches!(
            ValueError::Thrown(thrown.clone()),
            ValueError::Thrown(_)
        ));
    });
}
