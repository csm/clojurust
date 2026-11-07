//! The background GC service must let an idle executor suspend, yet still
//! collect promptly when a collection is requested (CLJ-ASYNC-004).
//!
//! Earlier the service was a `loop { yield_now().await; async_gc_collect(); }`
//! task: it woke the executor on every poll even with no request pending, so a
//! LocalSet with nothing else to do never went idle. These tests initialize the
//! async runtime *inside* a LocalSet, so the service is running, and check:
//!
//! * an idle executor is not woken (counting waker, and paused-clock
//!   auto-advance, which tokio performs only when no task is ready);
//! * a same-thread or cross-thread request wakes the service and collects;
//! * a governor credit recall is still serviced while idle;
//! * values held by a parked `go` block survive a service collection;
//! * repeated initialization and LocalSet shutdown leave no stale wakers.
#![cfg(all(not(target_arch = "wasm32"), not(feature = "no-gc")))]
#![allow(clippy::result_large_err)]

use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::gc_roots::push_env_root;
use cljrs_value::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::task::{Context, Wake, Waker};
use std::time::Duration;

/// Real-time budget for a whole test.
const WATCHDOG: Duration = Duration::from_secs(60);

/// Run `test` on a fresh thread (its own isolate) with a large stack, failing
/// if it does not finish within [`WATCHDOG`].
fn on_thread(test: impl FnOnce() + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            test();
            let _ = done.send(());
        })
        .unwrap();
    match finished.recv_timeout(WATCHDOG) {
        Ok(()) => handle.join().unwrap(),
        Err(RecvTimeoutError::Disconnected) => {
            if let Err(panic) = handle.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            panic!("test did not finish within {WATCHDOG:?}: the executor never went idle")
        }
    }
}

fn globals() -> Arc<GlobalEnv> {
    cljrs_runtime::Runtime::builder()
        .build()
        .unwrap()
        .globals()
        .clone()
}

/// Run `test` inside a LocalSet on a paused-clock current-thread runtime, with
/// the async runtime (and so the GC service) initialized inside it.
fn run_paused<F: Future<Output = ()>>(test: impl FnOnce(Env) -> F + Send + 'static) {
    on_thread(move || {
        let globals = globals();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            cljrs_async::init(&globals);
            let env = Env::new(globals, "user");
            let _root = push_env_root(&env);
            test(env).await;
        });
    });
}

/// Run `test` inside a LocalSet on a real-time current-thread runtime, with
/// the async runtime initialized inside it.
fn run_real<F: Future<Output = ()>>(test: impl FnOnce(Env) -> F + Send + 'static) {
    on_thread(move || {
        let globals = globals();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        tokio::task::LocalSet::new().block_on(&rt, async {
            cljrs_async::init(&globals);
            let env = Env::new(globals, "user");
            let _root = push_env_root(&env);
            test(env).await;
        });
    });
}

/// Wait one virtual hour. Completes only when every other task is parked.
async fn parked() {
    tokio::time::sleep(Duration::from_secs(3600)).await;
}

fn control() -> Arc<cljrs_gc::IsolateControl> {
    cljrs_gc::governor::current_control().expect("isolate control")
}

/// Poll real time until `done` holds, failing after ten seconds.
async fn eventually(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
}

fn eval(env: &mut Env, source: &str) -> Value {
    let source = format!("(require '[clojure.core.async :as a]) {source}");
    let forms = cljrs_reader::Parser::new(source, "<gc-service>".into())
        .parse_all()
        .unwrap();
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).unwrap();
    }
    result
}

struct CountingWaker(AtomicUsize);

impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

