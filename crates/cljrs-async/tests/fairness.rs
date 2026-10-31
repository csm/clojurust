#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cljrs_async::fiber::{self, Fiber};
use cljrs_gc::{GcPtr, MemoryClass};
use cljrs_runtime::env::{env::Env, error::EvalError};
use cljrs_value::{Arity, NativeFn, Value};

#[test]
fn configured_stacks_reject_unsafe_sizes_before_execution() {
    const CHILD: &str = "CLJRS_REVIEW_STACK_MINIMUM";
    if std::env::var_os(CHILD).is_none() {
        for size in ["64", "128", "256", "512"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "configured_stacks_reject_unsafe_sizes_before_execution",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .env("CLJRS_FIBER_STACK_KB", size)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{size} KiB: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    }
    if std::env::var("CLJRS_FIBER_STACK_KB").unwrap() != "512" {
        let error = Fiber::new(|| Ok(Value::Nil))
            .err()
            .expect("unsafe stack accepted");
        assert!(error.to_string().contains("at least 512"), "{error:?}");
        return;
    }
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let form = cljrs_reader::Parser::new("(+ 1 2)".into(), "<small-stack>".into())
                .parse_all()
                .unwrap()
                .remove(0);
            run(async {
                let task = fiber::spawn(move || {
                    cljrs_runtime::tiered::eval(
                        &form,
                        &mut Env::new(runtime.globals().clone(), "user"),
                    )
                })
                .unwrap();
                assert_eq!(task.await.unwrap().into_result().unwrap(), Value::Long(3));
            });
        })
        .unwrap()
        .join()
        .unwrap();
}

fn run(body: impl std::future::Future<Output = ()>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&rt, body);
}

#[test]
fn cpu_loops_yield_in_both_interpreters_and_preserve_roots() {
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
                let ready = Rc::new(Cell::new(false));
                let observed = ready.clone();
                let deadline = Instant::now() + Duration::from_secs(3);
                globals.intern(
                    "user",
                    "stop?".into(),
                    Value::NativeFunction(GcPtr::new(NativeFn::with_closure(
                        "stop?",
                        Arity::Fixed(0),
                        move |_| Ok(Value::Bool(observed.get() || Instant::now() >= deadline)),
                    ))),
                );
                let form = cljrs_reader::Parser::new(
                    "(let [saved (str \"live-\" 42)] (loop [] (if (stop?) saved (recur))))".into(),
                    "<fairness>".into(),
                )
                .parse_all()
                .unwrap()
                .remove(0);
                run(async {
                    let spinner_globals = globals.clone();
                    let spinner = fiber::spawn(move || {
                        cljrs_runtime::tiered::eval(&form, &mut Env::new(spinner_globals, "user"))
                    })
                    .unwrap();
                    let signal = ready.clone();
                    let collector = fiber::spawn(move || {
                        let env = Env::new(globals, "user");
                        cljrs_runtime::env::gc_roots::force_collect(&env);
                        signal.set(true);
                        Ok(Value::Nil)
                    })
                    .unwrap();
                    let result = spinner.await.unwrap().into_result().unwrap();
                    assert!(
                        ready.get(),
                        "CPU loop ran to its deadline without yielding: {mode:?}"
                    );
                    assert_eq!(result, Value::string("live-42"));
                    collector.await.unwrap().into_result().unwrap();
                });
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn stack_charges_cover_pool_reuse_completion_and_cancellation() {
    // The process-wide governor must not race with other tests in this binary.
    const CHILD: &str = "CLJRS_PHASE4_ACCOUNTING";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "stack_charges_cover_pool_reuse_completion_and_cancellation",
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
        return;
    }
    let governor = cljrs_gc::governor();
    let baseline = governor.snapshot();
    let fiber = Fiber::new(|| Ok(Value::Nil)).unwrap();
    let allocated = governor.snapshot();
    assert_eq!(allocated.fiber_count, baseline.fiber_count + 1);
    let bytes = allocated.class_bytes(MemoryClass::FiberStack);
    assert!(bytes >= 1024 * 1024);
    drop(fiber);
    let pooled = governor.snapshot();
    assert_eq!(pooled.fiber_count, baseline.fiber_count);
    assert_eq!(pooled.pooled_fiber_stacks, baseline.pooled_fiber_stacks + 1);
    assert_eq!(pooled.class_bytes(MemoryClass::FiberStack), bytes);
    run(async {
        let task = fiber::spawn(|| {
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(governor.class_bytes(MemoryClass::FiberStack), bytes);
        task.abort();
        assert!(task.await.err().unwrap().is_cancelled());
        let task = fiber::spawn(|| Ok(Value::Long(42))).unwrap();
        assert_eq!(task.await.unwrap().into_result().unwrap(), Value::Long(42));
    });
    assert_eq!(governor.snapshot().fiber_count, baseline.fiber_count);
    assert!(cljrs_gc::stats_report().contains("reserved stack bytes"));
    let mut config = cljrs_gc::MemoryConfig::with_limits(512 * 1024, 768 * 1024).unwrap();
    config.strict = true;
    governor.configure(config).unwrap();
    // The idle stack is reclaimed before refusing another reservation.
    // Request a different size so this cannot reuse an already charged stack.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    let mut carrier = cljrs_gc::mutator::MutatorState::fresh();
    let error = fiber::run_root(&rt, &local, &mut carrier, || Ok(Value::Nil)).unwrap_err();
    assert!(
        matches!(error, EvalError::MemoryLimitExceeded(ref fault) if fault.class == MemoryClass::FiberStack),
        "{error:?}"
    );
    assert_eq!(governor.class_bytes(MemoryClass::FiberStack), 0);
    assert_eq!(governor.snapshot().pooled_fiber_stacks, 0);
}

#[test]
fn watchdog_reports_a_blocked_native_frame_once() {
    const CHILD: &str = "CLJRS_PHASE4_STALL";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "watchdog_reports_a_blocked_native_frame_once",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("CLJRS_FIBER_STALL_MS", "10")
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        assert_eq!(stderr.matches("stalled its isolate").count(), 1, "{stderr}");
        assert!(
            stderr.contains("native frame: blocking-test-native"),
            "{stderr}"
        );
        return;
    }
    run(async {
        let blocker = fiber::spawn(|| {
            let _native = cljrs_gc::fiber::native_frame(&"blocking-test-native".into());
            std::thread::sleep(Duration::from_millis(150));
            Ok(Value::Nil)
        })
        .unwrap();
        let sibling = fiber::spawn(|| Ok(Value::Nil)).unwrap();
        blocker.await.unwrap().into_result().unwrap();
        sibling.await.unwrap().into_result().unwrap();
        // Parked and unstarted fibers do not make another fiber runnable.
        let unstarted = Fiber::new(|| Ok(Value::Nil)).unwrap();
        let sleeper = fiber::spawn(|| {
            fiber::park_on(tokio::time::sleep(Duration::from_millis(50)));
            Ok(Value::Nil)
        })
        .unwrap();
        let lone = fiber::spawn(|| {
            std::thread::sleep(Duration::from_millis(50));
            Ok(Value::Nil)
        })
        .unwrap();
        sleeper.await.unwrap().into_result().unwrap();
        lone.await.unwrap().into_result().unwrap();
        drop(unstarted);
    });
}

#[test]
fn recursion_reports_the_fiber_before_reaching_its_guard_page() {
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        for mode in [cljrs_runtime::ExecutionMode::TreeWalk, cljrs_runtime::ExecutionMode::Tiered] {
            let runtime = cljrs_runtime::Runtime::builder().execution_mode(mode).build().unwrap();
            let form = cljrs_reader::Parser::new(
                "(do (defn overflow [] (+ 1 (overflow))) (overflow))".into(), "<overflow>".into())
                .parse_all().unwrap().remove(0);
            run(async {
                let task = Fiber::new(move || cljrs_runtime::tiered::eval(&form,
                    &mut Env::new(runtime.globals().clone(), "user"))).unwrap();
                let id = task.id();
                let error = task.into_task().await.unwrap_err();
                assert!(matches!(&error, EvalError::Runtime(message) if message.contains("stack overflow in fiber")), "{id:?}: {error:?}");
            });
        }
    }).unwrap().join().unwrap();
}

