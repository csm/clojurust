#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]
use cljrs_async::{fiber, spawn_future};
use cljrs_gc::GcPtr;
use cljrs_runtime::env::gc_roots::push_env_root;
use cljrs_runtime::env::{
    callback,
    env::{Env, GlobalEnv},
    error::{EvalResult, value_error_to_eval_error},
};
use cljrs_value::{Arity, NativeFn, Value};
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

fn eval(source: &str, env: &mut Env) -> EvalResult {
    let forms = cljrs_reader::Parser::new(source.into(), "<parking-test>".into())
        .parse_all()
        .unwrap();
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)?;
    }
    Ok(result)
}
fn run<F: Future<Output = ()>>(test: impl Fn(Arc<GlobalEnv>) -> F + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            for mode in [
                cljrs_runtime::ExecutionMode::TreeWalk,
                cljrs_runtime::ExecutionMode::Tiered,
            ] {
                let runtime = cljrs_runtime::Runtime::builder()
                    .execution_mode(mode)
                    .build()
                    .unwrap();
                let globals = runtime.globals().clone();
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .unwrap();
                tokio::task::LocalSet::new().block_on(&rt, async {
                    cljrs_async::init(&globals);
                    let fork = NativeFn::new("fiber-call", Arity::Fixed(1), |args| {
                        let thunk = args[0].clone();
                        let (globals, ns) = callback::capture_eval_context().unwrap();
                        let task = fiber::Fiber::with_roots(vec![thunk.clone()], move || {
                            let _context = callback::install_eval_context_guard(globals, ns);
                            callback::invoke(&thunk, Vec::new()).map_err(value_error_to_eval_error)
                        })
                        .unwrap()
                        .into_task();
                        Ok(spawn_future(task))
                    });
                    globals.intern(
                        "user",
                        Arc::from("fiber-call"),
                        Value::NativeFunction(GcPtr::new(fork)),
                    );
                    tokio::time::timeout(Duration::from_secs(10), test(globals))
                        .await
                        .expect("parking test timed out");
                });
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
async fn check(globals: Arc<GlobalEnv>, source: &'static str, expected: &'static str) {
    let task = fiber::spawn(move || {
        let mut env = Env::new(globals, "user");
        let _root = push_env_root(&env);
        let result = eval(source, &mut env)?;
        assert_eq!(format!("{result}"), expected);
        Ok(Value::Nil)
    })
    .unwrap();
    task.await.unwrap().into_result().unwrap();
}

#[test]
fn blocking_style_channels_future_promise_and_timeout() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (let [ch (a/chan) p (promise)
                f (fiber-call (fn [] (Thread/sleep 2) (a/>!! ch 40) (deliver p 2) 42))]
            [(a/<!! ch) @p @f (deref (promise) 1 :timeout)
             (deref (fiber-call (fn [] (sleep 100) 7)) 1 :timeout)
             (try @(fiber-call (fn [] (throw (ex-info "boom" {:x 1}))))
                  (catch Exception e (:x (ex-data e))))])"#,
            "[40 2 42 :timeout :timeout 1]",
        )
        .await;
    });
}

#[test]
fn competing_realizers_receive_cached_values_and_errors() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [n (atom 0)
                d (delay (swap! n inc) (sleep 10) (str "delayed"))
                s (lazy-seq (swap! n inc) (sleep 10) (list "lazy"))
                a (fiber-call (fn [] [@d (first s)]))
                b (fiber-call (fn [] [@d (first s)]))]
            [@a @b @n
             (let [e (delay (sleep 1) (throw (ex-info "failure" {})))]
               [(try @e (catch Exception _ :error))
                (try @e (catch Exception _ :error))])])"#,
            r#"[["delayed" "lazy"] ["delayed" "lazy"] 2 [:error :error]]"#,
        )
        .await;
    });
}

