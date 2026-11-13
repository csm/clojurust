//! Exercise the browser's colored evaluator directly, even on native targets.
//! Fiber evaluation uses the synchronous literal handlers and cannot catch #494.
#![cfg(all(feature = "colored-eval", not(feature = "no-gc")))]

use std::cell::Cell;
use std::future::Future;
use std::rc::Rc;
use std::task::{Context, Poll};

use cljrs_async::eval_async::eval_async;
use cljrs_gc::{GcPtr, MarkVisitor, Trace};
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::gc_roots::{force_collect, push_env_root};
use cljrs_runtime::{ExecutionMode, Runtime};
use cljrs_value::{Arity, CljxPromise, NativeFn, NativeObject, Value, gc_native_object};

#[derive(Debug)]
struct Probe(Rc<Cell<usize>>);

impl Trace for Probe {
    fn trace(&self, _: &mut MarkVisitor) {}
}

impl NativeObject for Probe {
    fn type_tag(&self) -> &str {
        "CollectionProbe"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

fn check_literal(source: &str, marker_count: usize, suspend: bool) {
    let runtime = Runtime::builder()
        .execution_mode(ExecutionMode::TreeWalk)
        .build()
        .unwrap();
    let globals = runtime.globals().clone();
    let dropped = Rc::new(Cell::new(0));
    let observed = dropped.clone();
    globals.intern(
        "user",
        "make-probe".into(),
        Value::NativeFunction(GcPtr::new(NativeFn::with_closure(
            "make-probe",
            Arity::Fixed(0),
            move |_| {
                Ok(Value::NativeObject(gc_native_object(Probe(
                    observed.clone(),
                ))))
            },
        ))),
    );
    let observed = dropped.clone();
    globals.intern(
        "user",
        "collect-check".into(),
        Value::NativeFunction(GcPtr::new(NativeFn::with_closure(
            "collect-check",
            Arity::Fixed(0),
            move |_| {
                // Exhaust the collector's unreachable-object grace period.
                for _ in 0..3 {
                    cljrs_gc::request_gc();
                    cljrs_runtime::env::gc_roots::async_gc_collect();
                }
                // Check before the literal touches a potentially freed marker.
                assert_eq!(observed.get(), 0, "GC reclaimed an accumulated element");
                Ok(Value::Long(42))
            },
        ))),
    );
    let mut env = Env::new(globals, "user");
    let _env_root = push_env_root(&env);
    let marker = cljrs_reader::Parser::new(
        "(defn marker [] [(str \"mark\" \"er\") (make-probe)])".into(),
        "<collection-roots>".into(),
    )
    .parse_one()
    .unwrap()
    .unwrap();
    cljrs_runtime::interp::eval::eval(&marker, &mut env).unwrap();
    env.push_frame();
    let resume = GcPtr::new(CljxPromise::new());
    env.bind("resume".into(), Value::Promise(resume.clone()));
    let form = cljrs_reader::Parser::new(source.into(), "<collection-roots>".into())
        .parse_one()
        .unwrap()
        .unwrap();
    // Each marker's Clojure call frame has closed before collection, so its
    // nested vector and probe must survive solely through the literal's roots.
    let mut evaluation = Box::pin(eval_async(&form, &mut env));
    if suspend {
        let mut cx = Context::from_waker(futures_util::task::noop_waker_ref());
        let polled = evaluation.as_mut().poll(&mut cx);
        assert!(matches!(polled, Poll::Pending), "{polled:?}");
        let collector_env = Env::new(runtime.globals().clone(), "user");
        for _ in 0..3 {
            force_collect(&collector_env);
        }
        assert_eq!(dropped.get(), 0, "GC reclaimed an element across await");
        resume.get().deliver(Value::Long(42));
    }
    let executor = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let value = executor.block_on(evaluation).unwrap();
    assert_eq!(dropped.get(), 0);
    let printed = format!("{value}");
    assert_eq!(printed.matches("\"marker\"").count(), marker_count);
    assert!(printed.contains("42"), "{printed}");
}

#[test]
fn vector_elements_survive_later_collection() {
    check_literal("[(marker) (collect-check)]", 1, false);
}

#[test]
fn map_keys_and_values_survive_later_collection() {
    check_literal(
        "{(marker) (collect-check) :value (marker) :trigger (collect-check)}",
        2,
        false,
    );
}

#[test]
fn set_elements_survive_later_collection() {
    check_literal("#{(marker) (collect-check)}", 1, false);
}

#[test]
fn vector_elements_survive_await() {
    check_literal("[(marker) (await resume)]", 1, true);
}

#[test]
fn map_keys_and_values_survive_await() {
    check_literal("{(marker) (marker) :trigger (await resume)}", 2, true);
}

#[test]
fn set_elements_survive_await() {
    check_literal("#{(marker) (await resume)}", 1, true);
}
