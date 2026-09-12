//! `extenders`: the types that extend a protocol.
//!
//! It was simply unbound, so any code doing protocol introspection — a
//! registry that reports what it can dispatch, a test asserting an extension
//! took effect — failed with "unbound symbol: extenders" rather than answering.
//!
//! The three properties worth pinning are the ones a naive implementation gets
//! wrong:
//!
//! 1. **nil, not an empty seq, when nothing extends the protocol.** Clojure
//!    defines `extenders` as `(keys (:impls protocol))`, and `keys` of an empty
//!    map is `nil`. Callers thread the result through `seq`/`when-let`, which
//!    treat `()` as truthy.
//! 2. **A stable order.** The registry is a `HashMap`, so its natural order
//!    differs between runs of the same program. Anything printing or diffing
//!    the result would see churn that has nothing to do with the code.
//! 3. **Agreement with `extends?`.** The two read the same registry and must
//!    not disagree about any type.
//!
//! One environment is shared by every assertion here: building a runtime
//! re-evaluates `bootstrap.cljrs`, and doing that per assertion is what made
//! other suites expensive (CLJRS-TEST-RUNTIME-REUSE).

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

/// Evaluate every form in `src` in `env`; yield the last value.
fn eval_in(env: &mut Env, src: &str) -> Value {
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser
        .parse_all()
        .unwrap_or_else(|e| panic!("parse {src}: {e:?}"));
    let mut result = Value::Nil;
    for form in forms {
        result = cljrs_runtime::interp::eval::eval(&form, env)
            .unwrap_or_else(|e| panic!("eval {src}: {e:?}"));
    }
    result
}

/// `(pr-str <src>)`, so assertions read as the text a user would see.
fn printed(env: &mut Env, src: &str) -> String {
    match eval_in(env, &format!("(pr-str {src})")) {
        Value::Str(s) => s.get().to_string(),
        other => panic!("expected a string, got {other:?}"),
    }
}

#[test]
fn extenders_reports_the_types_extending_a_protocol() {
    let (_g, mut env) = make_env();

    eval_in(
        &mut env,
        r#"
        (defprotocol IShape (area [s]))
        (defprotocol IUntouched (nothing [s]))
        (defrecord Sq [n])
        (defrecord Tri [b h])
        "#,
    );

    // (1) nil, not (), before anything extends it.
    assert_eq!(printed(&mut env, "(extenders IUntouched)"), "nil");
    assert_eq!(printed(&mut env, "(nil? (extenders IUntouched))"), "true");

    eval_in(
        &mut env,
        r#"
        (extend-type Sq IShape (area [s] (* (:n s) (:n s))))
        (extend-type Tri IShape (area [s] (/ (* (:b s) (:h s)) 2)))
        "#,
    );

    // (2) every extending type, in a stable order.
    assert_eq!(printed(&mut env, "(extenders IShape)"), "(Sq Tri)");
    assert_eq!(
        printed(&mut env, "(= (extenders IShape) (extenders IShape))"),
        "true"
    );

    // The elements are symbols, which is what `extends?` accepts.
    assert_eq!(
        printed(&mut env, "(every? symbol? (extenders IShape))"),
        "true"
    );

    // (3) the two views of the registry agree.
    assert_eq!(
        printed(
            &mut env,
            "(every? (fn [t] (extends? IShape t)) (extenders IShape))"
        ),
        "true"
    );

    // Extending a protocol later shows up; the answer is not cached.
    eval_in(
        &mut env,
        "(defrecord Circle [r])
         (extend-type Circle IShape (area [s] (* 3 (:r s) (:r s))))",
    );
    assert_eq!(printed(&mut env, "(extenders IShape)"), "(Circle Sq Tri)");

    // A protocol nobody touched is still nil, not polluted by its neighbours.
    assert_eq!(printed(&mut env, "(extenders IUntouched)"), "nil");
}

#[test]
fn extenders_refuses_a_non_protocol() {
    let (_g, mut env) = make_env();
    let mut parser = Parser::new("(extenders 42)".to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let err = cljrs_runtime::interp::eval::eval(&forms[0], &mut env)
        .expect_err("extenders must reject a non-protocol");
    let msg = format!("{err:?}");
    assert!(
        msg.contains("protocol"),
        "error should name the expected type, got: {msg}"
    );
}
