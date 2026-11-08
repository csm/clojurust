//! `onto-chan!`, `to-chan!` and `mult` must park on channel events instead of
//! busy-polling (issue #457), and must resume, deliver and stop correctly.
//!
//! Each test runs on a current-thread runtime with a paused clock. Tokio
//! auto-advances paused time only when no task is ready to run, so
//! [`parked`] (a one-hour virtual sleep) completes only if every helper task
//! is parked. A `yield_now` polling loop keeps the executor permanently ready:
//! virtual time never advances and the real-time watchdog in [`run`] fails the
//! test instead of letting it hang.
//!
//! `cljrs_async::init` is called before any tokio runtime exists, so the
//! background GC-service task is not started. The service now suspends while
//! idle (see `gc_service.rs`), but keeping it out isolates the helpers.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use cljrs_async::channel::{CljChannel, as_channel};
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::gc_roots::push_env_root;
use cljrs_value::Value;
use futures_util::FutureExt;
use std::future::Future;
use std::sync::Arc;
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;

/// Real-time budget for a whole test. Every wait inside is on virtual time,
/// so a healthy test finishes in milliseconds.
const WATCHDOG: Duration = Duration::from_secs(30);

fn run<F: Future<Output = ()>>(test: impl FnOnce(Env) -> F + Send + 'static) {
    let (done, finished) = std::sync::mpsc::channel();
    let handle = std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let globals: Arc<GlobalEnv> = runtime.globals().clone();
            // No tokio runtime is current yet: init skips the GC service.
            cljrs_async::init(&globals);
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .start_paused(true)
                .build()
                .unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async {
                let env = Env::new(globals, "user");
                let _root = push_env_root(&env);
                test(env).await;
            });
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
            panic!("executor never went idle within {WATCHDOG:?}: a channel helper is busy-polling")
        }
    }
}

/// Evaluate `source` in `user` (with `a` aliased to core.async).
fn eval(env: &mut Env, source: &str) -> Value {
    let source = format!("(require '[clojure.core.async :as a]) {source}");
    let forms = cljrs_reader::Parser::new(source, "<helper-parking>".into())
        .parse_all()
        .unwrap();
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env).unwrap();
    }
    result
}

/// Wait one virtual hour. Completes only when every other task is parked.
async fn parked() {
    tokio::time::sleep(Duration::from_secs(3600)).await;
}

/// Look up a channel held by a `def`'d var (vars keep it rooted).
fn chan(env: &mut Env, name: &str) -> Value {
    let value = eval(env, name);
    assert!(as_channel(&value).is_some(), "{name} is not a channel");
    value
}

fn ch(value: &Value) -> &CljChannel {
    as_channel(value).unwrap()
}

/// The `Future` returned by `onto-chan!` is still unresolved.
fn assert_pending(env: &mut Env, future: &str) {
    let value = eval(env, future);
    assert!(
        cljrs_async::await_value(value).now_or_never().is_none(),
        "{future} resolved while it should still be parked"
    );
}

/// Await the `Future` returned by `onto-chan!`; it resolves to its channel.
async fn resolved(env: &mut Env, future: &str) {
    let value = eval(env, future);
    let result = cljrs_async::await_value(value).await.unwrap();
    assert!(
        as_channel(&result).is_some(),
        "{future} resolved to {result}"
    );
}

/// Non-blocking take via `poll!`: `nil` when nothing is available.
fn poll(env: &mut Env, source: &str) -> Value {
    eval(env, &format!("(a/poll! {source})"))
}

fn offer(env: &mut Env, source: &str, value: i64) {
    let accepted = eval(env, &format!("(a/offer! {source} {value})"));
    assert_eq!(accepted, Value::Bool(true), "offer! {value} onto {source}");
}

#[test]
fn onto_chan_parks_on_a_rendezvous_channel_without_a_taker_then_delivers() {
    run(|mut env| async move {
        eval(
            &mut env,
            "(def c (a/chan)) (def done (a/onto-chan! c [1 2 3]))",
        );
        let c = chan(&mut env, "c");
        parked().await;
        assert_pending(&mut env, "done");

        // Each take releases exactly one value; in between, the helper parks
        // again on the next rendezvous offer.
        for expected in 1..=3 {
            assert_eq!(ch(&c).take().await, Value::Long(expected));
            parked().await;
        }
        resolved(&mut env, "done").await;
        assert_eq!(ch(&c).take().await, Value::Nil, "onto-chan! closes ch");
    });
}

#[test]
fn onto_chan_parks_on_a_full_buffer_then_delivers() {
    run(|mut env| async move {
        eval(
            &mut env,
            "(def c (a/chan 1)) (def done (a/onto-chan! c [1 2 3]))",
        );
        let c = chan(&mut env, "c");
        parked().await;
        assert_pending(&mut env, "done");
        for expected in 1..=3 {
            assert_eq!(ch(&c).take().await, Value::Long(expected));
        }
        resolved(&mut env, "done").await;
        assert_eq!(ch(&c).take().await, Value::Nil);
    });
}

