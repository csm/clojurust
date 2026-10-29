//! The Clojure blocking form offloads typed native calls without moving closures.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]
use cljrs_async::fiber;
use cljrs_interop::{Value, export};
use cljrs_runtime::env::{env::Env, error::EvalResult, gc_roots::push_env_root};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock, mpsc};
use std::time::Duration;

static COMPLETED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GATES: OnceLock<Mutex<HashMap<String, mpsc::Receiver<i64>>>> = OnceLock::new();
#[export(ns = "blocking-fixture")]
fn wait_gate(name: String) -> Result<String, String> {
    let receiver = GATES.get().unwrap().lock().unwrap().remove(&name).unwrap();
    let value = receiver
        .recv_timeout(Duration::from_secs(2))
        .map_err(|e| e.to_string())?;
    COMPLETED.fetch_add(1, std::sync::atomic::Ordering::Release);
    Ok(format!(
        "{value}:{}",
        std::thread::current().name().unwrap_or("unnamed")
    ))
}
#[export(ns = "blocking-fixture")]
fn fail_native() -> Result<i64, String> {
    Err("native error".into())
}
#[export(ns = "blocking-fixture")]
fn panic_native() -> i64 {
    panic!("native panic")
}
#[export(ns = "blocking-fixture")]
fn add_one(value: i64) -> i64 {
    value + 1
}
#[export(ns = "blocking-fixture")]
fn non_send(value: Value) -> i64 {
    panic!("managed input reached the worker: {value:?}")
}
fn eval(source: &str, env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in cljrs_reader::Parser::new(source.into(), "<blocking-form>".into())
        .parse_all()
        .unwrap()
    {
        result = cljrs_runtime::interp::eval::eval(&form, env)?;
    }
    Ok(result)
}

#[test]
fn blocking_native_calls_park_and_preserve_the_isolate_boundary() {
    std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
        for (index, mode) in [cljrs_runtime::ExecutionMode::TreeWalk, cljrs_runtime::ExecutionMode::Tiered].into_iter().enumerate() {
            let runtime = cljrs_runtime::Runtime::builder().execution_mode(mode).build().unwrap();
            let globals = runtime.globals().clone();
            let _registry = cljrs_interop::Registry::new(globals.clone());
            let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async {
                cljrs_async::init(&globals);
                let name = format!("gate-{index}");
                let (tx, rx) = mpsc::channel();
                GATES.get_or_init(Default::default).lock().unwrap().insert(name.clone(), rx);
                let call_globals = globals.clone();
                let task = fiber::spawn(move || {
                    let mut env = Env::new(call_globals, "user");
                    let _root = push_env_root(&env);
                    eval(&format!("(do (require '[cljrs.fiber :as f]) (f/blocking (blocking-fixture/wait-gate (str {:?}))))", name), &mut env)
                }).unwrap();
                let peer = fiber::spawn(move || { tx.send(42).unwrap(); Ok(Value::Nil) }).unwrap();
                assert_eq!(task.await.unwrap().into_result().unwrap(), Value::string("42:cljrs-worker"));
                peer.await.unwrap().into_result().unwrap();

                let cancel_name = format!("cancel-{index}");
                let (cancel_tx, cancel_rx) = mpsc::channel();
                let completed = COMPLETED.load(std::sync::atomic::Ordering::Acquire);
                GATES.get().unwrap().lock().unwrap().insert(cancel_name.clone(), cancel_rx);
                let cancel_globals = globals.clone();
                let source = format!("(blocking (blocking-fixture/wait-gate {:?}))", cancel_name);
                let cancelled = fiber::spawn(move || {
                    let mut env = Env::new(cancel_globals, "user");
                    let _root = push_env_root(&env);
                    eval(&source, &mut env)
                }).unwrap();
                let deadline = std::time::Instant::now() + Duration::from_secs(5);
                while GATES.get().unwrap().lock().unwrap().contains_key(&cancel_name) {
                    assert!(std::time::Instant::now() < deadline, "blocking job never started");
                    tokio::task::yield_now().await;
                }
                cancelled.abort();
                assert!(matches!(cancelled.await, Err(error) if error.is_cancelled()));
                cancel_tx.send(99).unwrap();
                while COMPLETED.load(std::sync::atomic::Ordering::Acquire) == completed {
                    assert!(std::time::Instant::now() < deadline, "detached native job did not finish");
                    tokio::task::yield_now().await;
                }

                let task = fiber::spawn(move || {
                    let mut env = Env::new(globals, "user");
                    let _root = push_env_root(&env);
                    eval(r#"
                      (let [x (atom 0) thunk (fn [] (swap! x inc))]
                        [(blocking (blocking-fixture/add-one (swap! x inc)))
                         @x
                         (blocking (blocking-fixture/add-one 1) (blocking-fixture/add-one 41))
                         (blocking)
                         (try (blocking (thunk)) (catch Exception _ :closure-rejected))
                         @x
                         (try (blocking (blocking-fixture/non-send "managed")) (catch Exception _ :managed-rejected))
                         (try (blocking (blocking-fixture/fail-native)) (catch Exception e (ex-message e)))
                         (try (blocking (blocking-fixture/panic-native)) (catch Exception _ :panic))])"#, &mut env)
                }).unwrap();
                assert_eq!(format!("{}", task.await.unwrap().into_result().unwrap()),
                    r#"[2 1 42 nil :closure-rejected 1 :managed-rejected "native error" :panic]"#);
            });
        }
    }).unwrap().join().unwrap();
}
