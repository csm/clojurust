//! Regression tests: a record or deftype's dispatch tag is qualified by the
//! namespace that defined it.
//!
//! The tag used to be the bare type name, so two namespaces that each defined
//! a `Point` shared one tag: the second namespace's protocol impls replaced the
//! first one's, and `instance?` could not tell the two types apart. This broke
//! hosting several addons in one runtime, where unrelated libraries choose
//! record names independently.

use std::sync::Arc;

use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};
use cljrs_value::Value;

fn make_env() -> (Arc<GlobalEnv>, Env) {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

/// Evaluate `src` and return the last form's value as `pr-str` renders it.
fn eval_printed(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse error");
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    match result {
        Value::Str(s) => s.get().clone(),
        other => panic!("expected a string from pr-str, got {other}"),
    }
}

const TWO_POINTS: &str = r#"
(ns proto.p)
(defprotocol Named (nm [x]))

(ns a.one)
(defrecord Point [x] proto.p/Named (nm [_] :one))

(ns b.two)
(defrecord Point [x])
(extend-type Point proto.p/Named (nm [_] :two))

(ns user)
"#;

#[test]
fn same_named_records_in_two_namespaces_keep_their_own_impls() {
    let src = format!(
        "{TWO_POINTS}(pr-str [(proto.p/nm (a.one/->Point 1)) (proto.p/nm (b.two/->Point 1))])"
    );
    assert_eq!(eval_printed(&src), "[:one :two]");
}

#[test]
fn instance_q_tells_same_named_records_apart() {
    let src = format!(
        "{TWO_POINTS}(pr-str [(instance? a.one/Point (a.one/->Point 1)) \
                              (instance? b.two/Point (a.one/->Point 1))])"
    );
    assert_eq!(eval_printed(&src), "[true false]");
}

#[test]
fn extend_protocol_resolves_a_record_through_its_namespace() {
    let src = r#"
(ns proto.q)
(defprotocol Sized (size [x]))
(ns c.three)
(defrecord Box [w])
(extend-protocol proto.q/Sized
  Box (size [b] (:w b))
  String (size [s] (count s)))
(ns user)
(pr-str [(proto.q/size (c.three/->Box 3)) (proto.q/size "abcd")])
"#;
    assert_eq!(eval_printed(src), "[3 4]");
}

#[test]
fn a_record_prints_with_its_qualified_type_name() {
    let src = format!("{TWO_POINTS}(pr-str (a.one/->Point 1))");
    assert_eq!(eval_printed(&src), "#a.one.Point{:x 1}");
}

/// A record extended from another namespace under its unqualified name, with
/// `import` standing for that namespace's `(:import ...)` clause.
fn extend_imported(import: &str) -> String {
    format!(
        r#"
(ns proto.r)
(defprotocol Named (nm [x]))
(ns my-lib.geo)
(defrecord Point [x])
(ns b.imp {import})
(extend-type Point proto.r/Named (nm [_] :extended))
(ns user)
(pr-str (proto.r/nm (my-lib.geo/->Point 1)))
"#
    )
}

#[test]
fn an_imported_record_is_extended_under_its_qualified_tag() {
    for import in [
        "(:import [my-lib.geo Point])",
        "(:import (my-lib.geo Point))",
        "(:import my-lib.geo.Point)",
        // The JVM spelling of the namespace as a package.
        "(:import [my_lib.geo Point])",
    ] {
        assert_eq!(
            eval_printed(&extend_imported(import)),
            ":extended",
            "{import}"
        );
    }
}

#[test]
fn importing_a_name_that_is_no_loaded_type_is_ignored() {
    let src = r#"
(ns d.four (:import [java.util List] java.io.File))
(pr-str (ns-name *ns*))
"#;
    assert_eq!(eval_printed(src), "d.four");
}

/// Evaluate `src` and return the first error, rendered.
fn eval_error(src: &str) -> String {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let err = parser
        .parse_all()
        .expect("parse error")
        .iter()
        .find_map(|form| cljrs_runtime::interp::eval::eval(form, &mut env).err())
        .expect("evaluation should fail");
    format!("{err:?}")
}

#[test]
fn importing_over_a_local_definition_is_an_error() {
    let src = format!("{TWO_POINTS}(ns b.two (:import [a.one Point]))");
    let msg = eval_error(&src);
    assert!(
        msg.contains("Point already refers to: #'b.two/Point in namespace: b.two"),
        "got: {msg}"
    );
}

#[test]
fn importing_two_types_of_one_name_is_an_error() {
    let src = format!("{TWO_POINTS}(ns e.five (:import [a.one Point] [b.two Point]))");
    let msg = eval_error(&src);
    assert!(
        msg.contains("Point already refers to: #'a.one/Point in namespace: e.five"),
        "got: {msg}"
    );
}

#[test]
fn an_import_can_be_evaluated_again() {
    let src = format!(
        "{TWO_POINTS}\
         (ns e.six (:import [a.one Point]))\
         (ns e.six (:import [a.one Point] a.one.Point))\
         (ns a.one (:import [a.one Point]))\
         (pr-str (ns-name *ns*))"
    );
    assert_eq!(eval_printed(&src), "a.one");
}

#[test]
fn an_unresolved_type_name_is_named_by_the_dispatch_error() {
    let (_, mut env) = make_env();
    let src = extend_imported("");
    let mut parser = Parser::new(src, "<test>".to_string());
    let err = parser
        .parse_all()
        .expect("parse error")
        .iter()
        .find_map(|form| cljrs_runtime::interp::eval::eval(form, &mut env).err())
        .expect("the call should have no implementation");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("No implementation of protocol Named for type my-lib.geo.Point"),
        "got: {msg}"
    );
    assert!(
        msg.contains("registered under the unqualified tag Point"),
        "got: {msg}"
    );
}
