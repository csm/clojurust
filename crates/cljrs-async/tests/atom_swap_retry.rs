#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use cljrs_gc::mutator::MutatorState;
use cljrs_runtime::{ExecutionMode, Runtime, env::env::Env};
use cljrs_value::Value;

#[test]
fn parked_swap_preserves_a_sibling_fibers_update() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::Tiered] {
        let runtime = Runtime::builder().execution_mode(mode).build().unwrap();
        let globals = runtime.globals().clone();
        let executor = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let local = tokio::task::LocalSet::new();
        local.block_on(&executor, async { cljrs_async::init(&globals) });
        let mut carrier = MutatorState::fresh();
        let result = cljrs_async::fiber::run_root(&executor, &local, &mut carrier, move || {
            let mut env = Env::new(globals, "user");
            let source = r#"
              (let [state (atom {:left 0}) entered (promise) resume (promise)
                    writer (future (swap! state (fn [old]
                             (when-not (contains? old :right)
                               (deliver entered true)
                               @resume)
                             (assoc old :left 1))))]
                @entered
                (swap! state assoc :right 1)
                (deliver resume true)
                @writer
                (= {:left 1 :right 1} @state))
            "#;
            let forms = cljrs_reader::Parser::new(source.into(), "<fiber-swap>".into())
                .parse_all()
                .unwrap();
            cljrs_runtime::tiered::eval(&forms[0], &mut env)
        })
        .unwrap();
        assert_eq!(result, Value::Bool(true), "{mode:?}");
    }
}
