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

#[test]
fn cancellation_during_native_finally_parks_stays_inside_abi_boundaries() {
    const CHILD: &str = "CLJRS_CANCEL_FINALLY_ABI";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "cancellation_during_native_finally_parks_stays_inside_abi_boundaries",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    use cljrs_compiler::rt_abi;
    use cljrs_gc::GcPtr;
    use cljrs_value::{Arity, NativeFn, ValueError};
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = cljrs_runtime::Runtime::builder().build().unwrap();
            let globals = runtime.into_globals();
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            tokio::task::LocalSet::new().block_on(&rt, async {
                for kind in 0..3 {
                    let attempted = Rc::new(Cell::new(false));
                    let completed = Rc::new(Cell::new(false));
                    let flag = attempted.clone();
                    let done = completed.clone();
                    let body_globals = globals.clone();
                    let task =
                        fiber::spawn(move || {
                            let _ctx = cljrs_runtime::env::callback::install_eval_context_guard(
                                body_globals,
                                "user".into(),
                            );
                            let body = Value::NativeFunction(GcPtr::new(NativeFn::with_closure(
                                "spin",
                                Arity::Fixed(0),
                                |_| {
                                    while rt_abi::rt_safepoint() != 0 {}
                                    Ok(Value::Nil)
                                },
                            )));
                            let finally = Value::NativeFunction(GcPtr::new(
                                NativeFn::with_closure("cleanup", Arity::Fixed(0), move |_| {
                                    flag.set(true);
                                    match kind {
                                        0 => cljrs_value::parking::sleep(Duration::from_secs(60)),
                                        1 => {
                                            cljrs_value::parking::block_on(
                                                cljrs_async::channel::CljChannel::new(0).take(),
                                            );
                                        }
                                        _ => {
                                            // This infallible wait is directly inside an extern-C
                                            // bridge, rather than inside another NativeFn wrapper.
                                            let promise = Value::Promise(GcPtr::new(
                                                cljrs_value::CljxPromise::new(),
                                            ));
                                            unsafe {
                                                rt_abi::rt_deref(&promise);
                                            }
                                            assert!(matches!(
                                                rt_abi::take_pending_resource_error(),
                                                Some(ValueError::Cancelled)
                                            ));
                                            return Err(ValueError::Cancelled);
                                        }
                                    }
                                    panic!("cancelled cleanup resumed after an explicit park")
                                }),
                            ));
                            let roots = [body, finally];
                            let _roots = cljrs_runtime::env::gc_roots::root_values(&roots);
                            unsafe {
                                rt_abi::rt_try(&roots[0], &Value::Nil, &roots[1]);
                            }
                            assert!(matches!(
                                rt_abi::take_pending_resource_error(),
                                Some(ValueError::Cancelled)
                            ));
                            assert_eq!(rt_abi::rt_error_pending(), 0);
                            assert!(rt_abi::take_pending_exception_value().is_none());
                            done.set(true);
                            Err(cljrs_runtime::env::error::EvalError::Cancelled)
                        })
                        .unwrap();
                    let abort = task.abort_handle();
                    let peer = fiber::spawn(move || {
                        abort.abort();
                        Ok(Value::Nil)
                    })
                    .unwrap();
                    assert!(task.await.err().unwrap().is_cancelled());
                    peer.await.unwrap().into_result().unwrap();
                    assert!(attempted.get(), "finally was skipped: {kind}");
                    assert!(
                        completed.get(),
                        "cancellation escaped the native ABI: {kind}"
                    );
                    assert_eq!(
                        rt_abi::rt_error_pending(),
                        0,
                        "fiber leaked its error flag to the carrier"
                    );
                }
            });
        })
        .unwrap()
        .join()
        .unwrap();
}
