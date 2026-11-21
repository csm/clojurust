//! #420: realizing a lazy input can collect between transducer
//! reducing-function calls in `(into to xform from)`. The transformed
//! reducer and the accumulator must stay rooted across those collections.
#![cfg(not(feature = "no-gc"))]
#![allow(clippy::result_large_err)] // `with_eval_context` returns `EvalError`.

use cljrs_gc::GcPtr;
use cljrs_reader::Parser;
use cljrs_runtime::env::{callback, env::Env, gc_roots};
use cljrs_value::{Arity, NativeFn, Value};

#[test]
fn into_transducer_survives_collection_between_steps() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _mutator = cljrs_gc::register_mutator();
            let globals = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                .build()
                .unwrap()
                .into_globals();
            globals.intern(
                "user",
                "gc-id".into(),
                Value::NativeFunction(GcPtr::new(NativeFn::new(
                    "gc-id",
                    Arity::Fixed(1),
                    |args| {
                        callback::with_eval_context(|env| {
                            // Sweeping grants a grace cycle; collect twice so
                            // an unrooted value cannot survive by accident.
                            gc_roots::force_collect(env);
                            gc_roots::force_collect(env);
                            Ok(())
                        })
                        .unwrap();
                        Ok(args[0].clone())
                    },
                ))),
            );
            let mut env = Env::new(globals, "user");
            for source in [
                r#"(let [v (into [] (map inc) (map gc-id (range 100)))]
                     (and (= 100 (count v)) (= 1 (first v))
                          (= 100 (peek v)) (= 5050 (reduce + v))))"#,
                r#"(= [1 2 3 4 5]
                      (into [] (comp (map inc) (take 5)) (map gc-id (range 100))))"#,
                r#"(= [[0 1 2] [3 4]]
                      (into [] (partition-all 3) (map gc-id (range 5))))"#,
                r#"(= :caught
                      (try (into [] (map inc)
                                 (map (fn [x] (gc-id x)
                                        (if (= x 5) (throw (ex-info "stop" {})) x))
                                      (range 10)))
                           (catch :default e :caught)))"#,
            ] {
                let form = Parser::new(source.into(), "<into-transducer-gc>".into())
                    .parse_all()
                    .unwrap()
                    .remove(0);
                let result = cljrs_runtime::tiered::eval(&form, &mut env).unwrap();
                assert_eq!(result, Value::Bool(true), "{source}");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
