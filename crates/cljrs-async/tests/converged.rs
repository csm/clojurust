//! Virtual-threads plan Phase 5: one async model on native targets.
//!
//! `go` bodies and `^:async` calls run on fibers with the ordinary evaluator.
//! `<!`, `>!`, `alts!`, `await`, and `deref` park the fiber, including inside
//! ordinary functions called from those bodies. Every case runs through the
//! tree-walker and the tiered interpreter, with collections forced while
//! bodies are parked.

#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use cljrs_async::fiber;
use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

const MODES: [cljrs_runtime::ExecutionMode; 2] = [
    cljrs_runtime::ExecutionMode::TreeWalk,
    cljrs_runtime::ExecutionMode::Tiered,
];

/// Evaluate `source` in a root fiber for each interpreter mode and return the
/// printed value of its last form per mode.
fn run_in_both_modes(source: &'static str) -> Vec<String> {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            MODES
                .iter()
                .map(|mode| {
                    let runtime = cljrs_runtime::Runtime::builder()
                        .execution_mode(*mode)
                        .build()
                        .unwrap();
                    let globals = runtime.globals().clone();
                    globals.intern(
                        "user",
                        "collect-now".into(),
                        Value::NativeFunction(cljrs_gc::GcPtr::new(cljrs_value::NativeFn::new(
                            "collect-now",
                            cljrs_value::Arity::Fixed(0),
                            |_| {
                                #[cfg(not(feature = "no-gc"))]
                                for _ in 0..2 {
                                    cljrs_gc::request_gc();
                                    cljrs_runtime::env::gc_roots::async_gc_collect();
                                }
                                Ok(Value::Nil)
                            },
                        ))),
                    );
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    let local = tokio::task::LocalSet::new();
                    local.block_on(&rt, async { cljrs_async::init(&globals) });
                    let mut carrier = MutatorState::fresh();
                    let result = fiber::run_root(&rt, &local, &mut carrier, move || {
                        let mut env = Env::new(globals, "user");
                        let forms = cljrs_reader::Parser::new(source.into(), "<phase5>".into())
                            .parse_all()
                            .unwrap();
                        let mut last = Value::Nil;
                        for form in &forms {
                            last = cljrs_runtime::tiered::eval(form, &mut env)?;
                        }
                        Ok(Value::string(last.to_string()))
                    });
                    fiber::drop_local_with_carrier(local, &mut carrier);
                    match result {
                        Ok(Value::Str(s)) => s.get().to_string(),
                        Ok(other) => panic!("{mode:?}: unexpected {other}"),
                        Err(error) => panic!("{mode:?}: {error}"),
                    }
                })
                .collect()
        })
        .unwrap()
        .join()
        .unwrap()
}

fn assert_both(source: &'static str, expected: &str) {
    for (mode, got) in MODES.iter().zip(run_in_both_modes(source)) {
        assert_eq!(got, expected, "{mode:?}");
    }
}

#[test]
fn go_bodies_park_on_channel_ops() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan go <! >! <!! >!!]])
        (def in (chan))
        (def out (chan))
        (go (let [v (<! in)]
              (collect-now)
              (>! out (* v 2))))
        (>!! in 21)
        (<!! out)
        "#,
        "42",
    );
}

#[test]
fn ordinary_functions_called_from_go_can_park() {
    // No ^:async, no await: the helper parks through a nested call.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan go <! >!! <!!]])
        (def ch (chan))
        (defn read-twice [c] [(<! c) (<! c)])
        (defn via-hof [c] (mapv (fn [_] (<! c)) [1 2]))
        (def result (go [(read-twice ch) (via-hof ch)]))
        (doseq [v [1 2 3 4]] (>!! ch v))
        (<!! result)
        "#,
        "[[1 2] [3 4]]",
    );
}

#[test]
fn go_returns_a_future_that_takes_and_awaits() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go <!!]])
        (def g (go (sleep 1) :done))
        [(future? g) (<!! g) (await g) @g]
        "#,
        "[true :done :done :done]",
    );
}

#[test]
fn async_fns_run_on_fibers_and_may_deref() {
    // A deref inside an ^:async body used to be rejected by the colored
    // evaluator; on fibers it parks like await.
    assert_both(
        r#"
        (defn ^:async slow [x] (sleep 1) (collect-now) (* x 10))
        (defn ^:async total [xs] (reduce + (map (fn [x] @(slow x)) xs)))
        (let [f (total [1 2 3])]
          [(future? f) (await f)])
        "#,
        "[true 60]",
    );
}

#[test]
#[cfg_attr(
    feature = "no-gc",
    ignore = "pre-existing no-gc bug: values handed between two parked fibers over a \
              rendezvous channel are released early (also reproduces with plain `future`)"
)]
fn async_calls_are_concurrent() {
    // Both bodies must be parked at once for the rendezvous to complete.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan <! >!]])
        (def ping (chan))
        (def pong (chan))
        (defn ^:async left [] (>! ping :ping) (<! pong))
        (defn ^:async right [] (let [v (<! ping)] (>! pong :pong) v))
        (let [l (left) r (right)]
          [(await l) (await r)])
        "#,
        "[:pong :ping]",
    );
}

#[test]
fn alts_bang_selects_in_go() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan go alts! >!! <!!]])
        (def c1 (chan))
        (def c2 (chan))
        (def picked (go (let [[v ch] (alts! [c1 c2])]
                          [v (= ch c2)])))
        (>!! c2 :two)
        (<!! picked)
        "#,
        "[:two true]",
    );
}

#[test]
fn colored_spelling_still_works() {
    // Code written for the old evaluator keeps its meaning on fibers.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan go take! put! alt timeout]])
        (def ch (chan 1))
        (go (await (put! ch 5)))
        (defn ^:async pick []
          (alt (take! ch)     (fn [v] [:value v])
               (timeout 1000) (fn [_] [:timed-out])))
        (await (pick))
        "#,
        "[:value 5]",
    );
}

#[test]
fn go_errors_settle_the_future() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go]])
        (def g (go (throw (ex-info "boom" {:k 1}))))
        (try (await g) (catch Exception e (ex-data e)))
        "#,
        "{:k 1}",
    );
}