#[test]
fn monitors_are_reentrant_and_release_after_errors() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [lock (atom nil) n (atom 0)
                a (fiber-call (fn [] (locking lock (locking lock
                    (let [v @n] (sleep 5) (reset! n (inc v)))))))
                b (fiber-call (fn [] (locking lock
                    (let [v @n] (sleep 1) (reset! n (inc v))))))]
            @a @b
            (try (locking lock (sleep 1) (throw (ex-info "failure" {})))
                 (catch Exception _ nil))
            (locking lock @n))"#,
            "2",
        )
        .await;
    });
}

#[test]
fn alts_takes_only_one_value_and_reports_the_channel() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (let [x (a/chan 1) y (a/chan 1)
                f (fiber-call (fn [] (sleep 2) (a/>!! y 42)))]
            (let [[v ch] (a/alts!! [x y])]
              @f (a/>!! x 7) (a/close! y)
              [v (identical? ch y) (a/<!! x) (a/<!! y)]))"#,
            "[42 true 7 nil]",
        )
        .await;
    });
}

#[test]
fn blocked_native_work_runs_on_worker_and_allows_other_fibers() {
    run(|_| async move {
        let isolate_thread = std::thread::current().id();
        let (tx, rx) = std::sync::mpsc::channel();
        let blocked = fiber::spawn(move || {
            let value = cljrs_value::parking::blocking(move || {
                assert_ne!(std::thread::current().id(), isolate_thread);
                rx.recv_timeout(Duration::from_secs(2)).unwrap()
            });
            Ok(Value::Long(value))
        })
        .unwrap();
        let unblock = fiber::spawn(move || {
            tx.send(42).unwrap();
            Ok(Value::Nil)
        })
        .unwrap();
        assert_eq!(
            blocked.await.unwrap().into_result().unwrap(),
            Value::Long(42)
        );
        unblock.await.unwrap().into_result().unwrap();
    });
}

#[test]
fn promises_outside_fibers_still_support_blocking_and_timeouts() {
    let p = cljrs_value::CljxPromise::new();
    assert_eq!(p.wait_value(Some(Duration::ZERO)), None);
    p.deliver(Value::Long(42));
    assert_eq!(p.deref_blocking(), Value::Long(42));
}

#[test]
fn cancellation_retracts_rendezvous_offers() {
    run(|_| async move {
        let channel = cljrs_async::channel::make_chan(0);
        let anchor = Value::NativeObject(channel.clone());
        let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
        let put_channel = channel.clone();
        let put = fiber::spawn_rooted(vec![Value::NativeObject(channel.clone())], move || {
            cljrs_async::channel::chan_ref(put_channel.get()).put_blocking(Value::Long(1));
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        put.abort();
        assert!(put.await.is_err());
        let ch = cljrs_async::channel::chan_ref(channel.get());
        assert!(
            tokio::time::timeout(Duration::from_millis(2), ch.take())
                .await
                .is_err()
        );
        ch.close();
        assert_eq!(ch.take().await, Value::Nil);
    });
}

#[derive(Debug)]
struct WaitingThunk(Value, bool);
impl cljrs_gc::Trace for WaitingThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        cljrs_gc::Trace::trace(&self.0, visitor);
    }
}
impl cljrs_value::Thunk for WaitingThunk {
    fn force(&self) -> Result<Value, String> {
        if self.1 {
            fiber::park_on(std::future::pending::<()>());
        } else {
            cljrs_value::parking::sleep(Duration::from_millis(10));
        }
        Ok(self.0.clone())
    }
}
#[test]
fn realizing_thunks_remain_traced() {
    run(|_| async move {
        let delay = Value::Delay(GcPtr::new(cljrs_value::Delay::new(Box::new(WaitingThunk(
            Value::string("retained"),
            false,
        )))));
        let _root = cljrs_runtime::env::gc_roots::root_value(&delay);
        let copy = delay.clone();
        let a = fiber::spawn_rooted(vec![delay.clone()], move || {
            let Value::Delay(cell) = copy else {
                unreachable!()
            };
            cell.get()
                .force()
                .map_err(cljrs_runtime::env::error::EvalError::Runtime)
        })
        .unwrap();
        tokio::task::yield_now().await;
        #[cfg(not(feature = "no-gc"))]
        {
            cljrs_gc::request_gc();
            cljrs_runtime::env::gc_roots::async_gc_collect();
        }
        assert_eq!(
            format!("{}", a.await.unwrap().into_result().unwrap()),
            "\"retained\""
        );
    });
}

