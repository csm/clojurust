#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]
use cljrs_runtime::env::{env::Env, error::EvalError};
use cljrs_value::{Arity, NativeFn, Value, ValueError};

#[test]
fn thread_failures_reach_every_consumer_resource_limits_bypass_catch() {
    const CHILD: &str = "CLJRS_REVIEW_HANDLED_THREAD_LIMITS";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "thread_failures_reach_every_consumer_resource_limits_bypass_catch",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert!(
            !stderr.contains("[clojurust thread error]"),
            "handled failure was reported: {stderr}"
        );
        assert!(
            !stderr.contains("a failed future was discarded"),
            "internal delivery produced a duplicate warning: {stderr}"
        );
        return;
    }
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        for kind in ["gas", "memory", "cancelled"] {
            for read in ["(a/<!! ch)", "@(a/take! ch)", "(a/alts!! [ch])", "@(a/alts [(a/take! ch)])", "(do (sleep 1) (a/poll! ch))"] {
                let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
                runtime.globals().intern("user", "limit".into(), Value::NativeFunction(cljrs_gc::GcPtr::new(
                    NativeFn::with_closure("limit", Arity::Fixed(0), move |_| match kind {
                        "memory" => Err(ValueError::MemoryLimitExceeded(cljrs_gc::MemoryLimitExceeded {
                            class: cljrs_gc::MemoryClass::GcHeap, requested: 1, committed: 2, hard_limit: 2 })),
                        "cancelled" => Err(ValueError::Cancelled),
                        _ => Err(ValueError::GasExhausted),
                    }))));
                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                let local = tokio::task::LocalSet::new();
                local.block_on(&rt, async { cljrs_async::init(runtime.globals()); });
                let source = format!("(require '[clojure.core.async :as a]) (try (let [ch (a/thread (limit))] {read}) (catch Exception e :caught))");
                let forms = cljrs_reader::Parser::new(source, "<thread-limits>".into()).parse_all().unwrap();
                let result = cljrs_async::fiber::run_root_owned(&rt, local, || {
                    let mut env = Env::new(runtime.globals().clone(), "user");
                    let mut last = Value::Nil;
                    for form in &forms { last = cljrs_runtime::tiered::eval(form, &mut env)?; }
                    Ok(last)
                });
                let env = Env::new(runtime.globals().clone(), "user");
                cljrs_runtime::env::gc_roots::force_collect(&env);
                cljrs_runtime::env::gc_roots::force_collect(&env);
                match kind {
                    "memory" => assert!(matches!(result, Err(EvalError::MemoryLimitExceeded(_))), "{read}: {result:?}"),
                    // Cancellation reaches healthy readers as a catchable exception.
                    "cancelled" => assert!(
                        matches!(&result, Ok(Value::Keyword(k)) if k.get().name.as_ref() == "caught"),
                        "{read}: {result:?}"
                    ),
                    _ => assert!(matches!(result, Err(EvalError::GasExhausted)), "{read}: {result:?}"),
                }
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
        assert!(
            !stderr.contains("a failed future was discarded"),
            "duplicate delivery warning: {stderr}"
        );
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
            // Cancellation is deliberate and stays silent, and its report must
            // not allocate: the channel may be dropped inside a GC sweep.
            for kind in ["gas", "memory", "cancelled"] {
                let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
                runtime.globals().intern(
                    "user",
                    "limit".into(),
                    Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::with_closure(
                        "limit",
                        Arity::Fixed(0),
                        move |_| match kind {
                            "memory" => Err(ValueError::MemoryLimitExceeded(
                                cljrs_gc::MemoryLimitExceeded {
                                    class: cljrs_gc::MemoryClass::GcHeap,
                                    requested: 1,
                                    committed: 2,
                                    hard_limit: 2,
                                },
                            )),
                            "cancelled" => Err(ValueError::Cancelled),
                            _ => Err(ValueError::GasExhausted),
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
                    if kind == "cancelled" {
                        // A take! of a cancelled body, dropped unread, is silent too.
                        "(require '[clojure.core.async :as a]) (a/thread (limit)) (a/take! (a/thread (limit))) (sleep 1) nil"
                    } else {
                        "(require '[clojure.core.async :as a]) (a/thread (limit)) (sleep 1) nil"
                    }
                    .into(),
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
                let env = Env::new(runtime.globals().clone(), "user");
                cljrs_runtime::env::gc_roots::force_collect(&env);
                cljrs_runtime::env::gc_roots::force_collect(&env);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
