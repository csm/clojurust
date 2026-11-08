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
fn go_returns_a_result_channel() {
    // As in core.async: one non-nil value, then the channel is closed.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go <!!]])
        (def g (go (sleep 1) :done))
        (def h (go :awaited))
        [(future? g) (<!! g) (<!! g) (await h) (<!! (go nil))]
        "#,
        "[false :done nil :awaited nil]",
    );
}

#[test]
fn go_channels_compose_with_channel_ops() {
    // alts!, merge, and join-all accept go results like any channel.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go alts! <!! join-all]])
        (def slow (go (sleep 50) :slow))
        (def picked (go (let [[v ch] (alts! [slow (go :fast)])] [v (= ch slow)])))
        [(<!! picked)
         (sort (await (a/into [] (a/merge [(go 1) (go 2)]))))
         (await (join-all [(go :x) (go :y)]))]
        "#,
        "[[:fast false] (1 2) [:x :y]]",
    );
}

#[test]
fn parking_ops_name_themselves_in_errors() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan alts! alts!!]])
        (def c (chan 1))
        (defn msg [f] (try (f) (catch Exception e (str e))))
        [(boolean (re-find #"alts! accepts" (msg #(alts! [c] :foo 1))))
         (boolean (re-find #"alts!! accepts" (msg #(alts!! [c] :foo 1))))]
        "#,
        "[true true]",
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
    ignore = "csm/clojurust#442: no-gc releases values handed between two parked fibers \
              over a rendezvous channel too early (also reproduces with plain `future`)"
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
fn go_errors_close_the_channel() {
    // As in core.async: the exception is reported, and takers see nil.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go <!!]])
        (def g (go (throw (ex-info "boom" {:k 1}))))
        [(<!! g) (await (go (throw (ex-info "again" {}))))]
        "#,
        "[nil nil]",
    );
}

#[test]
fn async_errors_still_settle_the_future() {
    assert_both(
        r#"
        (defn ^:async boom [] (throw (ex-info "boom" {:k 1})))
        (try (await (boom)) (catch Exception e (ex-data e)))
        "#,
        "{:k 1}",
    );
}

#[test]
fn alts_future_selects_exactly_one_and_names_itself() {
    // The future-returning selection behind wasm32's `alts!` macro, called
    // directly: one take completes, the other channel keeps its value, and the
    // result is `[val ch]`. Its errors name `alts!-future`.
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan >!! poll! alts!-future]])
        (def c1 (chan 1))
        (def c2 (chan 1))
        (>!! c1 :one)
        (>!! c2 :two)
        (def picked (await (alts!-future [c1 c2] :priority true)))
        (defn msg [f] (try (f) (catch Exception e (str e))))
        [(first picked)
         (= (second picked) c1)
         (poll! c2)
         (boolean (re-find #"alts!-future accepts"
                           (msg #(await (alts!-future [c1] :bogus 1)))))]
        "#,
        "[:one true :two true]",
    );
}

#[test]
fn alts_keeps_its_ports_rooted_while_parked() {
    // The put value is referenced only by the parked selection. Collections
    // while it waits must not free it (or the channels).
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [chan go alts! <!!]])
        (def c1 (chan))
        (def c2 (chan))
        (def picked (go (let [[v ch] (alts! [c1 [c2 (str "put-" (inc 0))]] :priority true)]
                          [v (= ch c2)])))
        (sleep 1)
        (collect-now)
        (collect-now)
        [(<!! c2) (<!! picked)]
        "#,
        "[\"put-1\" [true true]]",
    );
}

#[test]
fn awaited_go_channels_survive_collections() {
    assert_both(
        r#"
        (require '[clojure.core.async :as a :refer [go]])
        (defn wait [i] (await (go (sleep 1) (collect-now) (vec (range i)))))
        (let [f (future (mapv wait [1 2 3]))]
          (collect-now)
          @f)
        "#,
        "[[0] [0 1] [0 1 2]]",
    );
}

/// `fiber::eval_form_unrooted` (like `eval_form`) conveys the caller's dynamic bindings and gas meters,
/// like `future`, so an embedder's binding or budget governs the form.
#[test]
fn eval_form_conveys_bindings_and_gas() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let globals = runtime.globals().clone();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                cljrs_async::init(&globals);
                let mut env = Env::new(globals.clone(), "user");
                let parse = |src: &str| {
                    cljrs_reader::Parser::new(src.into(), "<eval-form>".into())
                        .parse_all()
                        .unwrap()
                        .remove(0)
                };
                fiber::eval_form_unrooted(&parse("(def ^:dynamic *x* :root)"), &mut env)
                    .await
                    .unwrap();
                let var = globals.lookup_var_in_ns("user", "*x*").unwrap();
                let key = cljrs_runtime::env::dynamics::var_key_of(&var);
                let bound = {
                    let _frame = cljrs_runtime::env::dynamics::push_frame(
                        [(key, Value::keyword(cljrs_value::Keyword::simple("bound")))].into(),
                    );
                    fiber::eval_form_unrooted(&parse("*x*"), &mut env)
                        .await
                        .unwrap()
                };
                assert_eq!(bound.to_string(), ":bound");

                let meter = cljrs_runtime::env::gas::GasMeter::new(1_000);
                let spun = {
                    let _gas = cljrs_runtime::env::gas::GasGuard::install(meter);
                    fiber::eval_form_unrooted(&parse("(loop [] (recur))"), &mut env).await
                };
                assert!(
                    matches!(
                        spun,
                        Err(cljrs_runtime::env::error::EvalError::GasExhausted)
                    ),
                    "{spun:?}"
                );
            });
            fiber::drop_local_with_carrier(local, &mut MutatorState::fresh());
        })
        .unwrap()
        .join()
        .unwrap();
}

