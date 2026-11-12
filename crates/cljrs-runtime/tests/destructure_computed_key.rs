//! Issue #492 — a map-destructuring lookup key is an expression and must be
//! evaluated: `(let [{v (identity :value)} {:value 2}] v)` is `2`.

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval(src: &str) -> Value {
    let globals = cljrs_runtime::Runtime::builder()
        .build()
        .expect("runtime")
        .into_globals();
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new(src.to_string(), "<test>".to_string());
    let mut result = Value::Nil;
    for form in parser.parse_all().expect("parse error") {
        result = cljrs_runtime::interp::eval::eval(&form, &mut env).expect("eval error");
    }
    result
}

#[test]
fn computed_key_is_evaluated() {
    assert!(matches!(
        eval("(let [{v (identity :value)} {:value 2}] v)"),
        Value::Long(2)
    ));
}

#[test]
fn symbol_key_is_evaluated() {
    assert!(matches!(
        eval("(let [k :value {v k} {:value 2}] v)"),
        Value::Long(2)
    ));
}

#[test]
fn literal_key_still_works() {
    assert!(matches!(
        eval("(let [{v :value} {:value 2}] v)"),
        Value::Long(2)
    ));
}
