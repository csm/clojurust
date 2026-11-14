//! Clojure-level tests for `spawn-isolate`: starting an isolate from Clojure,
//! copying arguments and the result across the boundary, and moving
//! isolate-channel ends to the new isolate.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_runtime::env::error::EvalError;
use cljrs_value::Value;

/// The namespace spawned isolates load their entry points from.
const WORKER_NS: &str = r#"
(ns test.worker
  (:require [clojure.core.async :refer [isolate-put! isolate-take! spawn-isolate]]))

(defn add [a b] (+ a b))
(defn describe [m] (assoc m :keys (vec (sort (keys m)))))
(defn boom [x] (throw (ex-info "boom" {:x x})))
(defn closure-result [] (fn [] 1))
(defn ^:async async-add [a b] (+ a b))
(defn nested [n] @(spawn-isolate 'test.worker/add n 1))
(defn echo [rx tx]
  (loop [n 0]
    (if-let [m @(isolate-take! rx)]
      (do (isolate-put! tx [:echo m]) (recur (inc n)))
      n)))
"#;

/// A runtime that can spawn isolates: each isolate gets a runtime built the
/// same way, including the factory, so isolates can spawn isolates.
fn spawning_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .builtin_source("test.worker", WORKER_NS)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals.set_isolate_factory(Arc::new(|| Ok(spawning_env())));
    globals
}

/// Evaluate every form in `src` in a root fiber, so `deref` can park while the
/// spawned isolate runs.
#[allow(clippy::result_large_err)]
fn run(src: &'static str, with_factory: bool) -> Result<Value, EvalError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime");
    cljrs_async::fiber::run_root_owned(&rt, tokio::task::LocalSet::new(), move || {
        let globals = if with_factory {
            spawning_env()
        } else {
            let globals = cljrs_runtime::Runtime::builder()
                .build()
                .expect("runtime")
                .into_globals();
            cljrs_async::init(&globals);
            globals
        };
        let mut env = Env::new(globals, "user");
        let prelude = "(require '[clojure.core.async :refer \
                       [spawn-isolate isolate-chan isolate-put! isolate-poll! isolate-take!]])";
        let mut result = Value::Nil;
        for source in [prelude, src] {
            let mut parser = Parser::new(source.to_string(), "<test>".to_string());
            for form in parser.parse_all().expect("parse error") {
                result = cljrs_runtime::interp::eval::eval(&form, &mut env)?;
            }
        }
        // Compare by printed form: the value does not outlive this heap.
        Ok(Value::string(format!("{result}")))
    })
}

fn printed(src: &'static str) -> String {
    match run(src, true).expect("eval error") {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other}"),
    }
}

fn error(src: &'static str) -> String {
    run(src, true).expect_err("expected an error").to_string()
}

#[test]
fn result_is_copied_back() {
    assert_eq!(printed("@(spawn-isolate 'test.worker/add 40 2)"), "42");
    assert_eq!(
        printed("@(spawn-isolate 'test.worker/describe {:b 1 :a \"x\"})"),
        printed("{:b 1 :a \"x\" :keys [:a :b]}")
    );
}

#[test]
fn isolates_run_in_parallel_and_join_in_any_order() {
    assert_eq!(
        printed(
            "(let [fs (mapv #(spawn-isolate 'test.worker/add % 1) (range 4))]
               (mapv deref (reverse fs)))"
        ),
        "[4 3 2 1]"
    );
}

#[test]
fn async_entry_point_is_awaited() {
    assert_eq!(printed("@(spawn-isolate 'test.worker/async-add 1 2)"), "3");
}

#[test]
fn a_spawned_isolate_can_spawn() {
    assert_eq!(printed("@(spawn-isolate 'test.worker/nested 9)"), "10");
}

#[test]
fn thrown_ex_info_keeps_its_message_and_data() {
    assert_eq!(
        printed(
            "(try @(spawn-isolate 'test.worker/boom 7)
               (catch Exception e [(ex-message e) (ex-data e)]))"
        ),
        "[\"boom\" {:x 7}]"
    );
}

#[test]
fn channel_ends_move_to_the_new_isolate() {
    assert_eq!(
        printed(
            "(let [[to-worker worker-rx] (isolate-chan)
                   [worker-tx from-worker] (isolate-chan)
                   done (spawn-isolate 'test.worker/echo worker-rx worker-tx)]
               (isolate-put! to-worker {:n 1})
               (isolate-put! to-worker {:n 2})
               (let [a @(isolate-take! from-worker)
                     b @(isolate-take! from-worker)
                     moved (try (isolate-poll! worker-rx)
                                (catch Exception _ :moved))]
                 (isolate-put! to-worker nil)
                 [a b moved @done]))"
        ),
        "[[:echo {:n 1}] [:echo {:n 2}] :moved 2]"
    );
}

#[test]
fn entry_point_must_be_a_qualified_symbol() {
    assert!(error("(spawn-isolate (fn [] 1))").contains("namespace-qualified symbol"));
    assert!(error("(spawn-isolate 'add 1 2)").contains("namespace-qualified symbol"));
}

#[test]
fn unshareable_argument_fails_at_the_spawn_site() {
    let message = error("(spawn-isolate 'test.worker/add (atom 1) 2)");
    assert!(message.contains("argument 1"), "{message}");
    assert!(message.contains("atom"), "{message}");
}

#[test]
fn a_rejected_argument_leaves_receivers_in_place() {
    assert_eq!(
        printed(
            "(let [[tx rx] (isolate-chan)]
               (try (spawn-isolate 'test.worker/echo rx (atom 1))
                    (catch Exception _ nil))
               (isolate-put! tx :still-here)
               (isolate-poll! rx))"
        ),
        ":still-here"
    );
}

#[test]
fn failures_inside_the_isolate_fail_the_future() {
    assert!(error("@(spawn-isolate 'test.missing/f)").contains("could not load namespace"));
    assert!(error("@(spawn-isolate 'test.worker/missing)").contains("no var named missing"));
    assert!(error("@(spawn-isolate 'test.worker/closure-result)").contains("cannot be returned"));
}

#[test]
fn a_runtime_without_a_factory_refuses_to_spawn() {
    let message = run("(spawn-isolate 'clojure.core/identity 1)", false)
        .expect_err("expected an error")
        .to_string();
    assert!(message.contains("cannot start isolates"), "{message}");
}
