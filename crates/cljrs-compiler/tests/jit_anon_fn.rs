//! #486: the parent containing #(...) must progress through IR to native code.

#![cfg(not(target_arch = "wasm32"))]

use std::time::{Duration, Instant};

use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_runtime::tiered::jit_state::{set_ir_threshold, set_jit_threshold};
use cljrs_value::Value;

#[test]
fn parent_with_capturing_reader_anon_fn_reaches_native_code() {
    // Process-wide thresholds; this binary contains only this test.
    set_ir_threshold(1);
    set_jit_threshold(3);
    let _mutator = cljrs_gc::register_mutator();
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .build()
        .expect("runtime");
    cljrs_compiler::jit::install(&runtime);
    let globals = runtime.into_globals();
    let mut env = Env::new(globals.clone(), "user");
    let parse = |src: &str| {
        Parser::new(src.into(), "<jit-anon-fn>".into())
            .parse_all()
            .expect("parse")
            .remove(0)
    };
    cljrs_runtime::tiered::eval(&parse("(defn f [x] (mapv #(+ % x) [1 2 3]))"), &mut env)
        .expect("define f");
    let Value::Fn(f) = globals.lookup_in_ns("user", "f").expect("f") else {
        panic!("f is not a function");
    };
    let arity_id = f.get().arities[0].ir_arity_id;
    assert!(
        globals.ir_cache().get(arity_id).is_none(),
        "f must start cold"
    );

    let call = parse("(f 1)");
    let expected = cljrs_runtime::tiered::eval(&parse("[2 3 4]"), &mut env).expect("expected");
    let deadline = Instant::now() + Duration::from_secs(15);
    while globals.jit().get_native_fn(arity_id).is_none() {
        assert!(Instant::now() < deadline, "f never reached native code");
        assert_eq!(
            cljrs_runtime::tiered::eval(&call, &mut env).expect("warm call"),
            expected
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(globals.ir_cache().get(arity_id).is_some());
    // Calls after publication must execute the parent's native entry point.
    for _ in 0..10 {
        assert_eq!(
            cljrs_runtime::tiered::eval(&call, &mut env).expect("native call"),
            expected
        );
    }
}
