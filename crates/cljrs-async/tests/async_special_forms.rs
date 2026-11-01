//! Regression tests for special forms that evaluate an `await` in place
//! (issue #398, the follow-up to the `recur` fix in `async_recur.rs`).
//!
//! Before the fix every special form without an arm in `eval_async` was
//! delegated to the synchronous evaluator, which evaluated its sub-expressions
//! on the blocking-deref path. That parks the one `LocalSet` thread the awaited
//! future needs in order to settle, so the program deadlocked silently.
//!
//! Each case awaits a take against a *concurrent* producer over a one-slot
//! channel, so the take genuinely pends instead of finding a buffered value.
//! A deadlock stalls a test binary instead of failing it, so every case runs on
//! a watchdog thread and the assertion is on `recv_timeout`
//! (`tokio::time::timeout` cannot catch it: the parked thread is the one that
//! would poll the timer).
//!
//! Since virtual-threads Phase 5 these cases run on fibers (`fiber::eval_form`);
//! the colored `eval_async` evaluator they first targeted is wasm32-only.

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use cljrs_async::fiber::eval_form;
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

/// Evaluate every form in `src` with the async evaluator on a private
/// current-thread runtime, returning the printed last value, or `None` if the
/// worker did not answer within [`WATCHDOG`].
fn eval_async_src(src: &str) -> Option<String> {
    let (tx, rx) = mpsc::channel::<String>();
    let src = src.to_string();
    // Detached on purpose: a deadlocked worker never joins.
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
                result = eval_form(&form, &mut env).await.expect("eval error");
            }
            format!("{result}")
        });
        let _ = tx.send(out);
    });
    rx.recv_timeout(WATCHDOG).ok()
}

/// The issue's common prelude: a one-slot channel fed by a concurrent producer.
const PRELUDE: &str = "(require '[clojure.core.async :refer [chan take! put! go]])
                       (def ch (chan 1))
                       (go (await (put! ch 42)))";

fn run(body: &str) -> Option<String> {
    eval_async_src(&format!("{PRELUDE}\n{body}"))
}

#[test]
fn def_value_awaits() {
    assert_eq!(
        run("(def x (await (take! ch))) x").as_deref(),
        Some("42"),
        "def hung"
    );
}

#[test]
fn def_with_docstring_and_meta_keeps_both() {
    let out = run("(def ^:private x \"the doc\" (await (take! ch)))
                   [x (:doc (meta #'x)) (:private (meta #'x))]");
    assert_eq!(out.as_deref(), Some("[42 \"the doc\" true]"));
}

#[test]
fn defonce_value_awaits() {
    assert_eq!(
        run("(defonce y (await (take! ch))) y").as_deref(),
        Some("42"),
        "defonce hung"
    );
}

/// An already-bound `defonce` must not evaluate its value at all — here that
/// would block on a take nobody feeds.
#[test]
fn defonce_skips_value_when_bound() {
    let out = run("(defonce y 1)
                   (defonce y (await (take! (chan))))
                   y");
    assert_eq!(out.as_deref(), Some("1"));
}

#[test]
fn and_operand_awaits() {
    assert_eq!(
        run("(and true (await (take! ch)))").as_deref(),
        Some("42"),
        "and hung"
    );
}

#[test]
fn and_short_circuits() {
    assert_eq!(
        run("(and false (await (take! (chan))))").as_deref(),
        Some("false")
    );
}

#[test]
fn or_operand_awaits() {
    assert_eq!(
        run("(or false (await (take! ch)))").as_deref(),
        Some("42"),
        "or hung"
    );
}

#[test]
fn or_short_circuits() {
    assert_eq!(
        run("(or :first (await (take! (chan))))").as_deref(),
        Some(":first")
    );
}

#[test]
fn throw_argument_awaits() {
    let out = run("(try (throw (ex-info \"x\" {:v (await (take! ch))}))
                     (catch Exception e (:v (ex-data e))))");
    assert_eq!(out.as_deref(), Some("42"), "throw hung");
}

/// A non-error thrown value is still wrapped, as in the sync `throw`.
#[test]
fn throw_wraps_non_error_values() {
    let out = run("(try (throw (await (take! ch)))
                     (catch Exception e (ex-message e)))");
    assert_eq!(out.as_deref(), Some("\"42\""));
}

#[test]
fn binding_init_awaits() {
    let out = run("(def ^:dynamic *d* nil)
                   (binding [*d* (await (take! ch))] *d*)");
    assert_eq!(out.as_deref(), Some("42"), "binding hung");
}

/// The binding is visible after the body resumes from a yield, `set!` inside
/// the body survives the yield, and the frame is gone once the body is done.
#[test]
fn binding_survives_a_yield_in_the_body() {
    let out = run("(def ^:dynamic *d* :root)
                   (def seen
                     (binding [*d* 1]
                       (set! *d* 2)
                       (let [v (await (take! ch))]
                         [*d* v])))
                   [seen *d*]");
    assert_eq!(out.as_deref(), Some("[[2 42] :root]"));
}

/// A `go` block conveys the dynamic bindings in effect where it starts, as in
/// core.async (virtual-threads plan Phase 5: `go` bodies are fibers, which
/// convey bindings like `future`).
#[test]
fn go_conveys_bindings_into_its_fiber() {
    let out = run("(def ^:dynamic *d* :root)
                   (def seen (chan 1))
                   (binding [*d* :bound]
                     (go (await (put! seen *d*)))
                     [(await (take! ch)) *d* (await (take! seen))])");
    assert_eq!(out.as_deref(), Some("[42 :bound :bound]"));
}

/// A task started outside a binding must not see a frame that another fiber
/// installed while parked inside its `binding` body.
#[test]
fn binding_does_not_leak_into_other_tasks() {
    let out = run("(def ^:dynamic *d* :root)
                   (def seen (chan 1))
                   (def gate (chan 1))
                   (go (await (take! gate)) (await (put! seen *d*)))
                   (binding [*d* :bound]
                     (await (put! gate :go))
                     [(await (take! ch)) *d* (await (take! seen))])");
    assert_eq!(out.as_deref(), Some("[42 :bound :root]"));
}

#[test]
fn letfn_body_awaits() {
    assert_eq!(
        run("(letfn [(f [] 1)] (+ (f) (await (take! ch))))").as_deref(),
        Some("43"),
        "letfn hung"
    );
}

#[test]
fn with_out_str_body_awaits() {
    assert_eq!(
        run("(with-out-str (print (await (take! ch))))").as_deref(),
        Some("\"42\""),
        "with-out-str hung"
    );
}

/// Output printed before and after a yield is captured; output printed by
/// another task while this one is suspended is not.
#[test]
fn with_out_str_captures_only_its_own_task() {
    let out = run("(def done (chan 1))
                   (with-out-str
                     (print \"a\")
                     (go (print \"-other-\") (await (put! done :ok)))
                     (await (take! done))
                     (print (await (take! ch))))");
    assert_eq!(out.as_deref(), Some("\"a42\""));
}

#[test]
fn set_bang_value_awaits() {
    let out = run("(def ^:dynamic *e* nil)
                   (set! *e* (await (take! ch)))
                   *e*");
    assert_eq!(out.as_deref(), Some("42"), "set! hung");
}