/// `fiber::eval_form` keeps its result rooted across later yields and
/// collections, until the `FiberResult` is consumed.
#[test]
fn eval_form_result_survives_later_collections() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let globals = runtime.globals().clone();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async move {
                cljrs_async::init(&globals);
                let mut env = Env::new(globals.clone(), "user");
                let parse = |src: &str| {
                    cljrs_reader::Parser::new(src.into(), "<eval-form>".into())
                        .parse_all()
                        .unwrap()
                        .remove(0)
                };
                let kept = fiber::eval_form(
                    &parse("(mapv (fn [i] (str \"item-\" i)) (range 50))"),
                    &mut env,
                )
                .await;
                for _ in 0..3 {
                    // Allocate on another fiber, then collect, while `kept` waits.
                    fiber::eval_form_unrooted(&parse("(count (vec (range 2000)))"), &mut env)
                        .await
                        .unwrap();
                    tokio::task::yield_now().await;
                    #[cfg(not(feature = "no-gc"))]
                    {
                        cljrs_gc::request_gc();
                        cljrs_runtime::env::gc_roots::async_gc_collect();
                    }
                }
                let Ok(Value::Vector(items)) = kept.get() else {
                    panic!("unexpected {:?}", kept.get());
                };
                assert_eq!(items.get().count(), 50);
                assert_eq!(items.get().nth(49).unwrap().to_string(), "\"item-49\"");
                // Taking the value publishes it to this scope (in no-gc
                // builds, through its region leases); it stays usable while
                // more forms run.
                let taken = kept.into_result().unwrap();
                let _roots = cljrs_runtime::env::gc_roots::root_value(&taken);
                fiber::eval_form_unrooted(&parse("(count (vec (range 2000)))"), &mut env)
                    .await
                    .unwrap();
                #[cfg(not(feature = "no-gc"))]
                {
                    cljrs_gc::request_gc();
                    cljrs_runtime::env::gc_roots::async_gc_collect();
                }
                assert_eq!(taken.to_string().matches("item-").count(), 50);
            });
            fiber::drop_local_with_carrier(local, &mut MutatorState::fresh());
        })
        .unwrap()
        .join()
        .unwrap();
}
