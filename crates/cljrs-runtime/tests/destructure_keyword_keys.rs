//! Issue #493: keyword entries in `:keys` bind unqualified locals in both tiers.

mod common;

use cljrs_runtime::ExecutionMode;
use cljrs_value::Value;

#[test]
fn keyword_keys_bind_at_top_level() {
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::TieredNoJit] {
        let (_, mut env) = common::fresh_env_in(mode);
        for (source, expected) in [
            ("(let [{:keys [:x]} {:x 5}] x)", "5"),
            ("(let [{:keys [:person/x]} {:person/x 6}] x)", "6"),
            ("(let [{:person/keys [:x]} {:person/x 7}] x)", "7"),
            ("(let [{:other/keys [:person/x]} {:person/x 6}] x)", "6"),
            ("(let [{:keys [::x]} {::x 9}] x)", "9"),
            ("(let [{:keys [:x y]} {:x 5 :y 6}] [x y])", "[5 6]"),
            ("(let [{:keys [:x]} {}] x)", "nil"),
        ] {
            let got = common::eval_in(&mut env, &format!("(pr-str {source})")).expect(source);
            assert_eq!(got, Value::string(expected), "{mode:?}: {source}");
        }
    }
}

/// Check the IR cache so a silent fallback cannot make the tiered cases pass.
fn assert_both_tiers(params: &str, body: &str, args: &str, expected: &str) {
    cljrs_runtime::tiered::force_eager_lowering();
    for mode in [ExecutionMode::TreeWalk, ExecutionMode::TieredNoJit] {
        let (globals, mut env) = common::fresh_env_in(mode);
        let definition = format!("(defn probe {params} (pr-str {body}))");
        common::eval_in(&mut env, &definition).expect(&definition);
        if mode == ExecutionMode::TieredNoJit {
            let Value::Fn(f) = globals
                .lookup_in_ns(&env.current_ns, "probe")
                .expect("probe")
            else {
                panic!("probe is not a function");
            };
            assert!(
                globals
                    .ir_cache()
                    .get(f.get().arities[0].ir_arity_id)
                    .is_some(),
                "function did not reach the IR tier: {definition}"
            );
        }
        let got = common::eval_in(&mut env, &format!("(probe {args})")).expect(&definition);
        assert_eq!(got, Value::string(expected), "{mode:?}: {definition}");
    }
}

#[test]
fn keyword_keys_bind_function_parameters() {
    for (pattern, args, expected) in [
        ("{:keys [:x]}", "{:x 5}", "5"),
        ("{:keys [:person/x]}", "{:person/x 6}", "6"),
        ("{:person/keys [:x]}", "{:person/x 7}", "7"),
        ("{:other/keys [:person/x]}", "{:person/x 6}", "6"),
        ("{:keys [::x]}", "{::x 9}", "9"),
        ("{:keys [:x] :or {x 8}}", "{}", "8"),
        ("{:keys [:x] :or {x 8}}", "{:x 5}", "5"),
    ] {
        assert_both_tiers(&format!("[{pattern}]"), "x", args, expected);
    }
    assert_both_tiers("[{:keys [:x y]}]", "[x y]", "{:x 5 :y 6}", "[5 6]");
}

#[test]
fn keyword_keys_bind_in_lowered_let_loop_and_nested_patterns() {
    for body in [
        "(let [{:keys [:x]} m] x)",
        "(let [[{:keys [:x]}] [m]] x)",
        "((fn [{:keys [:x]}] x) m)",
        "(loop [{:keys [:x]} m] (if (< x 7) (recur {:x (inc x)}) x))",
    ] {
        let expected = if body.starts_with("(loop") { "7" } else { "5" };
        assert_both_tiers("[m]", body, "{:x 5}", expected);
    }
}

#[test]
fn keyword_keys_defaults_remain_eager_and_follow_binding_order() {
    assert_both_tiers(
        "[m]",
        "(let [calls (atom 0)
               {:keys [:x :y]
                :or {x (do (swap! calls inc) 8) y (inc x)}} m]
           [x y @calls])",
        "{:x 5}",
        "[5 6 1]",
    );
}
