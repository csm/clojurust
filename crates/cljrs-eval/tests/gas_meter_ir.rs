use cljrs_env::error::EvalError;
use cljrs_reader::Parser;

fn forms(src: &str) -> Vec<cljrs_reader::Form> {
    Parser::new(src.to_owned(), "<gas-ir-test>".to_owned())
        .parse_all()
        .expect("parse")
}

fn env() -> cljrs_eval::Env {
    cljrs_eval::Env::new(cljrs_eval::standard_env(), "user")
}

#[test]
fn ir_function_exhausts_meter() {
    cljrs_eval::force_eager_lowering();
    let mut env = env();
    for form in forms("(defn spin [n] (if (= n 0) 0 (spin (dec n))))") {
        cljrs_eval::eval(&form, &mut env).expect("defn");
    }
    let call = forms("(spin 100000)").remove(0);
    assert!(matches!(
        cljrs_eval::eval_with_gas(&call, &mut env, 100),
        Err(EvalError::GasExhausted)
    ));
}

#[test]
fn nested_meter_charges_outer_budget() {
    let mut env = env();
    let outer = cljrs_env::gas::GasMeter::new(20);
    let _outer_guard = cljrs_env::gas::GasGuard::install(outer.clone());
    let form = forms("1").remove(0);
    cljrs_eval::eval_with_gas(&form, &mut env, 10).expect("inner eval");
    assert!(
        outer.remaining() < 20,
        "inner eval did not charge outer meter"
    );
}
