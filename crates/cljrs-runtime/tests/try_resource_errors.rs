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
                        assert!(
                            matches!(result, Err(EvalError::Cancelled)),
                            "{}",
                            describe(&result)
                        )
                    }
                    _ => assert!(
                        matches!(result, Err(EvalError::GasExhausted)),
                        "{}",
                        describe(&result)
                    ),
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn interrupted_futures_reach_healthy_awaiters_as_catchable_errors() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let globals = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                .build()
                .unwrap()
                .into_globals();
            let future = cljrs_value::CljxFuture::new();
            *future.state.lock().unwrap() = cljrs_value::FutureState::Interrupted;
            globals.intern(
                "user",
                "interrupted".into(),
                Value::Future(cljrs_gc::GcPtr::new(future)),
            );
            let form = Parser::new(
                "(try @interrupted (catch :default e :fallback))".into(),
                "<interrupted>".into(),
            )
            .parse_all()
            .unwrap()
            .remove(0);
            let mut env = cljrs_runtime::tiered::Env::new(globals, "user");
            let result = cljrs_runtime::tiered::eval(&form, &mut env);
            assert!(
                matches!(&result, Ok(Value::Keyword(k)) if k.get().name.as_ref() == "fallback"),
                "{}",
                describe(&result)
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Summarize a result without printing payload contents.
fn describe(result: &Result<Value, EvalError>) -> &'static str {
    match result {
        Ok(_) => "a value",
        Err(EvalError::Cancelled) => "Cancelled",
        Err(EvalError::GasExhausted) => "GasExhausted",
        Err(EvalError::Thrown(_)) => "a thrown value",
        Err(_) => "another error",
    }
}
