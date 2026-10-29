#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use cljrs_async::fiber;
use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::env::{env::Env, gc_roots::root_value};
use cljrs_value::Value;

#[test]
fn blocking_surface_and_bindings_in_both_interpreters() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            for mode in [
                cljrs_runtime::ExecutionMode::TreeWalk,
                cljrs_runtime::ExecutionMode::Tiered,
            ] {
                let runtime = cljrs_runtime::Runtime::builder()
                    .execution_mode(mode)
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
                            for _ in 0..3 {
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
                    let source = r#"
                  (require '[clojure.core.async :as a])
                  (def ^:dynamic *x* :root)
                  (defn ordinary [] @(future (sleep 1) 42))
                  (let [gate (a/chan)
                        f (binding [*x* :parent]
                            (future (a/<!! gate)
                              (let [inherited *x*]
                                (binding [*x* :child]
                                  [inherited @(future *x*)]))))
                        sibling (binding [*x* :sibling] (future *x*))]
                    (a/>!! gate true)
                    (assert (= [:parent :child] @f))
                    (assert (= :sibling @sibling))
                    (assert (= :root *x*)))
                  (assert (= 42 @(future @(future (ordinary)))))
                  (assert (= 7 @(future-call (fn [] 7))))
                  (defn make-child []
                    (let [s (str "retained" " capture")]
                      (future (sleep 1) [s])))
                  (let [f (make-child)]
                    (collect-now)
                    (assert (= ["retained capture"] @f)))
                  (let [ch (a/chan)
                        result (a/io-thread (a/>!! ch "hello") "world")]
                    (assert (= "hello" (a/<!! ch)))
                    (assert (= "world" (a/<!! result)))
                    (assert (nil? (a/<!! result))))
                  (assert (= 9 (a/<!! (a/thread-call (fn [] (sleep 1) 9)))))
                  (assert (nil? (a/<!! (a/thread nil))))
                  (assert (nil? (a/<!! (a/io-thread (throw (ex-info "failure" {}))))))
                  (assert (= :caught (try @(future (throw (ex-info "failure" {})))
                                      (catch Exception e :caught))))
                  (let [p (promise) f (future @p)]
                    (assert (future? f))
                    (assert (= :timeout (deref f 1 :timeout)))
                    (assert (future-cancel f))
                    (deliver p 1)
                    (sleep 1)
                    (assert (future-cancelled? f))
                    (assert (= :cancelled (try @f (catch Exception e :cancelled)))))
                  (require '[cljrs.core.experimental :as x])
                  (assert (= 5 @(x/future 5)))
                  :passed
                "#;
                    let forms = cljrs_reader::Parser::new(source.into(), "<fiber-surface>".into())
                        .parse_all()
                        .unwrap();
                    let mut result = Value::Nil;
                    for form in forms {
                        result = cljrs_runtime::tiered::eval(&form, &mut env)?;
                    }
                    Ok(result)
                })
                .unwrap();
                assert_eq!(result.to_string(), ":passed");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn root_restores_host_capture_and_preserves_background_roots_between_drives() {
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
        let globals = runtime.globals().clone();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async { cljrs_async::init(&globals) });
        let mut carrier = MutatorState::fresh();
        cljrs_runtime::builtins::builtins::push_output_capture();
        let g = globals.clone();
        let future = fiber::run_root(&rt, &local, &mut carrier, move || {
            let mut env = Env::new(g, "user");
            let forms = cljrs_reader::Parser::new(
                "(defn ^:async later [] (await (clojure.core.async/timeout 5)) [1 2 3]) (println :before) (later)".into(),
                "<root>".into(),
            ).parse_all().unwrap();
            let mut result = Value::Nil;
            for form in forms { result = cljrs_runtime::tiered::eval(&form, &mut env)?; }
            // Poll the legacy task before the first drive returns.
            fiber::park_on(tokio::task::yield_now());
            Ok(result)
        }).unwrap();
        let anchor = future.clone();
            let _root = root_value(&anchor);
        let value = fiber::run_root(&rt, &local, &mut carrier, move || {
            #[cfg(not(feature = "no-gc"))]
            {
                cljrs_gc::request_gc();
                cljrs_runtime::env::gc_roots::async_gc_collect();
            }
            fiber::park_on(cljrs_async::await_value(future))
        }).unwrap();
        assert_eq!(value.to_string(), "[1 2 3]");
        assert_eq!(cljrs_runtime::builtins::builtins::pop_output_capture().unwrap(), ":before\n");
        cljrs_runtime::builtins::builtins::push_output_capture();
        assert!(fiber::run_root(&rt, &local, &mut carrier, || panic!("root panic")).is_err());
        assert_eq!(cljrs_runtime::builtins::builtins::pop_output_capture().unwrap(), "");
    }).unwrap().join().unwrap();
}
