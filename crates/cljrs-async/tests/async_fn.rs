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
/// Timers are enabled so `timeout` works.
fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// Print a value the way Clojure would (for asserting on collection results).
fn pr(v: &Value) -> String {
    format!("{v}")
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

// ── Phase C: deref enforcement in async context ────────────────────────────

#[test]
fn deref_of_future_in_async_fn_errors() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn ^:async producer [] 42)", &mut env);
        eval_sync("(defn ^:async bad [] (deref (producer)))", &mut env);
        let r = eval_async(&parse_one("(await (bad))"), &mut env).await;
        let err = format!("{:?}", r.unwrap_err());
        assert!(err.contains("await"), "error should steer to await: {err}");
    });
}

#[test]
fn at_deref_of_future_in_async_fn_errors() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync("(defn ^:async producer [] 42)", &mut env);
        eval_sync("(defn ^:async bad [] @(producer))", &mut env);
        let r = eval_async(&parse_one("(await (bad))"), &mut env).await;
        let err = format!("{:?}", r.unwrap_err());
        assert!(err.contains("await"), "error should steer to await: {err}");
    });
}

#[test]
fn deref_of_future_in_sync_context_still_works() {
    // With the async runtime registered, a *sync* (non-^:async) deref of a
    // thread-based future must still block-and-return, not error.
    let globals = async_env();
    let mut env = Env::new(globals, "user");
    assert_eq!(
        eval_sync("(deref (future (+ 1 2)))", &mut env),
        Value::Long(3)
    );
    assert_eq!(eval_sync("@(future (* 6 7))", &mut env), Value::Long(42));
}

// ── Phase D: timeout, alts, alt ────────────────────────────────────────────

const REQUIRE_ASYNC: &str = "(require '[clojure.core.async :refer [timeout alts alt]])";

#[test]
fn timeout_resolves_to_nil() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_ASYNC, &mut env);
        let r = eval_async(&parse_one("(await (timeout 5))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Nil);
    });
}

#[test]
fn alts_picks_first_ready_value_and_index() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_ASYNC, &mut env);
        eval_sync("(defn ^:async producer [] 42)", &mut env);
        // The immediate producer resolves before the 1s timeout: index 0.
        let val = eval_async(
            &parse_one("(first (await (alts [(producer) (timeout 1000)])))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(val, Value::Long(42));
        let idx = eval_async(
            &parse_one("(second (await (alts [(producer) (timeout 1000)])))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(idx, Value::Long(0));
    });
}

#[test]
fn alts_selects_timeout_when_it_wins() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_ASYNC, &mut env);
        // Only a timeout in the set: it must win at index 0 with value nil.
        let r = eval_async(&parse_one("(await (alts [(timeout 5)]))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&r), "[nil 0]");
    });
}

#[test]
fn alt_dispatches_to_matching_handler() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_ASYNC, &mut env);
        eval_sync("(defn ^:async producer [] :got)", &mut env);
        eval_sync(
            "(defn ^:async runner []
               (alt (producer)     (fn [v] [:value v])
                    (timeout 1000)  (fn [_] [:timed-out])))",
            &mut env,
        );
        let r = eval_async(&parse_one("(await (runner))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&r), "[:value :got]");
    });
}

// ── Phase E: channels (chan, take!, put!, close!, poll!, offer!, go) ─────────

const REQUIRE_CHAN: &str =
    "(require '[clojure.core.async :refer [chan take! put! close! poll! offer! go]])";

#[test]
fn buffered_chan_put_then_take() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_CHAN, &mut env);
        eval_sync("(def ch (chan 4))", &mut env);
        let put = eval_async(&parse_one("(await (put! ch 42))"), &mut env)
            .await
            .unwrap();
        assert_eq!(put, Value::Bool(true));
        let got = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(got, Value::Long(42));
    });
}

#[test]
fn take_on_closed_channel_yields_nil() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_CHAN, &mut env);
        eval_sync("(def ch (chan 1))", &mut env);
        // A buffered value remains takeable after close, then nil.
        eval_async(&parse_one("(await (put! ch :only))"), &mut env)
            .await
            .unwrap();
        eval_sync("(close! ch)", &mut env);
        let first = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&first), ":only");
        let second = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(second, Value::Nil);
    });
}

#[test]
fn put_on_closed_channel_is_false() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_CHAN, &mut env);
        eval_sync("(def ch (chan 1))", &mut env);
        eval_sync("(close! ch)", &mut env);
        let put = eval_async(&parse_one("(await (put! ch 1))"), &mut env)
            .await
            .unwrap();
        assert_eq!(put, Value::Bool(false));
    });
}

#[test]
fn poll_and_offer_are_nonblocking() {
    let globals = async_env();
    let mut env = Env::new(globals, "user");
    eval_sync(REQUIRE_CHAN, &mut env);
    eval_sync("(def ch (chan 1))", &mut env);
    // Buffer has room: offer succeeds; then it is full.
    assert_eq!(eval_sync("(offer! ch 1)", &mut env), Value::Bool(true));
    assert_eq!(eval_sync("(offer! ch 2)", &mut env), Value::Bool(false));
    // poll drains the one value, then returns nil on empty.
    assert_eq!(eval_sync("(poll! ch)", &mut env), Value::Long(1));
    assert_eq!(eval_sync("(poll! ch)", &mut env), Value::Nil);
}

#[test]
fn go_block_passes_value_through_channels() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_CHAN, &mut env);
        eval_sync("(def in (chan 1))", &mut env);
        eval_sync("(def out (chan 1))", &mut env);
        // A go block takes from `in`, doubles it, and puts it on `out`.
        eval_sync(
            "(go (let [v (await (take! in))] (await (put! out (* v 2)))))",
            &mut env,
        );
        eval_async(&parse_one("(await (put! in 21))"), &mut env)
            .await
            .unwrap();
        let r = eval_async(&parse_one("(await (take! out))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(42));
    });
}

#[test]
fn rendezvous_chan_hands_off_to_taker() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_CHAN, &mut env);
        eval_sync("(def ch (chan))", &mut env); // unbuffered (rendezvous)
        eval_sync("(def out (chan 1))", &mut env);
        // A consumer go block relays the rendezvous value to `out`.
        eval_sync("(go (await (put! out (await (take! ch)))))", &mut env);
        // The put resolves true only once the consumer has taken the value.
        let put = eval_async(&parse_one("(await (put! ch :hello))"), &mut env)
            .await
            .unwrap();
        assert_eq!(put, Value::Bool(true));
        let r = eval_async(&parse_one("(await (take! out))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&r), ":hello");
    });
}

#[test]
fn chan_is_a_channel_native_object() {
    let globals = async_env();
    let mut env = Env::new(globals, "user");
    eval_sync(REQUIRE_CHAN, &mut env);
    let ch = eval_sync("(chan)", &mut env);
    assert!(
        matches!(ch, Value::NativeObject(ref o) if o.get().type_tag() == "Channel"),
        "expected a Channel native object, got {ch:?}"
    );
}
