//! Regression test: GC root guards held across an `await` must not pop
//! another task's root.
//!
//! `eval_loop_async` roots its loop values with a `ValueRootGuard` that lives
//! across the body's `await`s.  Tasks on one `LocalSet` share the thread's
//! `VALUE_ROOTS` stack but need not finish in LIFO order.  When the guard
//! popped whatever entry was on top, task A finishing while task B was
//! suspended removed B's root and left A's own entry pointing at A's freed
//! loop values: B's accumulator was then collected while B still used it.
//!
//! Since virtual-threads Phase 5 these cases run on fibers (`fiber::eval_form`);
//! the colored `eval_async` evaluator they first targeted is wasm32-only.

#![cfg(not(feature = "no-gc"))]

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use cljrs_async::fiber::eval_form;
use cljrs_reader::{Form, Parser};
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals
}

fn parse_one(src: &str) -> Form {
    Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse error")
        .remove(0)
}

#[test]
fn interleaved_async_loops_keep_their_roots() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    let globals = async_env();

    let (a, b) = local.block_on(&rt, async move {
        let mut setup = Env::new(globals.clone(), "user");
        eval_form(
            &parse_one("(require '[clojure.core.async :refer [timeout]])"),
            &mut setup,
        )
        .await
        .expect("require");

        // A starts first: it roots its loop values, then yields.
        let mut env_a = Env::new(globals.clone(), "user");
        let task_a = tokio::task::spawn_local(async move {
            let form =
                parse_one("(loop* [i 0] (if (< i 2) (do (await (timeout 1)) (recur (inc i))) :a))");
            // Format before the task ends: a JoinHandle output is not a GC root.
            format!("{}", eval_form(&form, &mut env_a).await.expect("loop a"))
        });

        // B builds its accumulator without yielding, so each iteration's alloc
        // frame is released and the accumulator is rooted only by the loop's
        // value root.  It then waits long enough for A to finish first.
        let mut env_b = Env::new(globals.clone(), "user");
        let task_b = tokio::task::spawn_local(async move {
            let form = parse_one(
                "(loop* [acc [] i 0]
                   (if (< i 50)
                     (do (when (= i 49) (await (timeout 50)))
                         (recur (conj acc (str \"item-\" i)) (inc i)))
                     acc))",
            );
            let b = eval_form(&form, &mut env_b).await.expect("loop b");
            let Value::Vector(v) = &b else {
                panic!("loop b returned {b}");
            };
            v.get()
                .iter()
                .map(|x| format!("{x}"))
                .collect::<Vec<String>>()
        });

        // Collect at every scheduler turn until both loops are done.
        let done = Rc::new(Cell::new(false));
        let gc_done = done.clone();
        let collector = tokio::task::spawn_local(async move {
            while !gc_done.get() {
                tokio::task::yield_now().await;
                cljrs_gc::request_gc();
                cljrs_runtime::env::gc_roots::async_gc_collect();
            }
        });

        let a = task_a.await.expect("task a");
        let b = task_b.await.expect("task b");
        done.set(true);
        collector.await.expect("collector");
        (a, b)
    });

    assert_eq!(a, ":a");
    let items = b;
    let expected: Vec<String> = (0..50).map(|i| format!("\"item-{i}\"")).collect();
    assert_eq!(items, expected);
}
