#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

//! Namespace loading is claimed per computation (thread + fiber), not per OS
//! thread: a second fiber requiring a namespace whose load is parked on the
//! same carrier waits for it instead of reporting a circular require (#451).

use cljrs_async::fiber;
use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn run(source: &'static str, builtins: &'static [(&'static str, &'static str)]) -> String {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let globals = runtime.globals().clone();
            for (ns, src) in builtins {
                globals.register_builtin_source(ns, src);
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let local = tokio::task::LocalSet::new();
            local.block_on(&rt, async { cljrs_async::init(&globals) });
            let mut carrier = MutatorState::fresh();
            let result = fiber::run_root(&rt, &local, &mut carrier, move || {
                let mut env = Env::new(globals, "user");
                let forms = cljrs_reader::Parser::new(source.into(), "<concurrent-require>".into())
                    .parse_all()
                    .unwrap();
                let mut result = Value::Nil;
                for form in forms {
                    result = cljrs_runtime::tiered::eval(&form, &mut env)?;
                }
                Ok(result)
            })
            .unwrap();
            result.to_string()
        })
        .unwrap()
        .join()
        .unwrap()
}

#[test]
fn second_fiber_waits_for_a_parked_load() {
    let result = run(
        r#"
        (require '[clojure.core.async :as a])
        (let [req (fn [] (a/go (try (require 'parkns) @(resolve 'parkns/x)
                                    (catch Throwable e (str "ERR: " (ex-message e))))))
              c1 (req)
              _ (Thread/sleep 50)
              c2 (req)]
          [(a/<!! c1) (a/<!! c2)])
        "#,
        &[("parkns", "(ns parkns)\n(def x (do (Thread/sleep 300) 42))")],
    );
    assert_eq!(result, "[42 42]");
}

#[test]
fn genuine_circular_require_still_errors() {
    let result = run(
        r#"
        (try (require 'circa) :loaded
             (catch Throwable e (ex-message e)))
        "#,
        &[
            ("circa", "(ns circa (:require circb))"),
            ("circb", "(ns circb (:require circa))"),
        ],
    );
    assert!(result.contains("circular require: circa"), "{result}");
}
