//! Uncatchable resource errors keep their identity through `try`: a throwing
//! `finally` must not replace them with a catchable exception.

use cljrs_reader::Parser;
use cljrs_runtime::env::error::EvalError;
use cljrs_value::{Arity, NativeFn, Value, ValueError};

#[test]
fn throwing_finally_does_not_make_resource_errors_catchable() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            for kind in ["cancelled", "gas"] {
                let globals = cljrs_runtime::Runtime::builder()
                    .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                    .build()
                    .unwrap()
                    .into_globals();
                globals.intern(
                    "user",
                    "fail!".into(),
                    Value::NativeFunction(cljrs_gc::GcPtr::new(NativeFn::with_closure(
                        "fail!",
                        Arity::Fixed(0),
                        move |_| match kind {
                            "cancelled" => Err(ValueError::Cancelled),
                            _ => Err(ValueError::GasExhausted),
                        },
                    ))),
                );
                let source = "(try (try (fail!) (finally (throw (ex-info \"cleanup\" {})))) \
                              (catch :default e :caught))";
                let form = Parser::new(source.into(), "<try-resource>".into())
                    .parse_all()
                    .unwrap()
                    .remove(0);
                let mut env = cljrs_runtime::tiered::Env::new(globals, "user");
                let result = cljrs_runtime::tiered::eval(&form, &mut env);
                match kind {
                    "cancelled" => {
                        assert!(matches!(result, Err(EvalError::Cancelled)), "{result:?}")
                    }
                    _ => assert!(matches!(result, Err(EvalError::GasExhausted)), "{result:?}"),
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
