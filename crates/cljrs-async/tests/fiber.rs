#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::result_large_err)]

use std::cell::Cell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll};

use cljrs_async::fiber::{self, Fiber};
use cljrs_gc::{GcPtr, Trace};
use cljrs_runtime::env::gc_roots::{root_option_values, root_value, root_values};
use cljrs_runtime::tiered::jit_state::{current_jit_epoch, live_epochs, push_jit_frame};
use cljrs_value::Value;

fn run(future: impl Future<Output = ()>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    tokio::task::LocalSet::new().block_on(&rt, future);
}

fn collect() {
    #[cfg(not(feature = "no-gc"))]
    {
        cljrs_gc::request_gc();
        cljrs_runtime::env::gc_roots::async_gc_collect();
    }
}

struct CollectThenYield(bool);
impl Future for CollectThenYield {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        assert!(fiber::current().is_none());
        collect();
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    }
}

#[test]
fn parks_through_native_frames_and_borrows_stack_locals() {
    fn native(depth: usize, value: &mut i64) {
        if depth > 0 {
            native(depth - 1, value);
        } else {
            fiber::park_on(async {
                tokio::task::yield_now().await;
                *value += 1;
            });
        }
    }
    run(async {
        assert!(fiber::current().is_none());
        let thread = std::thread::current().id();
        let task = fiber::spawn(move || {
            let id = fiber::current().unwrap();
            let mut value = 40;
            native(10, &mut value);
            native(10, &mut value);
            assert_eq!(fiber::current(), Some(id));
            assert_eq!(std::thread::current().id(), thread);
            Ok(Value::Long(value))
        })
        .unwrap();
        assert_eq!(task.await.unwrap().into_result().unwrap(), Value::Long(42));
        assert!(fiber::current().is_none());
    });
}

#[test]
fn panics_and_pending_future_panics_become_eval_errors() {
    run(async {
        let body = fiber::spawn(|| panic!("body panic")).unwrap();
        let wait = fiber::spawn(|| {
            fiber::park_on(async { panic!("poll panic") });
            Ok(Value::Nil)
        })
        .unwrap();
        assert!(
            body.await
                .unwrap()
                .into_result()
                .unwrap_err()
                .to_string()
                .contains("body panic")
        );
        assert!(
            wait.await
                .unwrap()
                .into_result()
                .unwrap_err()
                .to_string()
                .contains("poll panic")
        );
        let good = fiber::spawn(|| Ok(Value::Long(42))).unwrap();
        assert_eq!(good.await.unwrap().into_result().unwrap(), Value::Long(42));
    });
}

struct OnDrop(Rc<Cell<bool>>);
impl Drop for OnDrop {
    fn drop(&mut self) {
        assert!(fiber::current().is_some());
        self.0.set(true);
    }
}

