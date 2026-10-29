#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]
use cljrs_runtime::env::{env::Env, error::EvalError};
use cljrs_value::{Arity, NativeFn, Value, ValueError};

#[test]
fn thread_resource_failures_reach_every_consumer_and_bypass_catch() {
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        for memory in [false, true] {
            for read in ["(a/<!! ch)", "@(a/take! ch)", "(a/alts!! [ch])", "(do (sleep 1) (a/poll! ch))"] {
                let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
                runtime.globals().intern("user", "limit".into(), Value::NativeFunction(cljrs_gc::GcPtr::new(
                    NativeFn::with_closure("limit", Arity::Fixed(0), move |_| {
                        if memory { Err(ValueError::MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded {
                            class: cljrs_gc::MemoryClass::GcHeap, requested: 1, committed: 2, hard_limit: 2 })) }
                        else { Err(ValueError::GasExhausted) }
                    }))));
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async { cljrs_async::init(runtime.globals()); });
                let source = format!("(require '[clojure.core.async :as a]) (try (let [ch (a/thread (limit))] {read}) (catch Exception e :caught))");
                let forms = cljrs_reader::Parser::new(source, "<thread-limits>".into()).parse_all().unwrap();
                let result = cljrs_async::fiber::run_root_owned(&rt, local, || {
                    let mut env = Env::new(runtime.globals().clone(), "user");
                    for form in &forms { cljrs_runtime::tiered::eval(form, &mut env)?; }
                    Ok(Value::Nil)
                });
                if memory { assert!(matches!(result, Err(EvalError::MemoryLimitExceeded(_))), "{read}: {result:?}"); }
                else { assert!(matches!(result, Err(EvalError::GasExhausted)), "{read}: {result:?}"); }
            }
        }
    }).unwrap().join().unwrap();
}

#[test]
fn unread_thread_resource_errors_are_reported_without_a_collection() {
    const CHILD: &str = "CLJRS_REVIEW_UNREAD_THREAD_LIMITS";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unread_thread_resource_errors_are_reported_without_a_collection",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            stderr.matches("[clojurust thread error]").count(),
            2,
            "{stderr}"
        );
        return;
    }
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            for memory in [false, true] {
                let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
                runtime.globals().intern(
                    "user",
                    "limit".into(),
                    Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::with_closure(
                        "limit",
                        Arity::Fixed(0),
                        move |_| {
                            if memory {
                                Err(ValueError::MemoryLimitExceeded(
                                    cljrs_gc::MemoryLimitExceeded {
                                        class: cljrs_gc::MemoryClass::GcHeap,
                                        requested: 1,
                                        committed: 2,
                                        hard_limit: 2,
                                    },
                                ))
                            } else {
                                Err(ValueError::GasExhausted)
                            }
                        },
                    ))),
                );
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async {
                    cljrs_async::init(runtime.globals());
                });
                let forms = cljrs_reader::Parser::new(
                    "(require '[clojure.core.async :as a]) (a/thread (limit)) (sleep 1) nil".into(),
                    "<unread-thread-limit>".into(),
                )
                .parse_all()
                .unwrap();
                cljrs_async::fiber::run_root_owned(&rt, local, || {
                    let mut env = Env::new(runtime.globals().clone(), "user");
                    for form in &forms {
                        cljrs_runtime::tiered::eval(form, &mut env)?;
                    }
                    Ok(Value::Nil)
                })
                .unwrap();
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
