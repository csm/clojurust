//! Phase H dispatch test: a registered compiled poll function takes over from
//! the tree-walking `eval_async` fallback when an `^:async` function is called.

use cljrs_async::state_machine::{CljxStateMachine, POLL_READY, register_poll_fn};
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

fn parse_one(src: &str) -> cljrs_reader::Form {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all()
        .expect("parse")
        .into_iter()
        .next()
        .expect("form")
}

/// A stand-in for compiled output: ignores its inputs and completes
/// immediately with a sentinel `999`, distinguishable from what the
/// interpreter would compute (`(await 5)` = 5).
extern "C" fn sentinel_poll(sm: *mut CljxStateMachine, out: *mut *const Value) -> i32 {
    let sm = unsafe { &mut *sm };
    sm.slots[0] = Value::Long(999);
    unsafe { *out = &sm.slots[0] as *const Value };
    POLL_READY
}

#[test]
fn registered_poll_fn_takes_over_dispatch() {
    let globals = cljrs_interp::standard_env(None, None, None);
    cljrs_async::init(&globals);

    block_on_local(async move {
        let mut env = Env::new(globals, "user");

        // Define a normal `^:async` function whose interpreted body returns its
        // awaited argument unchanged.
        for form in Parser::new(
            "(defn ^:async foo [x] (await x))".to_string(),
            "<test>".to_string(),
        )
        .parse_all()
        .unwrap()
        {
            cljrs_interp::eval::eval(&form, &mut env).unwrap();
        }

        // Register a compiled poll function for user/foo at arity 1.
        register_poll_fn("user", "foo", 1, sentinel_poll, 2);

        // Calling foo now spawns the native state machine, not the interpreter:
        // the result is the sentinel 999, not the awaited 5.
        let fut = cljrs_interp::eval::eval(&parse_one("(foo 5)"), &mut env).unwrap();
        assert!(matches!(fut, Value::Future(_)), "expected a Future");
        let result = cljrs_async::await_value(fut).await.expect("resolves");
        assert!(
            matches!(result, Value::Long(999)),
            "expected the compiled poll fn's sentinel 999, got {result:?}"
        );
    });
}
