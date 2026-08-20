//! Property oracle: for every special form, evaluating `^meta F` agrees with
//! evaluating `F`.
//!
//! Reader metadata is advisory for structural dispatch — a `^hint` in front of
//! a params vector, a field vector, a binding vector, a name symbol or an arity
//! clause must not change how the form is parsed.

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

fn eval_fresh(src: &str) -> Result<Value, String> {
    let (_, mut env) = make_env();
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().map_err(|e| format!("parse: {e:?}"))?;
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env)
            .map_err(|e| format!("eval: {e:?}"))?;
    }
    Ok(result)
}

/// Every annotation that must be structurally transparent.
const ANNOTATIONS: &[&str] = &["^:marker ", "^String ", "^{:doc \"d\"} "];

/// `template` contains one `{}` placeholder marking an annotation site.
fn assert_meta_transparent(template: &str) {
    let bare = template.replace("{}", "");
    let expected = eval_fresh(&bare).unwrap_or_else(|e| panic!("bare form failed: {bare}\n{e}"));

    for ann in ANNOTATIONS {
        let annotated = template.replace("{}", ann);
        let got = eval_fresh(&annotated)
            .unwrap_or_else(|e| panic!("annotated form failed: {annotated}\n{e}"));
        assert_eq!(
            got, expected,
            "`{annotated}` disagreed with `{bare}` (annotation `{ann}`)"
        );
    }
}

#[test]
fn def_name() {
    assert_meta_transparent("(def {}x 41) (inc x)");
}

#[test]
fn defn_name() {
    assert_meta_transparent("(defn {}f [a] (inc a)) (f 41)");
}

#[test]
fn defn_params_vector() {
    // `(defn f ^String [s] s)` — green on the JVM, previously rejected here
    // with "fn* expects vector or arity clauses".
    assert_meta_transparent("(defn f {}[a] (inc a)) (f 41)");
}

#[test]
fn defn_docstring_and_body() {
    assert_meta_transparent("(defn f \"doc\" {}[a] (inc a)) (f 41)");
}

#[test]
fn fn_params_vector() {
    assert_meta_transparent("((fn {}[a] (inc a)) 41)");
}

#[test]
fn fn_arity_clause() {
    assert_meta_transparent("((fn {}([a] (inc a)) ([a b] b)) 41)");
}

#[test]
fn fn_param_symbol() {
    assert_meta_transparent("((fn [{}a] (inc a)) 41)");
}

#[test]
fn let_binding_vector() {
    assert_meta_transparent("(let {}[a 42] a)");
}

#[test]
fn loop_binding_vector() {
    assert_meta_transparent("(loop {}[a 0] (if (< a 42) (recur (inc a)) a))");
}

#[test]
fn letfn_binding_vector() {
    assert_meta_transparent("(letfn {}[(g [x] (inc x))] (g 41))");
}

#[test]
fn letfn_binding_name() {
    assert_meta_transparent("(letfn [({}g [x] (inc x))] (g 41))");
}

#[test]
fn binding_vector() {
    assert_meta_transparent("(def ^:dynamic *v* 1) (binding {}[*v* 42] *v*)");
}

#[test]
fn defrecord_field() {
    // `(defrecord R [^String x])` — the reported repro.
    assert_meta_transparent("(defrecord R [{}x]) (:x (->R 42))");
}

#[test]
fn defrecord_name_and_field_vector() {
    assert_meta_transparent("(defrecord {}R [x]) (:x (->R 42))");
    assert_meta_transparent("(defrecord R {}[x]) (:x (->R 42))");
}

#[test]
fn defmulti_and_defmethod() {
    assert_meta_transparent("(defmulti {}m identity) (defmethod m 1 [_] 42) (m 1)");
    assert_meta_transparent("(defmulti m identity) (defmethod m 1 {}[_] 42) (m 1)");
}

#[test]
fn defprotocol_and_extend_type() {
    assert_meta_transparent(
        "(defprotocol {}P (pm [this])) (defrecord R [x]) \
         (extend-type R P (pm [this] (:x this))) (pm (->R 42))",
    );
    assert_meta_transparent(
        "(defprotocol P (pm [this])) (defrecord R [x]) \
         (extend-type {}R P (pm [this] (:x this))) (pm (->R 42))",
    );
    assert_meta_transparent(
        "(defprotocol P (pm [this])) (defrecord R [x]) \
         (extend-type R P ({}pm [this] (:x this))) (pm (->R 42))",
    );
}

#[test]
fn extend_protocol() {
    assert_meta_transparent(
        "(defprotocol P (pm [this])) (defrecord R [x]) \
         (extend-protocol {}P R (pm [this] (:x this))) (pm (->R 42))",
    );
}

#[test]
fn defrecord_inline_impl() {
    assert_meta_transparent(
        "(defprotocol P (pm [this])) (defrecord R [x] {}P (pm [this] (:x this))) \
         (pm (->R 42))",
    );
}

#[test]
fn reify_impl() {
    assert_meta_transparent("(defprotocol P (pm [this])) (pm (reify {}P (pm [this] 42)))");
}

#[test]
fn defmacro_params_vector() {
    assert_meta_transparent("(defmacro m {}[a] `(inc ~a)) (m 41)");
}

#[test]
fn pre_post_conditions() {
    assert_meta_transparent("(defn f [a] {}{:pre [(pos? a)]} (inc a)) (f 41)");
}
