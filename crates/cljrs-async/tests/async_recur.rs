//! Regression tests for `recur` whose *arguments* await (CLJRS-ASYNC-RECUR).
//!
//! `(recur (conj acc (<? ch)) (inc i))` is the read-until-delimiter shape: take
//! a chunk, append, test, go again. Before the fix `recur` had no arm in
//! `eval_async`, so it fell through to the synchronous `eval_recur`, which
//! evaluated the recur arguments on the blocking-deref path. That parks the one
//! LocalSet thread the awaited future needs in order to settle, so the program
//! deadlocks silently rather than failing.
//!
//! A deadlock stalls a test binary instead of failing it, so every case here
//! runs on a watchdog thread and the assertion is on `recv_timeout`.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use cljrs_async::eval_async::eval_async;
use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

/// Generous enough that a slow CI box never trips it, short enough that a real
/// deadlock is reported rather than waited on.
const WATCHDOG: Duration = Duration::from_secs(20);

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals
}

/// Evaluate every form in `src` with the async evaluator, printing the last
/// value the way Clojure would.
///
/// Runs on its own OS thread with a private current-thread runtime and
/// `LocalSet`; the result is delivered over a channel so the caller can apply a
/// watchdog. Returns `None` if the thread did not answer within [`WATCHDOG`],
/// which is how the pre-fix deadlock surfaces.
fn eval_async_src(src: &str) -> Option<String> {
    let (tx, rx) = mpsc::channel::<String>();
    let src = src.to_string();
    // Detached on purpose: a deadlocked worker never joins, and the test must
    // report that rather than hang with it.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("build runtime");
        let local = tokio::task::LocalSet::new();
        let out = local.block_on(&rt, async {
            let globals = async_env();
            let mut env = Env::new(globals, "user");
            let mut parser = Parser::new(src, "<test>".to_string());
            let mut result = Value::Nil;
            for form in parser.parse_all().expect("parse error") {
                result = eval_async(&form, &mut env).await.expect("eval error");
            }
            format!("{result}")
        });
        let _ = tx.send(out);
    });
    rx.recv_timeout(WATCHDOG).ok()
}

const PRELUDE: &str = "(require '[clojure.core.async :refer [chan take! put! close! go]])";

/// The card's minimal case: a `loop`/`recur` whose recur argument awaits a
/// buffered take. Deadlocks without an async `recur`.
#[test]
fn loop_recur_with_await_in_a_recur_argument() {
    let out = eval_async_src(&format!(
        "{PRELUDE}
         (def ch (chan 4))
         (await (put! ch 1))
         (await (put! ch 2))
         (loop [acc [] i 0]
           (if (>= i 2)
             acc
             (recur (conj acc (await (take! ch))) (inc i))))"
    ));
    assert_eq!(out.as_deref(), Some("[1 2]"), "recur across an await hung");
}

/// The same defect reached through the *function* recur target rather than a
/// loop header: `run_async_fn`'s trampoline, not `eval_loop_async`'s.
#[test]
fn fn_recur_with_await_in_a_recur_argument() {
    let out = eval_async_src(&format!(
        "{PRELUDE}
         (def ch (chan 4))
         (defn ^:async take-n [c acc i]
           (if (>= i 2)
             acc
             (recur c (conj acc (await (take! c))) (inc i))))
         (await (put! ch 7))
         (await (put! ch 8))
         (await (take-n ch [] 0))"
    ));
    assert_eq!(
        out.as_deref(),
        Some("[7 8]"),
        "fn recur across an await hung"
    );
}

/// The same recur position, but against a *concurrent* producer over a
/// one-slot channel, so each take genuinely pends instead of finding a value
/// already buffered: the recur target has to survive a real suspend/resume, not
/// just a future that was already settled.
#[test]
fn recur_argument_awaits_a_concurrent_producer() {
    let out = eval_async_src(&format!(
        "{PRELUDE}
         (def ch (chan 1))
         (go (loop [i 0]
               (when (< i 4)
                 (await (put! ch i))
                 (recur (inc i)))))
         (loop [acc [] i 0]
           (if (>= i 4)
             acc
             (recur (conj acc (await (take! ch))) (inc i))))"
    ));
    assert_eq!(
        out.as_deref(),
        Some("[0 1 2 3]"),
        "read-until-delimiter hung"
    );
}

/// Recur arity is still checked when the arguments are evaluated
/// asynchronously: the async arm must not become a hole in the loop contract.
#[test]
fn async_recur_still_enforces_loop_arity() {
    let out = eval_async_src(&format!(
        "{PRELUDE}
         (def ch (chan 1))
         (await (put! ch 1))
         (try
           (loop [acc [] i 0]
             (recur (conj acc (await (take! ch)))))
           (catch Exception e :arity-error))"
    ));
    assert_eq!(out.as_deref(), Some(":arity-error"));
}
