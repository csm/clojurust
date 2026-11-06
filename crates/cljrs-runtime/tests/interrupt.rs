//! An installed interrupt flag stops evaluation at the next execution-credit
//! checkpoint, in every tier, and is reported through the uncatchable
//! `GasExhausted` path.

use std::sync::Arc;
use std::time::Duration;

use cljrs_reader::Parser;
use cljrs_runtime::env::error::EvalError;
use cljrs_runtime::env::gas::{InterruptFlag, InterruptGuard, interrupt_requested};

fn forms(src: &str) -> Vec<cljrs_reader::Form> {
    Parser::new(src.to_owned(), "<interrupt-test>".to_owned())
        .parse_all()
        .expect("parse")
}

fn env() -> cljrs_runtime::tiered::Env {
    cljrs_runtime::tiered::Env::new(
        cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
            .build()
            .expect("runtime")
            .into_globals(),
        "user",
    )
}

/// Evaluate `setup` then `call` on a fresh thread with an interrupt flag
/// installed, set the flag from this thread after a moment, and return the
/// call's result.
/// Results cross threads as a description (`EvalError` is not `Send`).
fn describe(result: Result<cljrs_value::Value, EvalError>) -> String {
    match result {
        Err(EvalError::GasExhausted) => "interrupted".to_string(),
        Err(other) => format!("error: {other}"),
        Ok(v) => format!("value: {v}"),
    }
}

fn interrupted_eval(setup: &'static str, call: &'static str) -> String {
    let flag = Arc::new(InterruptFlag::new(false));
    let worker_flag = flag.clone();
    let handle = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut env = env();
            for form in forms(setup) {
                cljrs_runtime::tiered::eval(&form, &mut env).expect("setup");
            }
            let _guard = InterruptGuard::install(worker_flag);
            let result = cljrs_runtime::tiered::eval(&forms(call).remove(0), &mut env);
            assert!(interrupt_requested());
            drop(_guard);
            assert!(!interrupt_requested());
            // The environment remains usable after the interrupt.
            let after = cljrs_runtime::tiered::eval(&forms("(+ 1 2)").remove(0), &mut env)
                .expect("eval after interrupt");
            assert_eq!(format!("{after}"), "3");
            describe(result)
        })
        .expect("spawn");
    std::thread::sleep(Duration::from_millis(200));
    flag.request();
    handle.join().expect("worker panicked")
}

#[test]
fn interrupt_stops_infinite_loop() {
    let result = interrupted_eval("", "(loop [] (recur))");
    assert_eq!(result, "interrupted");
}

#[test]
fn interrupt_stops_loop_in_function() {
    let result = interrupted_eval("(defn spin [] (loop [n 0] (recur (inc n))))", "(spin)");
    assert_eq!(result, "interrupted");
}

#[test]
fn interrupt_is_not_caught_by_try() {
    let result = interrupted_eval("", "(try (loop [] (recur)) (catch :default e :caught))");
    assert_eq!(result, "interrupted");
}

/// The flag is set while thousands of non-tail frames are live: the
/// interrupt must unwind through all of them.
#[test]
fn interrupt_unwinds_live_non_tail_recursion() {
    let result = interrupted_eval(
        "(defn sink [n] (if (= n 0) (loop [] (recur)) (+ 1 (sink (dec n)))))",
        "(sink 5000)",
    );
    assert_eq!(result, "interrupted");
}

#[test]
fn interrupt_stops_deep_non_tail_recursion() {
    let flag = Arc::new(InterruptFlag::new(true));
    let result = std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(move || {
            let mut env = env();
            for form in forms("(defn deep [n] (if (= n 0) 0 (+ 1 (deep (dec n)))))") {
                cljrs_runtime::tiered::eval(&form, &mut env).expect("defn");
            }
            let _guard = InterruptGuard::install(flag);
            describe(cljrs_runtime::tiered::eval(
                &forms("(deep 100000)").remove(0),
                &mut env,
            ))
        })
        .expect("spawn")
        .join()
        .expect("worker panicked");
    assert_eq!(result, "interrupted");
}

/// A flag left set by an interrupted evaluation must not stop the next one:
/// once its guard is dropped, evaluation runs normally, including after the
/// interrupted evaluation ended in an error.
#[test]
fn stale_flag_does_not_stop_next_eval() {
    let mut env = env();
    let flag = Arc::new(InterruptFlag::new(true));
    {
        let _guard = InterruptGuard::install(flag.clone());
        let result = cljrs_runtime::tiered::eval(&forms("(loop [] (recur))").remove(0), &mut env);
        assert_eq!(describe(result), "interrupted");
    }
    assert!(flag.is_set(), "the old flag stays set");
    {
        // A fresh job with its own (clear) flag, one that errors.
        let _guard = InterruptGuard::install(Arc::new(InterruptFlag::new(false)));
        let result =
            cljrs_runtime::tiered::eval(&forms("(throw (ex-info \"x\" {}))").remove(0), &mut env);
        assert!(describe(result).starts_with("error"));
    }
    let _guard = InterruptGuard::install(Arc::new(InterruptFlag::new(false)));
    let result = cljrs_runtime::tiered::eval(
        &forms("(loop [n 0] (if (< n 100000) (recur (inc n)) n))").remove(0),
        &mut env,
    );
    assert_eq!(describe(result), "value: 100000");
}
