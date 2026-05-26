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

// ── Phase F: join-all, async-pmap, thread, onto-chan!, to-chan!, mult ─────────

const REQUIRE_F: &str = "(require '[clojure.core.async :refer \
    [chan take! put! close! poll! go join-all async-pmap thread onto-chan! to-chan! \
     merge reduce into mult tap! untap! untap-all!]])";

#[test]
fn join_all_awaits_all_futures() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(defn ^:async plus1 [x] (+ x 1))", &mut env);
        let r = eval_async(
            &parse_one("(await (join-all [(plus1 1) (plus1 2) (plus1 3)]))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(pr(&r), "[2 3 4]");
    });
}

#[test]
fn join_all_empty_seq_returns_empty_vector() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        let r = eval_async(&parse_one("(await (join-all []))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&r), "[]");
    });
}

#[test]
fn async_pmap_maps_fn_over_coll() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(defn ^:async double [x] (* x 2))", &mut env);
        let r = eval_async(
            &parse_one("(await (async-pmap double [1 2 3 4]))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(pr(&r), "[2 4 6 8]");
    });
}

#[test]
fn thread_macro_puts_result_on_channel() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def result-ch (thread (+ 6 7)))", &mut env);
        let r = eval_async(&parse_one("(await (take! result-ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(13));
    });
}

#[test]
fn onto_chan_seeds_and_closes_channel() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def ch (chan 5))", &mut env);
        eval_async(&parse_one("(await (onto-chan! ch [10 20 30]))"), &mut env)
            .await
            .unwrap();
        let a = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let b = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let c = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let d = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(a, Value::Long(10));
        assert_eq!(b, Value::Long(20));
        assert_eq!(c, Value::Long(30));
        assert_eq!(d, Value::Nil, "closed channel should return nil");
    });
}

#[test]
fn to_chan_creates_seeded_closed_channel() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def ch (to-chan! [:a :b :c]))", &mut env);
        let a = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let b = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let c = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        let d = eval_async(&parse_one("(await (take! ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&a), ":a");
        assert_eq!(pr(&b), ":b");
        assert_eq!(pr(&c), ":c");
        assert_eq!(d, Value::Nil, "closed channel should return nil");
    });
}

#[test]
fn merge_combines_two_channels() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync(
            "(def out (merge [(to-chan! [1 2]) (to-chan! [3 4])]))",
            &mut env,
        );
        // Drain all four values; order is unspecified so sort.
        let r = eval_async(&parse_one("(await (into [] out))"), &mut env)
            .await
            .unwrap();
        let mut vals: Vec<i64> = match &r {
            Value::Vector(v) => v
                .get()
                .iter()
                .filter_map(|x| {
                    if let Value::Long(n) = x {
                        Some(*n)
                    } else {
                        None
                    }
                })
                .collect(),
            _ => panic!("expected vector, got {r:?}"),
        };
        vals.sort();
        assert_eq!(vals, vec![1, 2, 3, 4]);
    });
}

#[test]
fn async_reduce_folds_over_channel() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        // reduce + on a channel of [1 2 3 4 5].
        let r = eval_async(
            &parse_one("(await (reduce + 0 (to-chan! [1 2 3 4 5])))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(r, Value::Long(15));
    });
}

#[test]
fn async_into_drains_channel_to_vector() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        let r = eval_async(
            &parse_one("(await (into [] (to-chan! [10 20 30])))"),
            &mut env,
        )
        .await
        .unwrap();
        assert_eq!(pr(&r), "[10 20 30]");
    });
}

#[test]
fn mult_broadcasts_to_two_taps() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def src (chan 4))", &mut env);
        eval_sync("(def m   (mult src))", &mut env);
        eval_sync("(def t1  (chan 4))", &mut env);
        eval_sync("(def t2  (chan 4))", &mut env);
        eval_sync("(tap! m t1)", &mut env);
        eval_sync("(tap! m t2)", &mut env);
        eval_async(&parse_one("(await (put! src :ping))"), &mut env)
            .await
            .unwrap();
        let v1 = eval_async(&parse_one("(await (take! t1))"), &mut env)
            .await
            .unwrap();
        let v2 = eval_async(&parse_one("(await (take! t2))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&v1), ":ping");
        assert_eq!(pr(&v2), ":ping");
    });
}

#[test]
fn mult_closes_taps_when_source_closes() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def src (chan 1))", &mut env);
        eval_sync("(def m   (mult src))", &mut env);
        eval_sync("(def tap (chan 4))", &mut env);
        eval_sync("(tap! m tap)", &mut env);
        eval_sync("(close! src)", &mut env);
        // The mult background loop should close `tap` when source closes.
        let v = eval_async(&parse_one("(await (take! tap))"), &mut env)
            .await
            .unwrap();
        assert_eq!(v, Value::Nil, "tap should be closed when source closes");
    });
}

#[test]
fn untap_removes_a_tap() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        eval_sync("(def src (chan 4))", &mut env);
        eval_sync("(def m   (mult src))", &mut env);
        eval_sync("(def t1  (chan 4))", &mut env);
        eval_sync("(def t2  (chan 4))", &mut env);
        eval_sync("(tap! m t1)", &mut env);
        eval_sync("(tap! m t2)", &mut env);
        eval_sync("(untap! m t1)", &mut env);
        // t1 has been removed; only t2 should receive the value.
        eval_async(&parse_one("(await (put! src :hello))"), &mut env)
            .await
            .unwrap();
        let v2 = eval_async(&parse_one("(await (take! t2))"), &mut env)
            .await
            .unwrap();
        assert_eq!(pr(&v2), ":hello");
        // t1 should be empty (offer! is non-blocking; t1 has nothing).
        let t1_empty = eval_sync("(poll! t1)", &mut env);
        assert_eq!(t1_empty, Value::Nil);
    });
}

#[test]
fn loop_with_await_inside_async_fn() {
    let globals = async_env();
    block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_sync(REQUIRE_F, &mut env);
        // Verify that loop/recur inside an ^:async fn correctly yields at await.
        eval_sync(
            "(defn ^:async sum-ch [ch]
               (loop [acc 0]
                 (let [v (await (take! ch))]
                   (if (nil? v) acc (recur (+ acc v))))))",
            &mut env,
        );
        eval_sync("(def ch (to-chan! [1 2 3 4 5]))", &mut env);
        let r = eval_async(&parse_one("(await (sum-ch ch))"), &mut env)
            .await
            .unwrap();
        assert_eq!(r, Value::Long(15));
    });
}
