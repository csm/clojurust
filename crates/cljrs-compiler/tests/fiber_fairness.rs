//! Exercise safepoints in published machine code, without a parking builtin.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use cljrs_async::fiber;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

fn eval(source: &str, env: &mut Env) -> Value {
    let form = cljrs_reader::Parser::new(source.into(), "<compiled-fairness>".into())
        .parse_all()
        .unwrap()
        .remove(0);
    cljrs_runtime::tiered::eval(&form, env).unwrap()
}

#[test]
fn compiled_cpu_loop_yields_to_a_ready_fiber() {
    cljrs_runtime::tiered::jit_state::set_ir_threshold(1);
    cljrs_runtime::tiered::jit_state::set_jit_threshold(1);
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            cljrs_compiler::jit::install(&runtime);
            let globals = runtime.into_globals();
            let mut env = Env::new(globals.clone(), "user");
            eval(
                "(defn spin [n] (loop [i n] (if (zero? i) 42 (recur (dec i)))))",
                &mut env,
            );
            let arity_id = match globals.lookup_in_ns("user", "spin").unwrap() {
                Value::Fn(f) => f.get().arities[0].ir_arity_id,
                _ => unreachable!(),
            };
            let deadline = Instant::now() + Duration::from_secs(15);
            while globals.jit().get_native_fn(arity_id).is_none() {
                assert!(Instant::now() < deadline, "JIT did not publish spin");
                eval("(spin 10)", &mut env);
                std::thread::sleep(Duration::from_millis(1));
            }
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async {
                let finished = Rc::new(Cell::new(false));
                let flag = finished.clone();
                let spinner_globals = globals.clone();
                let spinner = fiber::spawn(move || {
                    let value = eval("(spin 5000000)", &mut Env::new(spinner_globals, "user"));
                    flag.set(true);
                    Ok(value)
                })
                .unwrap();
                let sibling = fiber::spawn(move || Ok(Value::Bool(!finished.get()))).unwrap();
                assert_eq!(
                    spinner.await.unwrap().into_result().unwrap(),
                    Value::Long(42)
                );
                assert_eq!(
                    sibling.await.unwrap().into_result().unwrap(),
                    Value::Bool(true)
                );
                let cancelled = fiber::spawn(move || {
                    let form =
                        cljrs_reader::Parser::new("(spin 5000000)".into(), "<cancel-jit>".into())
                            .parse_all()
                            .unwrap()
                            .remove(0);
                    cljrs_runtime::tiered::eval(&form, &mut Env::new(globals, "user"))
                })
                .unwrap();
                let abort = cancelled.abort_handle();
                let canceller = fiber::spawn(move || {
                    abort.abort();
                    Ok(Value::Nil)
                })
                .unwrap();
                assert!(cancelled.await.err().unwrap().is_cancelled());
                canceller.await.unwrap().into_result().unwrap();
            });
        })
        .unwrap()
        .join()
        .unwrap();
}