/// Deterministic reproducer: drive a LocalSet whose root future never
/// completes with a counting waker, and count the wakes the executor raises on
/// itself once setup is over. The spinning service woke on every poll.
#[test]
fn idle_executor_does_not_wake_itself() {
    on_thread(|| {
        let globals = globals();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let _enter = rt.enter();
        let local = tokio::task::LocalSet::new();
        let mut root: Pin<Box<dyn Future<Output = ()>>> = Box::pin(local.run_until(async move {
            cljrs_async::init(&globals);
            std::future::pending::<()>().await;
        }));
        let counter = Arc::new(CountingWaker(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);

        // Setup: initialization and the service's first registration.
        for _ in 0..4 {
            assert!(root.as_mut().poll(&mut cx).is_pending());
        }
        let before = counter.0.load(Ordering::SeqCst);
        let mut self_waking = 0;
        for _ in 0..16 {
            let wakes = counter.0.load(Ordering::SeqCst);
            assert!(root.as_mut().poll(&mut cx).is_pending());
            if counter.0.load(Ordering::SeqCst) > wakes {
                self_waking += 1;
            }
        }
        assert_eq!(self_waking, 0, "idle executor woke itself");
        assert_eq!(counter.0.load(Ordering::SeqCst), before);

        // A request from another thread wakes the idle executor, and the next
        // poll collects.
        let ctl = control();
        let epoch = ctl.collection_epoch();
        let remote = ctl.clone();
        std::thread::spawn(move || remote.request_collection())
            .join()
            .unwrap();
        assert!(
            counter.0.load(Ordering::SeqCst) > before,
            "cross-thread request did not wake the executor"
        );
        assert!(root.as_mut().poll(&mut cx).is_pending());
        assert!(ctl.collection_epoch() > epoch, "no collection after wake");
        assert!(!ctl.collection_requested());
    });
}

#[test]
fn idle_executor_lets_virtual_time_advance() {
    run_paused(|_| async move {
        // Would hang (and trip the watchdog) if the service stayed runnable.
        parked().await;
        parked().await;
        assert_eq!(control().service_waker_count(), 1);
    });
}

#[test]
fn same_thread_request_collects_without_other_work() {
    run_paused(|_| async move {
        parked().await;
        let ctl = control();
        let epoch = ctl.collection_epoch();
        cljrs_gc::request_gc();
        parked().await;
        assert!(ctl.collection_epoch() > epoch, "request was not serviced");
        assert!(!cljrs_gc::gc_requested());
        // Back to idle: the service suspended again.
        parked().await;
        assert_eq!(ctl.service_waker_count(), 1);
    });
}

#[test]
fn cross_thread_request_wakes_the_service() {
    run_real(|_| async move {
        let ctl = control();
        let epoch = ctl.collection_epoch();
        let remote = ctl.clone();
        let requester = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            remote.request_collection();
        });
        eventually("cross-thread collection", || ctl.collection_epoch() > epoch).await;
        requester.join().unwrap();
        assert!(!ctl.collection_requested());
    });
}

#[test]
fn credit_recall_is_serviced_while_idle() {
    run_real(|_| async move {
        let ctl = control();
        let remote = ctl.clone();
        std::thread::spawn(move || remote.request_recall())
            .join()
            .unwrap();
        eventually("recall serviced", || !ctl.recall_requested()).await;
    });
}

#[test]
fn parked_go_block_roots_survive_service_collection() {
    run_paused(|mut env| async move {
        eval(
            &mut env,
            "(def c (a/chan))
             (def r (a/go (let [v (mapv #(str \"item-\" %) (range 200))]
                            (a/<! c)
                            (apply str (interpose \",\" v)))))",
        );
        parked().await;
        let ctl = control();
        let epoch = ctl.collection_epoch();
        // Allocate garbage so the collection has something to free.
        eval(&mut env, "(dotimes [_ 50] (vec (range 100)))");
        cljrs_gc::request_gc();
        parked().await;
        assert!(ctl.collection_epoch() > epoch, "request was not serviced");

        eval(&mut env, "(a/put! c :go)");
        let result = cljrs_async::await_value(eval(&mut env, "r")).await.unwrap();
        let expected = (0..200)
            .map(|i| format!("item-{i}"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(result, Value::string(expected));
    });
}

#[test]
fn repeated_init_and_shutdown_leave_no_stale_wakers() {
    on_thread(|| {
        let globals = globals();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .start_paused(true)
            .build()
            .unwrap();
        let ctl = control();
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async {
            cljrs_async::init(&globals);
            cljrs_async::init(&globals);
            parked().await;
            assert_eq!(ctl.service_waker_count(), 2);
            let epoch = ctl.collection_epoch();
            cljrs_gc::request_gc();
            parked().await;
            assert!(ctl.collection_epoch() > epoch, "request was not serviced");
            // Both services suspended again; neither spins.
            parked().await;
            assert_eq!(ctl.service_waker_count(), 2);
        });
        drop(local);
        assert_eq!(ctl.service_waker_count(), 0, "dropped services left wakers");

        // A later request with no service is kept for the next safepoint.
        cljrs_gc::request_gc();
        assert!(cljrs_gc::gc_requested());
        assert!(cljrs_gc::take_gc_request());

        // A fresh LocalSet on the same thread starts a working service.
        tokio::task::LocalSet::new().block_on(&rt, async {
            cljrs_async::init(&globals);
            parked().await;
            assert_eq!(ctl.service_waker_count(), 1);
            let epoch = ctl.collection_epoch();
            cljrs_gc::request_gc();
            parked().await;
            assert!(ctl.collection_epoch() > epoch, "request was not serviced");
        });
        assert_eq!(ctl.service_waker_count(), 0);
    });
}
