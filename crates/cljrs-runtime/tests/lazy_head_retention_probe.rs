//! Diagnostic for #537: how much of a consumed lazy-sequence prefix stays
//! live while different consumers walk it. A native mapper forces two
//! collections every 2,000 elements and records `HEAP.count()`.
//!
//! Run one case per process (ignored by default; it prints observations
//! rather than asserting the current liveness behaviour):
//!
//! ```sh
//! PROBE_CASE=reduce cargo test --release -p cljrs-runtime \
//!   --test lazy_head_retention_probe -- --ignored --nocapture
//! ```
//!
//! Cases: `reduce`, `loop`, `fn`, `recur`, `retained`.
#![allow(clippy::result_large_err)] // `with_eval_context` returns `EvalError`.

use cljrs_gc::{GcPtr, HEAP};
use cljrs_reader::Parser;
use cljrs_runtime::env::{callback, env::Env, gc_roots};
use cljrs_value::{Arity, NativeFn, Value};
use std::sync::{Arc, Mutex};

#[test]
#[ignore = "diagnostic; run with PROBE_CASE=... -- --ignored --nocapture"]
fn live_prefix_probe() {
    std::thread::Builder::new().stack_size(64 * 1024 * 1024).spawn(|| {
        let _mutator = cljrs_gc::register_mutator();
        let globals = cljrs_runtime::Runtime::builder()
            .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
            .build().unwrap().into_globals();
        let samples = Arc::new(Mutex::new(Vec::new()));
        let record = samples.clone();
        globals.intern("user", "probe".into(), Value::NativeFunction(GcPtr::new(
            NativeFn::with_closure("probe", Arity::Fixed(1), move |args| {
                let Value::Long(x) = args[0] else { panic!("long") };
                if x % 2000 == 0 {
                    callback::with_eval_context(|env| {
                        gc_roots::force_collect(env);
                        gc_roots::force_collect(env);
                        Ok(())
                    }).unwrap();
                    record.lock().unwrap().push((x, HEAP.count()));
                }
                Ok(args[0].clone())
            })
        )));
        let mut env = Env::new(globals, "user");
        let case = std::env::var("PROBE_CASE").unwrap_or("reduce".into());
        let source = match case.as_str() {
            "reduce" => "(reduce + (map probe (range 8000)))",
            "loop" => "(loop [s (map probe (range 8000)) acc 0] (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))",
            "fn" => "(defn consume [s] (loop [s s acc 0] (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc))) (consume (map probe (range 8000)))",
            "recur" => "(defn consume [s acc] (if-let [s (seq s)] (recur (next s) (+ acc (first s))) acc)) (consume (map probe (range 8000)) 0)",
            "retained" => "(def source (map probe (range 8000))) (reduce + source)",
            _ => panic!("unknown probe case"),
        };
        let forms = Parser::new(source.into(), "<retention-probe>".into()).parse_all().unwrap();
        let mut result = Value::Nil;
        for form in &forms { result = cljrs_runtime::tiered::eval(form, &mut env).unwrap(); }
        assert_eq!(result, Value::Long(31996000));
        for (x, live) in samples.lock().unwrap().iter() { println!("PROBE,{case},{x},{live}"); }
        gc_roots::force_collect(&env);
        gc_roots::force_collect(&env);
        println!("PROBE,{case},after,{}", HEAP.count());
    }).unwrap().join().unwrap();
}