#[test]
fn idle_stacks_are_reclaimed_for_other_classes_across_carriers() {
    const CHILD: &str = "CLJRS_IDLE_STACK_RECLAIM";
    let Ok(mode) = std::env::var(CHILD) else {
        for mode in ["child", "root"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "idle_stacks_are_reclaimed_for_other_classes_across_carriers",
                    "--nocapture",
                ])
                .env(CHILD, mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{mode}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        return;
    };
    let mib = 1024 * 1024;
    let (soft, hard, requested) = if mode == "root" {
        (120 * mib, 128 * mib, 96 * mib)
    } else {
        (1800 * 1024, 2 * mib, 1500 * 1024)
    };
    let governor = cljrs_gc::governor();
    let mut config = cljrs_gc::MemoryConfig::with_limits(soft, hard).unwrap();
    config.strict = true;
    governor.configure(config).unwrap();
    let root = mode == "root";
    std::thread::spawn(move || {
        if root {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            let mut carrier = cljrs_gc::mutator::MutatorState::fresh();
            fiber::run_root(&rt, &local, &mut carrier, || Ok(Value::Nil)).unwrap();
        } else {
            drop(Fiber::new(|| Ok(Value::Nil)).unwrap());
        }
    })
    .join()
    .unwrap();
    assert_eq!(governor.snapshot().pooled_fiber_stacks, 1);
    assert!(governor.class_bytes(MemoryClass::FiberStack) >= if root { 64 * mib } else { mib });
    let code = governor
        .reserve_shared(MemoryClass::Code, requested)
        .unwrap();
    assert_eq!(governor.class_bytes(MemoryClass::FiberStack), 0);
    assert_eq!(governor.snapshot().pooled_fiber_stacks, 0);
    assert_eq!(code.bytes(), requested);
}