#[test]
fn cancelling_a_monitor_owner_releases_parked_waiters() {
    run(|_| async move {
        let lock = Value::Atom(GcPtr::new(cljrs_value::Atom::new(Value::Nil)));
        let lock_a = lock.clone();
        let a = fiber::spawn_rooted(vec![lock.clone()], move || {
            let _monitor = cljrs_runtime::env::monitor::enter(&lock_a).unwrap();
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        let b = fiber::spawn_rooted(vec![lock.clone()], move || {
            let _monitor = cljrs_runtime::env::monitor::enter(&lock).unwrap();
            Ok(Value::Long(42))
        })
        .unwrap();
        tokio::task::yield_now().await;
        a.abort();
        assert!(a.await.is_err());
        assert_eq!(b.await.unwrap().into_result().unwrap(), Value::Long(42));
    });
}

#[test]
fn native_higher_order_calls_and_future_cancellation_can_park() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (require 'cljrs.core.experimental)
          (let [ch (a/chan 1)
                p (fiber-call (fn [] (dotimes [i 4] (a/>!! ch i))))
                total (reduce (fn [acc _] (+ acc (a/<!! ch))) 0 (range 4))
                f (fiber-call (fn [] (sleep 1000) 7))
                c (fiber-call (fn [] (sleep 2) (cljrs.core.experimental/future-cancel f)))]
            [total @p (try (deref f 100 :timeout) (catch Exception _ :cancelled)) @c])"#,
            "[6 nil :cancelled true]",
        )
        .await;
    });
}

#[test]
fn synchronous_file_builtins_work_inside_fibers() {
    run(|globals| async move {
        let path =
            std::env::temp_dir().join(format!("clojurust-fiber-file-{}", std::process::id()));
        let source = format!(
            "(do (spit {:?} \"fiber file\") (slurp {:?}))",
            path.to_str().unwrap(),
            path.to_str().unwrap()
        );
        let task = fiber::spawn(move || {
            let mut env = Env::new(globals, "user");
            let _root = push_env_root(&env);
            eval(&source, &mut env)
        })
        .unwrap();
        assert_eq!(
            task.await.unwrap().into_result().unwrap(),
            Value::string("fiber file")
        );
        std::fs::remove_file(path).unwrap();
    });
}

#[test]
fn closing_a_channel_wakes_every_parked_putter() {
    run(|_| async move {
        for capacity in [0, 1] {
            let channel = cljrs_async::channel::make_chan(capacity);
            let anchor = Value::NativeObject(channel.clone());
            let _root = cljrs_runtime::env::gc_roots::root_value(&anchor);
            if capacity == 1 {
                assert!(cljrs_async::channel::chan_ref(channel.get()).put_blocking(Value::Long(0)));
            }
            let mut putters = Vec::new();
            for _ in 0..16 {
                let copy = channel.clone();
                putters.push(
                    fiber::spawn_rooted(vec![Value::NativeObject(channel.clone())], move || {
                        assert!(
                            !cljrs_async::channel::chan_ref(copy.get())
                                .put_blocking(Value::Long(1))
                        );
                        Ok(Value::Nil)
                    })
                    .unwrap(),
                );
            }
            tokio::task::yield_now().await;
            cljrs_async::channel::chan_ref(channel.get()).close();
            for putter in putters {
                putter.await.unwrap().into_result().unwrap();
            }
        }
    });
}

