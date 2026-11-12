//! A closure with an unused factory parameter must reach both IR and JIT (#487).

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval(source: &str, env: &mut Env) -> Value {
    let forms = Parser::new(source.into(), "<unused-capture-test>".into())
        .parse_all()
        .expect("parse");
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::tiered::eval(form, env).expect("eval");
    }
    result
}

#[test]
fn factory_closure_reaches_ir_and_native_code() {
    // Process-wide thresholds: keep this integration binary to one test.
    cljrs_runtime::tiered::jit_state::set_ir_threshold(1);
    cljrs_runtime::tiered::jit_state::set_jit_threshold(1);
    let _mutator = cljrs_gc::register_mutator();
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .build()
        .expect("runtime");
    cljrs_compiler::jit::install(&runtime);
    let mut env = Env::new(runtime.into_globals(), "user");
    eval(
        "(defn factory [x] (fn [v] (+ v 1)))
         (def f (factory 100))",
        &mut env,
    );
    let Value::Fn(f) = env.lookup("f").expect("f") else {
        panic!("expected function");
    };
    let f = f.get();
    assert!(f.closed_over_names.is_empty());
    let arity_id = f.arities[0].ir_arity_id;
    let deadline = Instant::now() + Duration::from_secs(15);
    while env.globals.jit().get_native_fn(arity_id).is_none() {
        assert!(Instant::now() < deadline, "closure never reached JIT");
        assert_eq!(eval("(f 41)", &mut env), Value::Long(42));
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(env.globals.ir_cache().get(arity_id).is_some());
    assert_eq!(
        eval("(reduce (fn [s i] (+ s (f i))) 0 (range 10000))", &mut env),
        Value::Long(50005000)
    );
}
