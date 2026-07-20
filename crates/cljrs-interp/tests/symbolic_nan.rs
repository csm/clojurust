//! Regression coverage for symbolic NaN macro expansion (issue #280).

use cljrs_env::env::Env;
use cljrs_reader::Parser;
use cljrs_value::Value;

#[test]
fn symbolic_nan_evaluates_without_hanging() {
    let globals = cljrs_interp::standard_env(None, None, None);
    let mut env = Env::new(globals, "user");
    let mut parser = Parser::new("(NaN? ##NaN)".to_string(), "<test>".to_string());
    let form = parser
        .parse_all()
        .expect("parse error")
        .into_iter()
        .next()
        .expect("missing form");

    assert_eq!(
        cljrs_interp::eval::eval(&form, &mut env).expect("eval error"),
        Value::Bool(true)
    );
}