#[derive(Debug)]
struct RetryingThunk {
    attempts: std::rc::Rc<std::cell::Cell<usize>>,
    panic_first: bool,
    value: Value,
}
impl cljrs_gc::Trace for RetryingThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        cljrs_gc::Trace::trace(&self.value, visitor);
    }
}
impl cljrs_value::Thunk for RetryingThunk {
    fn force(&self) -> Result<Value, String> {
        let attempt = self.attempts.get() + 1;
        self.attempts.set(attempt);
        if attempt == 1 {
            if self.panic_first {
                panic!("first realization panicked");
            }
            fiber::park_on(std::future::pending::<()>());
        }
        Ok(self.value.clone())
    }
}
fn realize(value: &Value) -> EvalResult {
    match value {
        Value::Delay(cell) => cell.get().force(),
        Value::LazySeq(cell) => cell.get().try_realize(),
        _ => unreachable!(),
    }
    .map_err(cljrs_runtime::env::error::EvalError::Runtime)
}
#[test]
fn cancelled_and_panicked_realizers_allow_other_readers_to_retry() {
    run(|_| async move {
        for lazy in [false, true] {
            for panic_first in [false, true] {
                let attempts = std::rc::Rc::new(std::cell::Cell::new(0));
                let thunk = RetryingThunk {
                    attempts: attempts.clone(),
                    panic_first,
                    value: Value::string("retried"),
                };
                let cell = if lazy {
                    Value::LazySeq(GcPtr::new(cljrs_value::LazySeq::new(Box::new(thunk))))
                } else {
                    Value::Delay(GcPtr::new(cljrs_value::Delay::new(Box::new(thunk))))
                };
                let _root = cljrs_runtime::env::gc_roots::root_value(&cell);
                let copy = cell.clone();
                let first =
                    fiber::spawn_rooted(vec![cell.clone()], move || realize(&copy)).unwrap();
                tokio::task::yield_now().await;
                let copy = cell.clone();
                let waiter =
                    fiber::spawn_rooted(vec![cell.clone()], move || realize(&copy)).unwrap();
                tokio::task::yield_now().await;
                if panic_first {
                    assert!(first.await.unwrap().into_result().is_err());
                } else {
                    first.abort();
                    assert!(first.await.is_err());
                }
                assert_eq!(
                    waiter.await.unwrap().into_result().unwrap(),
                    Value::string("retried")
                );
                assert_eq!(realize(&cell).unwrap(), Value::string("retried"));
                assert_eq!(attempts.get(), 2);
            }
        }
    });
}
#[test]
fn carrier_waits_on_same_thread_fibers_fail_without_poisoning_values() {
    run(|globals| async move {
        let lock = Value::Atom(GcPtr::new(cljrs_value::Atom::new(Value::Nil)));
        let delay = Value::Delay(GcPtr::new(cljrs_value::Delay::new(Box::new(WaitingThunk(
            Value::Nil,
            true,
        )))));
        let lazy = Value::LazySeq(GcPtr::new(cljrs_value::LazySeq::new(Box::new(
            WaitingThunk(Value::Nil, true),
        ))));
        let roots = vec![lock.clone(), delay.clone(), lazy.clone()];
        let _root = cljrs_runtime::env::gc_roots::root_values(&roots);
        let copy = lock.clone();
        let monitor = fiber::spawn_rooted(vec![copy.clone()], move || {
            let _guard = cljrs_runtime::env::monitor::enter(&copy).unwrap();
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        let mut realizers = Vec::new();
        for value in [delay.clone(), lazy.clone()] {
            realizers
                .push(fiber::spawn_rooted(vec![value.clone()], move || realize(&value)).unwrap());
        }
        tokio::task::yield_now().await;
        let mut env = Env::new(globals, "user");
        env.push_frame();
        env.bind("lock".into(), lock);
        env.bind("d".into(), delay.clone());
        env.bind("s".into(), lazy.clone());
        let _root = push_env_root(&env);
        let result = eval(
            r#"[(try (locking lock 1) (catch Exception _ :rejected))
                              (try @d (catch Exception _ :rejected))
                              (try (first s) (catch Exception _ :rejected))]"#,
            &mut env,
        )
        .unwrap();
        assert_eq!(format!("{result}"), "[:rejected :rejected :rejected]");
        let Value::LazySeq(lazy) = lazy else {
            unreachable!()
        };
        assert!(lazy.get().error().is_none());
        assert!(matches!(
            &*lazy.get().state.lock().unwrap(),
            cljrs_value::types::LazySeqState::Realizing { .. }
        ));
        let Value::Delay(delay) = delay else {
            unreachable!()
        };
        assert!(!delay.get().is_realized());
        monitor.abort();
        assert!(monitor.await.is_err());
        for realizer in realizers {
            realizer.abort();
            assert!(realizer.await.is_err());
        }
    });
}
#[test]
fn alts_does_not_starve_a_ready_channel_later_in_the_input() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (let [x (a/chan 1) y (a/chan 1)]
            (a/>!! x 1) (a/>!! y 2)
            (loop [i 0 seen #{}]
              (if (< i 256)
                (let [[v ch] (a/alts!! [x y])]
                  (a/>!! ch v)
                  (recur (inc i) (conj seen (identical? ch x))))
                (count seen))))"#,
            "2",
        )
        .await;
    });
}

