//! `extend-type` / `extend-protocol` are Clojure macros over the `extend`
//! primitive, not Rust special forms.
//!
//! The Rust handlers walked the flat spec list themselves and registered each
//! `(method [params] body)` form into the protocol's impl table one at a time.
//! That made a two-arity method impossible to express: the second
//! `(-show [x pre] ...)` form overwrote the first under the same method name,
//! so the single-arity call became an arity error. Routing both surface forms
//! through one `extend` call — with the arities of a method collapsed into one
//! `fn` — fixes that as a side effect of removing the duplication.
//!
//! These tests pin the surface the macros must keep: grouping by protocol,
//! grouping by type, multi-arity methods, metadata-carrying heads, and the
//! `extend` primitive being callable on its own.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `src` and return the last value rendered with `pr-str`.
fn eval_pr(src: &str) -> String {
    let (_globals, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().as_str().to_string(),
        other => panic!("expected a string from pr-str, got a {}", other.type_name()),
    }
}

const PROTO: &str = "(defprotocol IShow (-show [x] [x pre]))\n";

// ── extend-type ──────────────────────────────────────────────────────────────

#[test]
fn extend_type_registers_one_protocol() {
    let src = format!(
        "{PROTO}
         (extend-type String IShow (-show [s] (str \"S:\" s)))
         (pr-str (-show \"hi\"))"
    );
    assert_eq!(eval_pr(&src), "\"S:hi\"");
}

#[test]
fn extend_type_registers_several_protocols_in_one_form() {
    let src = format!(
        "{PROTO}
         (defprotocol ISize (-size [x]))
         (extend-type String
           IShow (-show [s] (str \"S:\" s))
           ISize (-size [s] (count s)))
         (pr-str [(-show \"ab\") (-size \"ab\")])"
    );
    assert_eq!(eval_pr(&src), "[\"S:ab\" 2]");
}

/// The regression the refactor fixes: two arities of the same method.
#[test]
fn extend_type_collapses_a_methods_arities_into_one_fn() {
    let src = format!(
        "{PROTO}
         (extend-type String
           IShow
           (-show [s] (str \"S:\" s))
           (-show [s pre] (str pre s)))
         (pr-str [(-show \"hi\") (-show \"hi\" \">>\")])"
    );
    assert_eq!(eval_pr(&src), "[\"S:hi\" \">>hi\"]");
}

// ── extend-protocol ──────────────────────────────────────────────────────────

#[test]
fn extend_protocol_registers_several_types_in_one_form() {
    let src = format!(
        "{PROTO}
         (extend-protocol IShow
           String (-show [s] (str \"S:\" s))
           Long   (-show [n] (str \"L:\" n))
           Vector (-show [v] (str \"V:\" (count v))))
         (pr-str [(-show \"a\") (-show 42) (-show [1 2 3])])"
    );
    assert_eq!(eval_pr(&src), "[\"S:a\" \"L:42\" \"V:3\"]");
}

#[test]
fn extend_protocol_collapses_a_methods_arities_into_one_fn() {
    let src = format!(
        "{PROTO}
         (extend-protocol IShow
           String
           (-show [s] (str \"S:\" s))
           (-show [s pre] (str pre s)))
         (pr-str [(-show \"hi\") (-show \"hi\" \"<\")])"
    );
    assert_eq!(eval_pr(&src), "[\"S:hi\" \"<hi\"]");
}

// ── metadata transparency ────────────────────────────────────────────────────
//
// The head of a group is found with `symbol?`, and a method form is read with
// `first` — both must see through a metadata wrapper, because a macro receives
// its arguments as values with the metadata still attached.

#[test]
fn a_metadata_carrying_head_still_starts_a_group() {
    let src = format!(
        "{PROTO}
         (extend-type ^{{:doc \"t\"}} String
           ^{{:doc \"p\"}} IShow
           ^{{:doc \"m\"}} (-show [s] (str \"S:\" s)))
         (pr-str (-show \"hi\"))"
    );
    assert_eq!(eval_pr(&src), "\"S:hi\"");
}

// ── the primitive ────────────────────────────────────────────────────────────

#[test]
fn extend_is_callable_as_a_plain_function() {
    let src = format!(
        "{PROTO}
         (extend 'String IShow {{:-show (fn [s] (str \"S:\" s))}})
         (pr-str (-show \"hi\"))"
    );
    assert_eq!(eval_pr(&src), "\"S:hi\"");
}

#[test]
fn extend_takes_several_protocol_method_map_pairs() {
    let src = format!(
        "{PROTO}
         (defprotocol ISize (-size [x]))
         (extend 'String
           IShow {{:-show (fn [s] (str \"S:\" s))}}
           ISize {{:-size (fn [s] (count s))}})
         (pr-str [(-show \"ab\") (-size \"ab\")])"
    );
    assert_eq!(eval_pr(&src), "[\"S:ab\" 2]");
}

/// A later `extend` replaces the impl a former one registered for the same
/// type and method — the property `extend-type`'s reload story rests on.
#[test]
fn a_later_extend_replaces_the_earlier_impl() {
    let src = format!(
        "{PROTO}
         (extend-type String IShow (-show [s] (str \"first:\" s)))
         (extend-type String IShow (-show [s] (str \"second:\" s)))
         (pr-str (-show \"hi\"))"
    );
    assert_eq!(eval_pr(&src), "\"second:hi\"");
}