#[test]
fn cancellation_unwinds_with_fiber_state_and_removes_epochs() {
    run(async {
        let dropped = Rc::new(Cell::new(false));
        let flag = dropped.clone();
        let handle = fiber::spawn(move || {
            let _drop = OnDrop(flag);
            let _frame = push_jit_frame(0xF1BE_0001);
            let value = Value::string("live until cancellation");
            let _root = root_value(&value);
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        assert!(live_epochs().contains(&0xF1BE_0001));
        assert_eq!(current_jit_epoch(), None);
        collect();
        handle.abort();
        assert!(handle.await.is_err());
        assert!(dropped.get());
        assert!(!live_epochs().contains(&0xF1BE_0001));
        collect();
    });
}

#[test]
fn captured_values_and_completed_results_survive_collection() {
    run(async {
        let frame = cljrs_gc::push_alloc_frame();
        let value = Value::string("captured and returned");
        let task = Fiber::with_roots(vec![value.clone()], move || Ok(value))
            .unwrap()
            .into_task();
        drop(frame);
        collect(); // Before first poll: only the fiber's capture roots retain it.
        assert_eq!(
            format!("{}", task.await.unwrap()),
            "\"captured and returned\""
        );
        let handle = fiber::spawn(|| Ok(Value::string("completed result"))).unwrap();
        tokio::task::yield_now().await;
        assert!(handle.is_finished());
        collect(); // Result is queued in JoinHandle, with no live coroutine stack.
        assert_eq!(
            format!("{}", handle.await.unwrap().into_result().unwrap()),
            "\"completed result\""
        );
    });
}

// A region object that points to a GC-heap Value; the region is its only root.
struct RegionEdge(Value);
impl Trace for RegionEdge {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.0.trace(visitor);
    }
}

#[test]
fn five_thousand_fibers_keep_shadow_roots_and_regions_alive() {
    run(async {
        let mut tasks = Vec::new();
        for i in 0..5_000 {
            tasks.push(
                fiber::spawn(move || {
                    let alloc = cljrs_gc::push_alloc_frame();
                    #[cfg(feature = "no-gc")]
                    let _scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
                    let values = (0..16)
                        .map(|j| Value::string(format!("{i}/{j}")))
                        .collect::<Vec<_>>();
                    let _values = root_values(&values);
                    let options =
                        vec![Some(Value::string(format!("option/{i}")))].into_boxed_slice();
                    let _options = root_option_values(&options);
                    let edge = RegionEdge(Value::string(format!("region/{i}")));
                    let mut region = cljrs_gc::region::Region::new();
                    let edge = region.alloc(edge);
                    let region_guard = unsafe { cljrs_gc::region::RegionGuard::new(&mut region) };
                    alloc.release(); // Exercise roots other than ALLOC_ROOTS.
                    for turn in 0..2 {
                        let _alloc_roots = cljrs_gc::push_alloc_frame();
                        let allocation_only = GcPtr::new(format!("allocation/{i}/{turn}"));
                        fiber::park_on(CollectThenYield(false));
                        assert_eq!(allocation_only.get(), &format!("allocation/{i}/{turn}"));
                        assert_eq!(format!("{}", values[15]), format!("\"{i}/15\""));
                        assert_eq!(
                            format!("{}", options[0].as_ref().unwrap()),
                            format!("\"option/{i}\"")
                        );
                        assert_eq!(format!("{}", edge.get().0), format!("\"region/{i}\""));
                    }
                    drop(region_guard);
                    Ok(Value::Long(i))
                })
                .unwrap(),
            );
        }
        for (i, task) in tasks.into_iter().enumerate() {
            assert_eq!(
                task.await.unwrap().into_result().unwrap(),
                Value::Long(i as i64)
            );
        }
        collect(); // No stale stack pointers may remain after stack pooling.
    });
}

#[test]
fn synchronous_evaluations_keep_bindings_output_and_context_separate() {
    use cljrs_runtime::env::callback::capture_eval_context;
    use cljrs_runtime::env::env::Env;
    use cljrs_value::{Arity, NativeFn};
    run(async {
        let globals = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
            .build()
            .unwrap()
            .into_globals();
        let mut env = Env::new(globals.clone(), "user");
        fn eval(source: &str, env: &mut Env) -> cljrs_runtime::env::error::EvalResult {
            let forms = cljrs_reader::Parser::new(source.into(), "<fiber-test>".into())
                .parse_all()
                .unwrap();
            cljrs_runtime::interp::eval::eval(&forms[0], env)
        }
        eval("(def ^:dynamic *fiber-who* :root)", &mut env).unwrap();
        let park = NativeFn::with_closure("park!", Arity::Fixed(0), |_| {
            assert!(capture_eval_context().is_some());
            fiber::park_on(CollectThenYield(false));
            assert!(capture_eval_context().is_some());
            Ok(Value::Nil)
        });
        globals.intern(
            "user",
            "park!".into(),
            Value::NativeFunction(GcPtr::new(park)),
        );
        let mut tasks = Vec::new();
        for who in [":a", ":b"] {
            let globals = globals.clone();
            tasks.push(fiber::spawn(move || {
                let mut env = Env::new(globals, "user");
                eval(&format!("(binding [*fiber-who* {who}] (with-out-str (print *fiber-who*) (park!) (print *fiber-who*)))"), &mut env)
            }).unwrap());
        }
        for (task, expected) in tasks.into_iter().zip([":a:a", ":b:b"]) {
            assert_eq!(
                task.await.unwrap().into_result().unwrap(),
                Value::string(expected)
            );
        }
        assert!(capture_eval_context().is_none());
        assert_eq!(
            format!("{}", eval("*fiber-who*", &mut env).unwrap()),
            ":root"
        );
    });
}

#[test]
fn carrier_roots_survive_collection_inside_a_fiber() {
    run(async {
        let alloc = cljrs_gc::push_alloc_frame();
        let carrier_value = Value::string("carrier root");
        let _root = root_value(&carrier_value);
        alloc.release();
        Fiber::new(|| {
            collect();
            Ok(Value::Nil)
        })
        .unwrap()
        .into_task()
        .await
        .unwrap();
        assert_eq!(format!("{carrier_value}"), "\"carrier root\"");
    });
}

#[test]
fn borrowed_mutator_slot_is_rejected_before_any_switch() {
    use cljrs_gc::mutator::MutatorState;
    run(async {
        let handle = fiber::spawn(|| {
            // A public binding-stack accessor does not expose its borrow, so
            // register a test slot to exercise the same macro contract.
            thread_local! {
                static BORROWED: std::cell::RefCell<Vec<usize>> = const { std::cell::RefCell::new(Vec::new()) };
            }
            cljrs_gc::mutator_slot!("fiber-test/BORROWED", refcell BORROWED: Vec<usize>);
            BORROWED.with(|slot| {
                let _borrow = slot.borrow();
                fiber::park_on(std::future::ready(()));
            });
            Ok(Value::Nil)
        }).unwrap();
        assert!(
            handle
                .await
                .unwrap()
                .into_result()
                .unwrap_err()
                .to_string()
                .contains("borrowed across a context switch")
        );
        // Failed preflight left all slots intact and the next swap succeeds.
        let state = MutatorState::save();
        state.restore();
    });
}

#[cfg(feature = "no-gc")]
#[test]
fn cancellation_releases_fiber_scratch_regions() {
    struct ScratchValue(Rc<Cell<usize>>);
    impl Trace for ScratchValue {
        fn trace(&self, _: &mut cljrs_gc::MarkVisitor) {}
    }
    impl Drop for ScratchValue {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    run(async {
        let dropped = Rc::new(Cell::new(0));
        let count = dropped.clone();
        let handle = fiber::spawn(move || {
            let _scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let _value = GcPtr::new(ScratchValue(count));
            fiber::park_on(std::future::pending::<()>());
            Ok(Value::Nil)
        })
        .unwrap();
        tokio::task::yield_now().await;
        assert_eq!(dropped.get(), 0);
        // A second fiber must allocate into its own scratch context.
        let other_count = dropped.clone();
        fiber::spawn(move || {
            let _scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
            let _value = GcPtr::new(ScratchValue(other_count));
            Ok(Value::Nil)
        })
        .unwrap()
        .await
        .unwrap()
        .into_result()
        .unwrap();
        assert_eq!(dropped.get(), 1);
        handle.abort();
        assert!(handle.await.is_err());
        assert_eq!(dropped.get(), 2);
    });
}