#[test]
fn sequence_consumers_propagate_carrier_wait_errors() {
    run(|globals| async move {
        let lazy = Value::LazySeq(GcPtr::new(cljrs_value::LazySeq::new(Box::new(
            WaitingThunk(Value::Nil, true),
        ))));
        let copy = lazy.clone();
        let realizer = fiber::spawn_rooted(vec![lazy.clone()], move || realize(&copy)).unwrap();
        tokio::task::yield_now().await;
        let mut env = Env::new(globals, "user");
        env.push_frame();
        env.bind("s".into(), lazy.clone());
        let _root = push_env_root(&env);
        for source in [
            "(let [[x] s] x)",
            "(= s '(1))",
            "(hash s)",
            "(pr-str s)",
            "(str s)",
            "(get {s 1} '(1))",
            "{s 1 :a 2 :b 3 :c 4 :d 5 :e 6 :f 7 :g 8 :h 9}",
            "#{s}",
            "(apply vector s)",
            "(let [f (fn [& xs] (if (seq? xs) (recur s) xs))] (f 1))",
            "`(~@s)",
            "(do (require '[clojure.core.async :as a]) (a/alts!! s))",
            "(clojure.core.async/join-all s)",
            "(clojure.core.async/onto-chan! (clojure.core.async/chan 1) s)",
            "(clojure.core.async/to-chan! s)",
        ] {
            let error = eval(source, &mut env).expect_err(source);
            assert!(
                error.to_string().contains("parked fiber"),
                "{source}: {error}"
            );
        }
        let Value::LazySeq(cell) = &lazy else {
            unreachable!()
        };
        assert!(cell.get().error().is_none());
        realizer.abort();
        assert!(realizer.await.is_err());
    });
}

