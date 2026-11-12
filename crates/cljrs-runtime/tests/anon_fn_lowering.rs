//! Regression for #486: reader anonymous functions must reach the IR tier.

mod common;

use cljrs_reader::{Form, Parser};
use cljrs_runtime::interp::macros::macroexpand_all;
use cljrs_value::Value;

fn parse(src: &str) -> Form {
    Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse")
        .remove(0)
}

#[test]
fn anon_fn_expansion_recursively_expands_body_macros() {
    let (_globals, mut env) = common::fresh_env();
    let input = parse("#(when % (inc %))");
    let expected = parse("(fn* [p__1] (if p__1 (do (inc p__1)) nil))");
    let expanded = macroexpand_all(&input, &mut env).expect("expand");
    assert_eq!(expanded, expected);
    assert_eq!(expanded.span, input.span);
}

#[test]
fn quoted_anon_fns_are_not_walked() {
    let (_globals, mut env) = common::fresh_env();
    for src in [
        "'#(when % (inc %))",
        "(quote #(when % (inc %)))",
        "'[{:f #(when % (inc %))}]",
    ] {
        let input = parse(src);
        assert_eq!(macroexpand_all(&input, &mut env).expect("expand"), input);
    }
}

#[test]
fn anon_fn_bodies_lower_and_agree_with_tree_walking() {
    cljrs_runtime::tiered::force_eager_lowering();
    let (_walk_globals, mut walked_env) = common::fresh_env();
    let (globals, mut tiered_env) = common::fresh_env_in(cljrs_runtime::ExecutionMode::Tiered);

    for (body, expected) in [
        ("(mapv #(+ % x) [1 2 3])", "[2 3 4]"),
        (
            "(#(vector % %1 %3 %&) 10 20 30 40 50)",
            "[10 10 30 (40 50)]",
        ),
        ("(#(vector 7))", "[7]"),
        ("(let [f #(when % (inc %))] (f 3))", "4"),
        ("((first [#(+ % x)]) 2)", "3"),
        ("((get {:f #(* % x)} :f) 3)", "3"),
        ("((first #{#(- % x)}) 4)", "3"),
        ("(#(+ @% x) (atom 2))", "3"),
        // Placeholder rewriting follows reader semantics even inside quote.
        ("(#(vector '% %) 7)", "[p__1 7]"),
    ] {
        let definition = format!("(defn probe [x] (pr-str {body}))");
        common::eval_in(&mut walked_env, &definition).expect("tree-walk definition");
        common::eval_in(&mut tiered_env, &definition).expect("tiered definition");

        let value = globals
            .lookup_in_ns(&tiered_env.current_ns, "probe")
            .expect("probe");
        let Value::Fn(f) = value else {
            panic!("probe is not a function");
        };
        assert!(
            globals
                .ir_cache()
                .get(f.get().arities[0].ir_arity_id)
                .is_some(),
            "{body} never reached the IR tier"
        );

        let walked = common::eval_in(&mut walked_env, "(probe 1)").expect("tree-walk call");
        let lowered = common::eval_in(&mut tiered_env, "(probe 1)").expect("IR call");
        assert_eq!(walked, lowered, "tier mismatch for {body}");
        let Value::Str(result) = lowered else {
            panic!("pr-str did not return a string");
        };
        assert_eq!(result.get().as_str(), expected, "{body}");
    }
}
