use cljrs_env::env::Env;
use cljrs_env::error::EvalError;
use cljrs_reader::Parser;
use cljrs_value::Value;

fn form(src: &str) -> cljrs_reader::Form {
    Parser::new(src.to_owned(), "<gas-test>".to_owned())
        .parse_all()
        .expect("parse")
        .remove(0)
}

fn env() -> Env {
    Env::new(cljrs_interp::standard_env(None, None, None), "user")
}

#[test]
fn ordinary_eval_is_unmetered() {
    assert_eq!(
        cljrs_interp::eval::eval(&form("(+ 1 2)"), &mut env()).unwrap(),
        Value::Long(3)
    );
}

#[test]
fn zero_credits_exhaust_before_evaluation() {
    assert!(matches!(
        cljrs_interp::eval::eval_with_gas(&form("1"), &mut env(), 0),
        Err(EvalError::GasExhausted)
    ));
}

#[test]
fn recursive_work_exhausts_and_cannot_be_caught() {
    let source = "(try (loop* [x 0] (recur (inc x))) (catch :default e :caught))";
    assert!(matches!(
        cljrs_interp::eval::eval_with_gas(&form(source), &mut env(), 40),
        Err(EvalError::GasExhausted)
    ));
}
