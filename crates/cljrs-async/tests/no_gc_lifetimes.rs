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

#[test]
fn top_level_accumulators_and_replaced_opaque_values_do_not_accumulate_static_copies() {
    let _lock = TEST_LOCK.lock().unwrap();
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let mut env = Env::new(runtime.globals().clone(), "user");
            let evaluate = |source: &str, env: &mut Env| {
                let mut result = cljrs_value::Value::Nil;
                for form in cljrs_reader::Parser::new(source.into(), "<top-level-storage>".into())
                    .parse_all()
                    .unwrap()
                {
                    result = cljrs_runtime::interp::eval::eval(&form, env).unwrap();
                }
                result
            };
            evaluate("(def cell (atom [])) (def opaque (atom nil))", &mut env);
            let before = cljrs_gc::governor::snapshot().class_bytes(cljrs_gc::MemoryClass::Static);
            assert_eq!(
                evaluate(
                    r#"
            (count (loop [acc [] i 0]
                (if (< i 2000) (recur (conj acc i) (inc i)) acc)))
        "#,
                    &mut env
                ),
                cljrs_value::Value::Long(2000)
            );
            evaluate(
                r#"
            (dotimes [i 2000] (swap! cell conj i))
            (assert (= 2000 (count @cell)))
            (let [old @cell]
                (reset! cell [])
                (assert (= 1999 (nth old 1999))))
            (dotimes [i 300]
                (reset! opaque (fn [] i)))
            (assert (= 299 (@opaque)))
            (reset! opaque nil)
            (loop [xs nil i 0]
                (if (< i 300)
                    (recur (lazy-seq (list i)) (inc i))
                    (assert (= 299 (first xs)))))
        "#,
                &mut env,
            );
            let retired =
                || cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap();
            let baseline = retired();
            for _ in 0..3 {
                evaluate(
                    r#"
                (dotimes [i 100] (reset! opaque (fn [] i)))
                (reset! opaque nil)
                (loop [xs nil i 0]
                    (if (< i 100)
                        (recur (lazy-seq (list i)) (inc i))
                        (assert (= 99 (first xs)))))
            "#,
                    &mut env,
                );
                assert_eq!(
                    retired(),
                    baseline,
                    "replaced opaque contents permanently retained iteration regions"
                );
            }
            let after = cljrs_gc::governor::snapshot().class_bytes(cljrs_gc::MemoryClass::Static);
            assert!(
                after - before < 2 * 1024 * 1024,
                "top-level updates copied accumulators into static storage: {} bytes",
                after - before
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn returned_updates_and_top_level_reads_release_obsolete_backing_scopes() {
    let _lock = TEST_LOCK.lock().unwrap();
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                .build()
                .unwrap();
            let mut env = Env::new(runtime.globals().clone(), "user");
            let evaluate = |source: &str, env: &mut Env| {
                for form in
                    cljrs_reader::Parser::new(source.into(), "<returned-cell-values>".into())
                        .parse_all()
                        .unwrap()
                {
                    // The embedding caller discards this result before its read scope ends.
                    let _read = cljrs_gc::region::ReadScope::enter();
                    cljrs_runtime::interp::eval::eval(&form, env).unwrap();
                }
            };
            evaluate(
                r#"
            (def a (atom (zipmap (range 40) (repeat nil))))
            (def v (volatile! (zipmap (range 40) (repeat nil))))
            (defn seed-a [] (swap! a assoc :j (str "shared" " text")) nil)
            (defn seed-v [] (vswap! v assoc :j (str "shared" " text")) nil)
            (defn take-a []
              (let [m (swap! a assoc :k 1)]
                (reset! a {})
                (dotimes [i 100] (str "reuse " i))
                (assert (= "shared text" (get m :j)))))
            (defn take-v []
              (let [m (vswap! v assoc :k 1)]
                (vreset! v {})
                (dotimes [i 100] (str "reuse " i))
                (assert (= "shared text" (get m :j)))))
            (seed-a) (take-a) (seed-v) (take-v)
            (let [value [1 :a (str "identity" " value")]
                  returned (reset! a value)]
              (assert (identical? value returned))
              (assert (identical? value @a))
              (assert (identical? value (swap! a (fn [old] old))))
              (assert (identical? value (vreset! v value)))
              (assert (identical? value @v)))
            (reset! a {})
            (vreset! v {})
            (defn make-watched []
              (let [text (str "callback" " alive")]
                (atom 1 :meta {:label text} :validator (fn [value] (and (= "callback alive" text) (pos? value))))))
            (def watched (make-watched))
            (def events (atom []))
            (defn install-callbacks []
              (let [text (str "callback" " alive")]
                (add-watch watched :key (fn [key reference old new] (swap! events conj [text old new])))))
            (install-callbacks)
            (dotimes [i 100] (str "reuse " i))
            (assert (= "callback alive" (:label (meta watched))))
            (reset! watched 2)
            (assert (= [["callback alive" 1 2]] @events))
            (assert (= :rejected (try (reset! watched -1) (catch Exception e :rejected))))
            (remove-watch watched :key)

            (let [captured (the-ns 'user)] (def namespace-reader (fn [] captured)))
            (assert (identical? *ns* (namespace-reader)))
            (def namespace-reader nil)
        "#,
                &mut env,
            );
            let retired =
                || cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap();
            // Warm the macro and formatter paths before comparing repeated batches.
            let workload = r#"
            (def f (let [captured (str "captured" " closure")] (fn [] captured)))
            (assert (= "captured closure" (f)))
            (def f nil)
            (swap! a assoc :j (str "new" " payload"))
            (with-out-str (println @a))
            (reset! a {})
        "#;
            evaluate(workload, &mut env);
            let baseline = retired();
            for _ in 0..100 {
                evaluate(workload, &mut env);
                assert_eq!(
                    retired(),
                    baseline,
                    "a transient top-level read or Var rebind permanently retained a version"
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn spawned_tasks_keep_actual_captures_without_inheriting_unrelated_reads() {
    let _lock = TEST_LOCK.lock().unwrap();
    #[derive(Debug)]
    struct Probe(std::rc::Rc<std::cell::Cell<bool>>);
    impl cljrs_gc::Trace for Probe {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    impl cljrs_value::NativeObject for Probe {
        fn type_tag(&self) -> &str {
            "Probe"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        let read = cljrs_gc::region::ReadScope::enter().unwrap();
        let atom = cljrs_gc::GcPtr::new(cljrs_value::Atom::new(cljrs_value::Value::Nil));
        let mut dropped = Vec::new();
        let mut captured = cljrs_value::Value::Nil;
        for _ in 0..100 {
            let probe = std::rc::Rc::new(std::cell::Cell::new(false));
            {
                let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
                atom.get().reset(cljrs_value::Value::NativeObject(
                    cljrs_value::gc_native_object(Probe(probe.clone())),
                ));
            }
            captured = atom.get().deref();
            atom.get().reset(cljrs_value::Value::Nil);
            dropped.push(probe);
        }
        let task =
            cljrs_async::spawn_future_with_roots(
                vec![captured.clone()],
                async move { Ok(captured) },
            );
        drop(read);
        assert!(
            dropped[..99].iter().all(|p| p.get()),
            "a task inherited unrelated read leases"
        );
        assert!(
            !dropped[99].get(),
            "the actual capture was released before polling"
        );
        let _consumer = cljrs_gc::alloc_ctx::ScratchGuard::new();
        assert!(matches!(
            cljrs_async::await_value(task).await.unwrap(),
            cljrs_value::Value::NativeObject(_)
        ));
    });
}

#[test]
fn definitions_transients_and_mutable_slots_preserve_escapes_and_bound_retention() {
    let _lock = TEST_LOCK.lock().unwrap();
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        use cljrs_value::{Arity, NativeFn, NativeObject, Value};
        #[derive(Debug)]
        struct Probe(std::rc::Rc<std::cell::Cell<usize>>);
        impl cljrs_gc::Trace for Probe { fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {} }
        impl Drop for Probe { fn drop(&mut self) { self.0.set(self.0.get() + 1); } }
        impl NativeObject for Probe {
            fn type_tag(&self) -> &str { "ReviewProbe" }
            fn as_any(&self) -> &dyn std::any::Any { self }
        }
        let runtime = cljrs_runtime::Runtime::builder().execution_mode(cljrs_runtime::ExecutionMode::TreeWalk).build().unwrap();
        let dropped = std::rc::Rc::new(std::cell::Cell::new(0));
        let observed = dropped.clone();
        runtime.globals().intern("user", "make-probe".into(), Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::with_closure("make-probe", Arity::Fixed(0), move |_| Ok(Value::NativeObject(cljrs_value::gc_native_object(Probe(observed.clone()))))))));
        let observed = dropped.clone();
        runtime.globals().intern("user", "drop-count".into(), Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::with_closure("drop-count", Arity::Fixed(0), move |_| Ok(Value::Long(observed.get() as i64))))));
        runtime.globals().intern("user", "probe?".into(), Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::new("probe?", Arity::Fixed(1), |args| Ok(Value::Bool(matches!(&args[0], Value::NativeObject(object) if object.get().type_tag() == "ReviewProbe")))))));
        let mut env = Env::new(runtime.globals().clone(), "user");
        let source = r#"
          (assert (= {:k "v"} (try (def broken (throw (ex-info "boom" {:k (str "v")}))) (catch Exception e (ex-data e)))))
          (assert (= {:k "attribute"} (try (defn broken-fn {:x (throw (ex-info "boom" {:k (str "attribute")}))} [] nil) (catch Exception e (ex-data e)))))
          (assert (= ["a"] (let [t (transient [])] (conj! t (str "a")) (persistent! t))))
          (assert (= {"key" "new"} (let [t (transient {})] (assoc! t (str "key") (str "old")) (assoc! t (str "key") (str "new")) (persistent! t))))
          (assert (= #{"a"} (let [t (transient #{})] (conj! t (str "a")) (conj! t (str "a")) (persistent! t))))
          (defmulti retained-method identity)
          (defn install-retained [label] (defmethod retained-method :x [_] label))
          (install-retained (str "method" " alive"))
          (assert (= "method alive" (retained-method :x)))
          (let [snapshot (methods retained-method)]
            (remove-method user/retained-method :x)
            (assert (= "method alive" ((get snapshot :x) :x))))
          (defmethod retained-method :x [_] (remove-method user/retained-method :x) (str "after" " removal"))
          (assert (= "after removal" (retained-method :x)))
          (def scalar-cache (atom nil))
          (let [text (str "identity" " stays") stored (reset! scalar-cache text)]
            (assert (identical? text stored)) (assert (identical? text @scalar-cache)))
          (defn churn-array []
            (let [array (object-array 1)]
              (dotimes [i 100] (aset array 0 (make-probe)) (assert (= i (drop-count)))))
            nil)
          (churn-array)
          (assert (= 100 (drop-count)))
          (def ^:dynamic *item* nil)
          (defn churn-binding []
            (binding [*item* nil]
              (dotimes [i 100] (set! *item* (make-probe)) (assert (= (+ 100 i) (drop-count)))))
            nil)
          (churn-binding)
          (assert (= 200 (drop-count)))
          (defn transient-alias []
            (let [t (transient {})]
              (assoc! t :x (make-probe))
              (let [old (get t :x)] (assoc! t :x nil) (assert (probe? old))))
            nil)
          (transient-alias)
          (assert (= 201 (drop-count)))
        "#;
        for form in cljrs_reader::Parser::new(source.into(), "<review-seven>".into()).parse_all().unwrap() {
            cljrs_runtime::interp::eval::eval_discard(&form, &mut env).unwrap();
        }
        let form = cljrs_reader::Parser::new("(vec (range 100))".into(), "<discarded>".into()).parse_one().unwrap().unwrap();
        cljrs_runtime::interp::eval::eval_discard(&form, &mut env).unwrap();
        let retired = || cljrs_gc::governor::with_current_account(|a| a.retired_region_bytes()).unwrap();
        let baseline = retired();
        for _ in 0..100 { cljrs_runtime::interp::eval::eval_discard(&form, &mut env).unwrap(); assert_eq!(retired(), baseline); }
    }).unwrap().join().unwrap();
}

#[test]
fn asynchronous_read_leases_survive_the_polling_forms_scope() {
    let _lock = TEST_LOCK.lock().unwrap();
    #[derive(Debug)]
    struct Probe(std::rc::Rc<std::cell::Cell<bool>>);
    impl cljrs_gc::Trace for Probe {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    }
    impl Drop for Probe {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    impl cljrs_value::NativeObject for Probe {
        fn type_tag(&self) -> &str {
            "ReadProbe"
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, async {
        use cljrs_value::Value;
        let caller = cljrs_gc::region::ReadScope::enter().unwrap();
        let atom = cljrs_gc::GcPtr::new(cljrs_value::Atom::new(Value::Nil));
        let dropped = std::rc::Rc::new(std::cell::Cell::new(false));
        {
            let _source = cljrs_gc::alloc_ctx::ScratchGuard::new();
            atom.get().reset(Value::NativeObject(cljrs_value::gc_native_object(Probe(dropped.clone()))));
        }
        let captured = atom.clone();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, wait) = tokio::sync::oneshot::channel();
        let future = cljrs_async::spawn_future_with_roots(vec![Value::Atom(atom.clone())], async move {
            let value = captured.get().deref();
            started.send(()).unwrap();
            wait.await.unwrap();
            assert!(matches!(value, Value::NativeObject(object) if object.get().type_tag() == "ReadProbe"));
            Ok(Value::Nil)
        });
        ready.await.unwrap();
        drop(caller);
        atom.get().reset(Value::Nil);
        assert!(!dropped.get(), "a suspended task borrowed its poller's read scope");
        release.send(()).unwrap();
        cljrs_async::await_value(future).await.unwrap();
        assert!(dropped.get());
    });
}
