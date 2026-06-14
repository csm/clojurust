//! Phase H activation test: with the JIT initialised, calling an `^:async`
//! function compiles its body to a native state machine on first dispatch and
//! runs it (instead of the `eval_async` tree-walker), producing the same result.

#![cfg(not(feature = "no-gc"))]

use cljrs_async::state_machine::lookup_poll_fn;
use cljrs_env::env::Env;
use cljrs_reader::Parser;
use cljrs_value::Value;

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

fn eval_all(src: &str, env: &mut Env) -> Value {
    let mut last = Value::Nil;
    for form in Parser::new(src.to_string(), "<test>".to_string())
        .parse_all()
        .expect("parse")
    {
        last = cljrs_interp::eval::eval(&form, env).expect("eval");
    }
    last
}

#[test]
fn async_fn_compiles_to_native_state_machine_on_call() {
    cljrs_jit::init();
    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_stdlib::standard_env();

    // Wait for the background compiler-namespace load (mirrors versioned_jit.rs).
    while !globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    block_on_local(async move {
        cljrs_async::init(&globals);
        let mut env = Env::new(globals, "user");

        eval_all("(defn ^:async f [x] (+ (await x) 1))", &mut env);

        // Grab the arity id so we can confirm native compilation happened.
        let f = eval_all("f", &mut env);
        let arity_id = match &f {
            Value::Fn(g) => g.get().arities[0].ir_arity_id,
            other => panic!("expected fn, got {other:?}"),
        };

        // Before the first call, nothing is compiled.
        assert!(lookup_poll_fn(arity_id).is_none(), "not compiled yet");

        // First call triggers JIT compilation of the poll function, then runs it.
        let fut = cljrs_interp::eval::eval(
            &Parser::new("(f 41)".to_string(), "<test>".to_string())
                .parse_all()
                .unwrap()[0],
            &mut env,
        )
        .unwrap();
        let result = cljrs_async::await_value(fut).await.expect("resolves");
        assert!(matches!(result, Value::Long(42)), "got {result:?}");

        // The arity is now backed by a compiled native poll function.
        assert!(
            lookup_poll_fn(arity_id).is_some(),
            "async fn should have been JIT-compiled on first dispatch"
        );
    });
}
