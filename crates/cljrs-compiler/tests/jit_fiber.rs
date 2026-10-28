//! Suspend an actual JIT frame, redefine its function, and collect while parked.
//! The superseded machine code must survive until that frame returns.

#![cfg(all(not(target_arch = "wasm32"), not(feature = "no-gc")))]
#![allow(clippy::result_large_err)]

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use cljrs_async::fiber;
use cljrs_compiler::jit::code_cache;
use cljrs_gc::GcPtr;
use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::gc_roots::force_collect;
use cljrs_runtime::tiered::jit_state::{
    current_jit_epoch, live_epochs, set_ir_threshold, set_jit_threshold,
};
use cljrs_value::{Arity, NativeFn, Value};

fn eval(source: &str, env: &mut Env) -> Value {
    let form = Parser::new(source.into(), "<jit-fiber-test>".into())
        .parse_all()
        .expect("parse")
        .remove(0);
    cljrs_runtime::tiered::eval(&form, env).expect("eval")
}

#[test]
fn parked_jit_frame_survives_redefinition_and_collection() {
    // Thresholds and the code cache are process-wide; this binary has one test.
    set_ir_threshold(1);
    set_jit_threshold(3);
    let _mutator = cljrs_gc::register_mutator();
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .build()
        .expect("runtime");
    cljrs_compiler::jit::install(&runtime);
    let globals = runtime.into_globals();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("Tokio runtime");
    tokio::task::LocalSet::new().block_on(&rt, async move {
        let mut env = Env::new(globals.clone(), "user");
        let armed = Rc::new(Cell::new(false));
        let observed = Rc::new(RefCell::new(Vec::new()));
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let (resume_tx, resume_rx) = tokio::sync::oneshot::channel();
        let park = NativeFn::with_closure("park-jit!", Arity::Fixed(0), {
            let armed = armed.clone();
            let observed = observed.clone();
            let entered_tx = RefCell::new(Some(entered_tx));
            let resume_rx = RefCell::new(Some(resume_rx));
            move |_| {
                if armed.get() {
                    // Observe the dispatch path's epoch, without push_jit_frame.
                    observed.borrow_mut().push(current_jit_epoch());
                    let receiver = resume_rx.borrow_mut().take().expect("one park");
                    let _ = entered_tx.borrow_mut().take().unwrap().send(());
                    let _ = fiber::park_on(receiver);
                    observed.borrow_mut().push(current_jit_epoch());
                }
                Ok(Value::Nil)
            }
        });
        globals.intern(
            "user",
            "park-jit!".into(),
            Value::NativeFunction(GcPtr::new(park)),
        );
        eval(
            r#"(defn fiber-jit [x]
                 (let [saved (str "before-" x)]
                   (park-jit!)
                   (str saved "-after")))"#,
            &mut env,
        );
        let arity_id = match globals.lookup_in_ns("user", "fiber-jit").unwrap() {
            Value::Fn(f) => f.get().arities[0].ir_arity_id,
            other => panic!("expected function, got {other:?}"),
        };

        // Wait for native publication explicitly: interpreting the body must
        // never satisfy this regression by accident.
        let deadline = Instant::now() + Duration::from_secs(15);
        let epoch = loop {
            if let Some((_, epoch)) = globals.jit().get_native_fn(arity_id) {
                break epoch;
            }
            assert!(Instant::now() < deadline, "JIT never published fiber-jit");
            assert_eq!(
                eval("(fiber-jit 41)", &mut env),
                Value::string("before-41-after")
            );
            tokio::time::sleep(Duration::from_millis(1)).await;
        };

        armed.set(true);
        let fiber_globals = globals.clone();
        let task = fiber::spawn(move || {
            let mut env = Env::new(fiber_globals, "user");
            let form = Parser::new("(fiber-jit 41)".into(), "<fiber>".into())
                .parse_all()
                .unwrap()
                .remove(0);
            cljrs_runtime::tiered::eval(&form, &mut env)
        })
        .expect("spawn fiber");
        tokio::time::timeout(Duration::from_secs(5), entered_rx)
            .await
            .expect("fiber reached park")
            .expect("entered notification");
        assert_eq!(&*observed.borrow(), &[Some(epoch)]);
        assert_eq!(current_jit_epoch(), None, "carrier has no native frame");
        assert!(live_epochs().contains(&epoch));

        // Redefinition makes the old module eligible for reclamation. A full
        // collection must retain it while its machine stack is suspended.
        let reclaimed_before = code_cache::reclaimed_count();
        eval(r#"(defn fiber-jit [x] "replacement")"#, &mut env);
        assert!(globals.jit().get_native_fn(arity_id).is_none());
        assert_eq!(code_cache::stale_count(), 1);
        force_collect(&env);
        assert_eq!(code_cache::reclaimed_count(), reclaimed_before);
        assert_eq!(code_cache::stale_count(), 1);
        assert!(live_epochs().contains(&epoch));

        resume_tx.send(()).expect("resume parked fiber");
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("fiber returned")
            .expect("fiber task")
            .into_result()
            .expect("JIT result");
        assert_eq!(result, Value::string("before-41-after"));
        assert_eq!(&*observed.borrow(), &[Some(epoch), Some(epoch)]);
        assert!(!live_epochs().contains(&epoch));

        // The old code becomes reclaimable only after returning through the
        // suspended JIT frame. New calls use the replacement definition.
        force_collect(&env);
        assert_eq!(code_cache::stale_count(), 0);
        assert_eq!(code_cache::reclaimed_count(), reclaimed_before + 1);
        assert_eq!(
            eval("(fiber-jit 41)", &mut env),
            Value::string("replacement")
        );
    });
}
