#![cfg(all(feature = "no-gc", not(target_arch = "wasm32")))]
#![allow(clippy::result_large_err)]
use cljrs_async::fiber;
use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::env::env::Env;

static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "no-gc")]
#[test]
fn repeated_publishes_and_completed_spawns_release_scratch_regions() {
    let _lock = TEST_LOCK.lock().unwrap();
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async {
                cljrs_async::init(runtime.globals());
            });
            let mut carrier = MutatorState::fresh();
            runtime.globals().intern(
                "user",
                "scratch-result?".into(),
                cljrs_value::Value::NativeFunction(cljrs_gc::GcPtr::new(
                    cljrs_value::NativeFn::new(
                        "scratch-result?",
                        cljrs_value::Arity::Fixed(1),
                        |args| {
                            let cljrs_value::Value::Vector(vector) = &args[0] else {
                                panic!("not a vector")
                            };
                            Ok(cljrs_value::Value::Bool(!vector.is_static_alloc()))
                        },
                    ),
                )),
            );
            let mut env = Env::new(runtime.globals().clone(), "user");
            let retired = || {
                cljrs_gc::governor::with_current_account(|account| account.retired_region_bytes())
                    .unwrap()
            };
            let source = r#"
          (require '[clojure.core.async :as a])
          (def values (atom []))
          (dotimes [i 300]
            (swap! values conj i)
            (assert (= (inc i) @(future (inc i))))
            (assert (= [i] (a/<!! (a/thread [i])))))
          (dotimes [i 300] (future i))
          (sleep 1)
          (defn child []
            (let [text (str "retained " "value")]
              (future (sleep 1) @(future text))))
          (assert (= "retained value" @(child)))
        "#;
            for form in cljrs_reader::Parser::new(source.into(), "<no-gc-retention>".into())
                .parse_all()
                .unwrap()
            {
                fiber::run_root(&rt, &local, &mut carrier, || {
                    cljrs_runtime::tiered::eval(&form, &mut env)
                })
                .unwrap();
            }
            // Static definitions retain their actual backing scopes. Completed
            // transient tasks must not add to that baseline on later batches.
            let workload = cljrs_reader::Parser::new(
                r#"
              (defn transient-batch []
                (dotimes [i 10]
                (let [lock []]
                  (monitor-enter lock)
                  (monitor-exit lock))
                (let [v @(future (vec (range 1000)))
                      w (a/<!! (a/thread (vec (range 1000))))]
                  (assert (scratch-result? v))
                  (assert (scratch-result? w))
                  (assert (= 1000 (count v)))
                  (assert (= 1000 (count w)))))
                nil)
              (transient-batch)
            "#
                .into(),
                "<bounded-results>".into(),
            )
            .parse_all()
            .unwrap();
            let (definition, workload) = workload.split_first().unwrap();
            fiber::run_root(&rt, &local, &mut carrier, || {
                cljrs_runtime::tiered::eval(definition, &mut env)
            })
            .unwrap();
            let mut samples = Vec::new();
            let mut static_samples = Vec::new();
            for _ in 0..3 {
                for form in workload {
                    fiber::run_root(&rt, &local, &mut carrier, || {
                        cljrs_runtime::tiered::eval(form, &mut env)
                    })
                    .unwrap();
                }
                samples.push(retired());
                static_samples.push(
                    cljrs_gc::governor::snapshot().class_bytes(cljrs_gc::MemoryClass::Static),
                );
            }
            cljrs_async::fiber::drop_local_with_carrier(local, &mut carrier);
            assert_eq!(
                samples[1..],
                [samples[0]; 2],
                "completed tasks permanently retained scratch regions: {samples:?}"
            );
            assert!(
                static_samples[2] - static_samples[0] <= 256 * 1024,
                "completed results accumulated in static storage: {static_samples:?}"
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn publications_preserve_laziness_and_dynamic_return_storage() {
    let _lock = TEST_LOCK.lock().unwrap();
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async {
                cljrs_async::init(runtime.globals());
            });
            fiber::run_root_owned(&rt, local, || {
                let mut env = Env::new(runtime.globals().clone(), "user");
                let source = r#"
              (require '[clojure.core.async :as a])
              (def touched (atom 0))
              (def xs (lazy-seq (swap! touched inc) (list 9)))
              (assert (= 0 @touched))
              (let [f (future (lazy-seq (swap! touched inc) (list 10)))
                    ch (a/thread (lazy-seq (swap! touched inc) (list 11)))
                    ys @f
                    zs (a/<!! ch)]
                (assert (= 0 @touched))
                (assert (= 10 (first ys)))
                (assert (= 11 (first zs))))
              (assert (= 2 @touched))
              (def infinite (iterate inc 0))
              (assert (= [0 1 2] (vec (take 3 infinite))))
              (let [ys @(future (range))
                    zs (a/<!! (a/thread (iterate inc 0)))]
                (assert (= 3 (nth ys 3)))
                (assert (= 3 (nth zs 3))))
              (def ^:dynamic *state* {})
              (defn write-state []
                (set! *state* {:text (str "scoped " "binding")})
                nil)
              (binding [*state* {}]
                (write-state)
                (dotimes [i 50] (str "reuse " i))
                (assert (= "scoped binding" (:text *state*))))
              (let [array (object-array 1)
                    write-array (fn [] (aset array 0 (str "scoped " "array")) nil)]
                (write-array)
                (dotimes [i 50] (str "reuse " i))
                (assert (= "scoped array" (aget array 0))))
              (defn dynamic-return []
                (do (vec (range 200))
                    (if true (str "live" " result") nil)))
              (def kept (dynamic-return))
              (dotimes [i 500] (dynamic-return))
              (assert (= "live result" kept))
              nil
            "#;
                for form in cljrs_reader::Parser::new(source.into(), "<lazy-publication>".into())
                    .parse_all()
                    .unwrap()
                {
                    cljrs_runtime::tiered::eval(&form, &mut env)?;
                }
                Ok(cljrs_value::Value::Nil)
            })
            .unwrap();
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn queued_rust_spawn_results_transfer_into_the_consumers_scope() {
    let _lock = TEST_LOCK.lock().unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    fiber::run_root_owned(&runtime, local, || {
        let retired =
            || cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap();
        let before = retired();
        for _ in 0..30 {
            let consumer = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let task = {
                let _caller = cljrs_gc::alloc_ctx::ScratchGuard::new();
                fiber::spawn(|| {
                    Ok(cljrs_value::Value::Vector(cljrs_gc::GcPtr::new(
                        cljrs_value::PersistentVector::from_iter([cljrs_value::Value::Long(42)]),
                    )))
                })
                .unwrap()
            };
            fiber::park_on(tokio::task::yield_now());
            let value = fiber::park_on(task).unwrap().into_result()?;
            let cljrs_value::Value::Vector(vector) = value else {
                panic!("not a vector")
            };
            assert!(!vector.is_static_alloc());
            assert_eq!(
                vector.get().iter().next(),
                Some(&cljrs_value::Value::Long(42))
            );
            drop(consumer);
        }
        assert_eq!(retired(), before);
        Ok(cljrs_value::Value::Nil)
    })
    .unwrap();
}