#[test]
fn onto_chan_stops_when_its_rendezvous_channel_closes_while_parked() {
    run(|mut env| async move {
        eval(
            &mut env,
            "(def c (a/chan)) (def done (a/onto-chan! c [1 2 3]))",
        );
        let c = chan(&mut env, "c");
        parked().await;
        ch(&c).close();
        resolved(&mut env, "done").await;
        // The parked offer was retracted, and nothing later was put.
        assert_eq!(ch(&c).take().await, Value::Nil);
        parked().await;
    });
}

#[test]
fn onto_chan_stops_when_its_buffered_channel_closes_while_parked() {
    run(|mut env| async move {
        eval(
            &mut env,
            "(def c (a/chan 1)) (def done (a/onto-chan! c [1 2 3]))",
        );
        let c = chan(&mut env, "c");
        parked().await; // 1 is buffered; the put of 2 is parked
        ch(&c).close();
        resolved(&mut env, "done").await;
        // Already-buffered values still drain; the parked put did not land.
        assert_eq!(ch(&c).take().await, Value::Long(1));
        assert_eq!(ch(&c).take().await, Value::Nil);
    });
}

#[test]
fn to_chan_delivers_everything_then_closes_without_spinning() {
    run(|mut env| async move {
        eval(&mut env, "(def c (a/to-chan! [1 2 3]))");
        let c = chan(&mut env, "c");
        parked().await;
        for expected in 1..=3 {
            assert_eq!(ch(&c).take().await, Value::Long(expected));
        }
        assert_eq!(ch(&c).take().await, Value::Nil);
    });
}

#[test]
fn mult_parks_on_an_empty_source_and_on_rendezvous_and_full_taps() {
    run(|mut env| async move {
        // Taps are visited in registration order: buffered `t1`, then
        // rendezvous `t2`.
        eval(
            &mut env,
            "(def src (a/chan 4)) (def m (a/mult src))
             (def t1 (a/chan 1)) (def t2 (a/chan))
             (a/tap! m t1) (a/tap! m t2)",
        );
        let t1 = chan(&mut env, "t1");
        let t2 = chan(&mut env, "t2");

        // Empty source: the mult parks on its take.
        parked().await;
        assert_eq!(poll(&mut env, "t1"), Value::Nil);

        // 10 lands in t1; the mult parks on the rendezvous tap t2.
        offer(&mut env, "src", 10);
        parked().await;
        assert_eq!(ch(&t1).take().await, Value::Long(10));
        parked().await;
        assert_eq!(ch(&t2).take().await, Value::Long(10));

        // Back on the empty source. 20 reaches t1, then t2 blocks again.
        parked().await;
        offer(&mut env, "src", 20);
        parked().await;
        assert_eq!(ch(&t2).take().await, Value::Long(20));
        // t1 still holds 20 (capacity 1). 30 now blocks on the full tap t1,
        // so t2 must not see it yet.
        offer(&mut env, "src", 30);
        parked().await;
        assert_eq!(poll(&mut env, "t2"), Value::Nil);
        assert_eq!(ch(&t1).take().await, Value::Long(20));
        assert_eq!(ch(&t1).take().await, Value::Long(30));
        assert_eq!(ch(&t2).take().await, Value::Long(30));

        // Closing the source while the mult is parked on it closes the taps.
        parked().await;
        eval(&mut env, "(a/close! src)");
        assert_eq!(ch(&t1).take().await, Value::Nil);
        assert_eq!(ch(&t2).take().await, Value::Nil);
        parked().await;
    });
}

#[test]
fn mult_skips_a_tap_closed_while_it_is_parked_on_it() {
    run(|mut env| async move {
        eval(
            &mut env,
            "(def src (a/chan 4)) (def m (a/mult src))
             (def blocked (a/chan)) (def t (a/chan 1))
             (a/tap! m blocked) (a/tap! m t)",
        );
        let blocked = chan(&mut env, "blocked");
        let t = chan(&mut env, "t");

        offer(&mut env, "src", 1);
        parked().await; // parked on the rendezvous put to `blocked`
        assert_eq!(poll(&mut env, "t"), Value::Nil);
        ch(&blocked).close();
        // The failed put is abandoned and the value continues to `t`.
        assert_eq!(ch(&t).take().await, Value::Long(1));

        // The closed tap does not stall later values either.
        offer(&mut env, "src", 2);
        assert_eq!(ch(&t).take().await, Value::Long(2));
        parked().await;
        eval(&mut env, "(a/close! src)");
        assert_eq!(ch(&t).take().await, Value::Nil);
    });
}
