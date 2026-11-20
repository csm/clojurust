//! Native code that buffers `callback::invoke` results in Rust storage must
//! be able to keep them across later callbacks that collect. A tree-walked
//! (or IR) callee scopes its own allocations, so `invoke` re-roots the result
//! in the caller's allocation frame. Before that, `sort-by` with a keyfn that
//! builds fresh values and a comparator that collects read freed keys.
#![cfg(not(feature = "no-gc"))]
#![allow(clippy::result_large_err)] // `with_eval_context` returns `EvalError`.

use cljrs_gc::GcPtr;
use cljrs_reader::Parser;
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::{callback, env::Env, gc_roots};
use cljrs_value::{Arity, NativeFn, Value};

#[test]
fn buffered_callback_results_survive_collection() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _mutator = cljrs_gc::register_mutator();
            for mode in [ExecutionMode::TreeWalk, ExecutionMode::TieredNoJit] {
                let globals = cljrs_runtime::Runtime::builder()
                    .execution_mode(mode)
                    .build()
                    .unwrap()
                    .into_globals();
                globals.intern(
                    "user",
                    "gc!".into(),
                    Value::NativeFunction(GcPtr::new(NativeFn::new(
                        "gc!",
                        Arity::Fixed(0),
                        |_| {
                            callback::with_eval_context(|env| {
                                gc_roots::force_collect(env);
                                gc_roots::force_collect(env);
                                Ok(())
                            })
                            .unwrap();
                            Ok(Value::Nil)
                        },
                    ))),
                );
                let mut env = Env::new(globals, "user");
                let src = r#"
                  (pr-str
                    (sort-by (fn [x] [(str "k" (- 12 x)) {:v [x]}])
                             (fn [a b] (gc!) (compare (first a) (first b)))
                             (vec (range 12))))"#;
                let form = Parser::new(src.into(), "<callback-retention>".into())
                    .parse_all()
                    .unwrap()
                    .remove(0);
                let result = cljrs_runtime::tiered::eval(&form, &mut env).unwrap();
                assert_eq!(
                    result.to_string(),
                    "\"(11 2 1 0 10 9 8 7 6 5 4 3)\"",
                    "{mode:?}"
                );
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
