//! A local atom can change while an update function parks or yields on a fiber.
#![allow(clippy::result_large_err)]

use cljrs_reader::Parser;
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::Value;

#[test]
fn local_swap_retries_a_conflicting_update_through_both_call_paths() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::Tiered] {
        for call in ["(swap! state update 1)", "(apply swap! [state update 1])"] {
            let runtime = Runtime::builder().execution_mode(mode).build().unwrap();
            let mut env = runtime.env("user");
            let source = format!(
                r#"
              (def state (atom {{:left 0}}))
              (def first-call (atom true))
              (def calls (atom 0))
              (def watched (atom []))
              (add-watch state :events (fn [_ _ before after]
                (swap! watched conj [before after])))
              (defn update [old amount]
                (swap! calls inc)
                (when (compare-and-set! first-call true false)
                  (reset! state {{:left 0 :right 1}}))
                (assoc old :left (+ (:left old) amount)))
              {call}
              (and (= {{:left 1 :right 1}} @state)
                   (= 2 @calls)
                   (= [[{{:left 0}} {{:left 0 :right 1}}]
                       [{{:left 0 :right 1}} {{:left 1 :right 1}}]] @watched))
            "#
            );
            let forms = Parser::new(source, "<swap-retry>".into())
                .parse_all()
                .unwrap();
            let mut result = Value::Nil;
            for form in forms {
                result = cljrs_runtime::tiered::eval(&form, &mut env).unwrap();
            }
            let diagnostic = Parser::new("[@state @calls @watched]".into(), "<state>".into())
                .parse_all()
                .unwrap();
            let state = cljrs_runtime::tiered::eval(&diagnostic[0], &mut env).unwrap();
            assert_eq!(result, Value::Bool(true), "{mode:?}: {call}: {state}");
        }
    }
}