#[test]
fn sequence_consumers_preserve_cached_thunk_errors() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [s (lazy-seq (throw (ex-info "bad-sequence" {})))]
            [(try (let [[x] s] x) (catch Exception _ :rejected))
             (try (apply vector s) (catch Exception _ :rejected))
             (try `(~@s) (catch Exception _ :rejected))])"#,
            "[:rejected :rejected :rejected]",
        )
        .await;
    });
}

#[test]
fn sequence_conversion_parks_for_a_competing_realizer() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [s (lazy-seq (sleep 2)
                    (cons (str "head") (lazy-seq (sleep 2) (list (str "tail")))))
                owner (fiber-call (fn [] (first s)))
                consumer (fiber-call (fn [] (let [[x y] s] [x y])))
                spread (fiber-call (fn [] (apply vector s)))]
            [@owner @consumer @spread])"#,
            r#"["head" ["head" "tail"] ["head" "tail"]]"#,
        )
        .await;
    });
}

#[test]
fn alts_put_default_priority_and_rendezvous_are_atomic() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (let [x (a/chan 1) y (a/chan 1) rv (a/chan)]
            [(first (a/alts!! [x [y 7]] :priority true))
             (a/alts!! [x] :default :empty)
             (a/alts!! [[y 8]] :default :full)
             (a/<!! y)
             (let [taker (fiber-call (fn [] (a/<!! rv)))
                   choice (a/alts!! [[rv 42] x])]
               [(first choice) (identical? (second choice) rv) @taker])
             (let [other (a/chan 1)]
               (a/>!! other 9)
               [(first (a/alts!! [[rv 100] other] :priority true))
                (a/alts!! [rv] :default :no-ghost)])])"#,
            "[true [:empty :default] [:full :default] 7 [true true 42] [9 [:no-ghost :default]]]",
        )
        .await;
    });
}

#[test]
fn explicit_monitors_are_reentrant_and_have_fiber_ownership() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [lock (atom nil) n (atom 0)
                a (fiber-call (fn []
                    (monitor-enter lock) (monitor-enter lock)
                    (sleep 3) (swap! n inc)
                    (monitor-exit lock) (monitor-exit lock)))
                b (fiber-call (fn [] (locking lock (swap! n inc))))]
            @a @b
            [@n (try (monitor-exit lock) (catch Exception _ :not-owner))])"#,
            "[2 :not-owner]",
        )
        .await;
    });
}

#[test]
fn cancelled_explicit_monitor_owners_release_waiters() {
    run(|_| async move {
        let lock = Value::Atom(GcPtr::new(cljrs_value::Atom::new(Value::Nil)));
        let copy = lock.clone();
        let owner = fiber::spawn_rooted(vec![copy.clone()], move || {
            cljrs_runtime::env::monitor::monitor_enter(&copy).unwrap();
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        let waiter = fiber::spawn_rooted(vec![lock.clone()], move || {
            let _guard = cljrs_runtime::env::monitor::enter(&lock).unwrap();
            Ok(Value::Long(42))
        })
        .unwrap();
        tokio::task::yield_now().await;
        owner.abort();
        assert!(owner.await.is_err());
        assert_eq!(
            waiter.await.unwrap().into_result().unwrap(),
            Value::Long(42)
        );
    });
}

#[test]
fn failed_transient_hashing_does_not_poison_or_mutate_the_collection() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [s (lazy-seq (throw (ex-info "hash failed" {})))
                m (transient {}) t (transient #{})]
            [(try (assoc! m s 1) (catch Exception _ :rejected))
             (persistent! (assoc! m :ok 2))
             (try (conj! t s) (catch Exception _ :rejected))
             (persistent! (conj! t :ok))])"#,
            "[:rejected {:ok 2} :rejected #{:ok}]",
        )
        .await;
    });
}

#[test]
fn two_alts_rendezvous_selectors_commit_only_one_pair() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (require '[clojure.core.async :as a])
          (let [x (a/chan) y (a/chan)
                p (fiber-call (fn [] (a/alts!! [[x 1] [y 2]])))
                result (a/alts!! [x y])
                sent @p]
            [(first sent) (identical? (second result) (second sent))
             (a/alts!! [x y] :default :empty)
             (a/alts!! [] :default nil)])"#,
            "[true true [:empty :default] [nil :default]]",
        )
        .await;
    });
}

#[test]
fn transient_updates_preserve_changes_made_while_a_key_realizes() {
    run(|globals| async move {
        check(
            globals,
            r#"
          (let [m (transient {}) s (transient #{})
                key (lazy-seq (sleep 5) (list 1))
                set-key (lazy-seq (sleep 5) (list 2))
                a (fiber-call (fn [] (assoc! m key :lazy)))
                b (fiber-call (fn [] (sleep 1) (assoc! m :other :kept)))
                c (fiber-call (fn [] (conj! s set-key)))
                d (fiber-call (fn [] (sleep 1) (conj! s :other)))]
            @a @b @c @d
            (let [m (persistent! m) s (persistent! s)]
              [(count m) (get m '(1)) (:other m)
               (count s) (contains? s '(2)) (contains? s :other)]))"#,
            "[2 :lazy :kept 2 true true]",
        )
        .await;
    });
}
