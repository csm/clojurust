//! `System/getenv` — reading the process environment.
//!
//! cljrs had no environment accessor at all: `System/getenv` was an unbound
//! symbol and nothing under `crates/` registered an equivalent. A `.cljrs`
//! program could not honour `TMPDIR`, `HOME`, `NO_COLOR` or any other ambient
//! configuration, which is why hive-universe's `native/universe/cli.cljrs`
//! hardcodes `/tmp` where its `.cljw` twin reads the variable.
//!
//! These read variables the test process already has rather than setting any:
//! `std::env::set_var` is `unsafe` under edition 2024 because the environment
//! is process-global, and a test that mutates it races every other test in the
//! same binary.

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

fn value_of(src: &str) -> Value {
    eval_fresh(src).unwrap_or_else(|e| panic!("{src}\n{e}"))
}

/// A variable the test process is overwhelmingly likely to have, and which the
/// harness itself does not touch.
const PRESENT: &str = "PATH";

/// The one-arg form answers with the value the host reports.
#[test]
fn reads_a_variable_that_is_set() {
    let expected = std::env::var(PRESENT).expect("test host has no PATH");
    assert_eq!(
        value_of(&format!(r#"(System/getenv "{PRESENT}")"#)),
        Value::string(expected)
    );
}

/// Unset is `nil`, not an error and not an empty string — the JVM's answer.
#[test]
fn an_unset_variable_is_nil() {
    let absent = "CLJRS_GETENV_TEST_DEFINITELY_UNSET";
    assert!(std::env::var(absent).is_err(), "test premise broken");
    assert_eq!(
        value_of(&format!(r#"(System/getenv "{absent}")"#)),
        Value::Nil
    );
}

/// The no-arg form is the whole environment as a map.
#[test]
fn the_no_arg_form_returns_the_environment_as_a_map() {
    let expected = std::env::var(PRESENT).expect("test host has no PATH");
    assert_eq!(
        value_of(&format!(r#"(get (System/getenv) "{PRESENT}")"#)),
        Value::string(expected)
    );
}

/// A non-string name is a type error rather than a silent nil, so a keyword
/// slipping in where a name belongs is reported at the call.
#[test]
fn a_non_string_name_is_a_type_error() {
    let err = eval_fresh("(System/getenv :PATH)").expect_err("keyword name");
    assert!(
        err.contains("string") || err.contains("WrongType"),
        "unhelpful error: {err}"
    );
}
