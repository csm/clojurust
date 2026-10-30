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

#[test]
fn pending_local_future_rejects_carrier_waits_and_root_can_borrow_env() {
    cljrs_async::isolate::Isolate::new("carrier-wait-review")
        .spawn(|| async {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            cljrs_async::init(runtime.globals());
            let mut env = Env::new(runtime.globals().clone(), "user");
            let forms = cljrs_reader::Parser::new("@(future 42)".into(), "<carrier>".into())
                .parse_all()
                .unwrap();
            assert!(
                cljrs_value::CljxFuture::new_local()
                    .wait_value(Some(std::time::Duration::ZERO))
                    .unwrap()
                    .is_none()
            );
            let error = cljrs_runtime::tiered::eval(&forms[0], &mut env).unwrap_err();
            assert!(error.to_string().contains("outside a fiber"), "{error}");
            // A regular function called by a legacy go body must also reject the
            // carrier wait instead of pinning the async evaluator.
            let forms = cljrs_reader::Parser::new(
                "(defn ordinary [] @(future 42)) (await (clojure.core.async/go (ordinary)))".into(),
                "<legacy>".into(),
            )
            .parse_all()
            .unwrap();
            cljrs_async::eval_async::eval_async(&forms[0], &mut env)
                .await
                .unwrap();
            let error = cljrs_async::eval_async::eval_async(&forms[1], &mut env)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("outside a fiber"), "{error}");
        })
        .join()
        .unwrap();

    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            cljrs_async::init(runtime.globals());
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            let mut carrier = MutatorState::fresh();
            let mut env = Env::new(runtime.globals().clone(), "user");
            env.push_frame();
            env.bind("saved".into(), Value::Long(42));
            let forms = cljrs_reader::Parser::new("@(future saved)".into(), "<embedding>".into())
                .parse_all()
                .unwrap();
            assert_eq!(
                fiber::run_root(&rt, &local, &mut carrier, || cljrs_runtime::tiered::eval(
                    &forms[0], &mut env
                ))
                .unwrap(),
                Value::Long(42)
            );
            carrier.swap_with_current();
            drop(local);
            carrier.swap_with_current();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn declared_native_captures_survive_collection_in_a_parked_fiber() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async {
                let _ = runtime;
                let frame = cljrs_gc::push_alloc_frame();
                let captured = Value::Vector(cljrs_gc::GcPtr::new(
                    cljrs_value::PersistentVector::from_iter(vec![Value::string(
                        "managed capture",
                    )]),
                ));
                let native = Value::NativeFunction(cljrs_gc::GcPtr::new(
                    cljrs_value::NativeFn::with_captures(
                        "declared-capture",
                        cljrs_value::Arity::Fixed(0),
                        vec![captured],
                        |captures, _| Ok(captures[0].clone()),
                    ),
                ));
                let task = fiber::spawn_rooted(vec![native.clone()], move || {
                    fiber::park_on(tokio::task::yield_now());
                    let Value::NativeFunction(native) = native else {
                        unreachable!()
                    };
                    (native.get().func)(&[])
                        .map_err(cljrs_runtime::env::error::value_error_to_eval_error)
                })
                .unwrap();
                drop(frame);
                tokio::task::yield_now().await;
                #[cfg(not(feature = "no-gc"))]
                for _ in 0..3 {
                    cljrs_gc::request_gc();
                    cljrs_runtime::env::gc_roots::async_gc_collect();
                }
                assert_eq!(
                    task.await.unwrap().into_result().unwrap().to_string(),
                    "[\"managed capture\"]"
                );
            });
        })
        .unwrap()
        .join()
        .unwrap();
}
