//! Repeated evaluation of a function literal shares code, not closure state.
use cljrs_reader::Parser;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;
use std::sync::Arc;

fn eval(src: &str, env: &mut Env) -> Value {
    let forms = Parser::new(src.into(), "<closure-body-sharing>".into())
        .parse_all()
        .unwrap();
    let mut result = Value::Nil;
    for form in &forms {
        result = cljrs_runtime::tiered::eval(form, env).unwrap();
    }
    result
}

#[test]
fn closure_instances_share_syntax_but_keep_fresh_captures_and_ids() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _mutator = cljrs_gc::register_mutator();
            let globals = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                .build()
                .unwrap()
                .into_globals();
            let mut env = Env::new(globals, "user");
            eval(
                "(defn make-fn [n] (fn ([] n) ([x] {:pre [(pos? x)] :post [(> % n)]} (+ n x))))",
                &mut env,
            );
            let a = eval("(def a (make-fn 3)) a", &mut env);
            let b = eval("(def b (make-fn 7)) b", &mut env);
            let (Value::Fn(a), Value::Fn(b)) = (a, b) else {
                panic!("closures")
            };
            assert!(!cljrs_gc::GcPtr::ptr_eq(&a, &b));
            for (a, b) in a.get().arities.iter().zip(&b.get().arities) {
                assert!(Arc::ptr_eq(&a.body, &b.body), "body code is shared");
                assert_ne!(
                    a.ir_arity_id, b.ir_arity_id,
                    "arity identities remain fresh"
                );
            }
            assert_eq!(
                eval("[(a) (b) (a 2) (b 2)]", &mut env).to_string(),
                "[3 7 5 9]"
            );
            assert_eq!(
                eval("(try (a 0) (catch :default e :failed))", &mut env).to_string(),
                ":failed"
            );
            // A freshly read, different function must never inherit this site's code.
            assert_eq!(eval("((fn [x] (* x 10)) 4)", &mut env), Value::Long(40));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn sharing_body_does_not_cache_parameter_namespace_resolution() {
    std::thread::Builder::new()
        .stack_size(64 * 1024 * 1024)
        .spawn(|| {
            let _mutator = cljrs_gc::register_mutator();
            let globals = cljrs_runtime::Runtime::builder()
                .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
                .build()
                .unwrap()
                .into_globals();
            let form = Parser::new("(fn [{:keys [::x]}] x)".into(), "<ns-sharing>".into())
                .parse_all()
                .unwrap()
                .remove(0);
            let mut alpha = Env::new(globals.clone(), "alpha");
            let mut beta = Env::new(globals, "beta");
            let a = cljrs_runtime::tiered::eval(&form, &mut alpha).unwrap();
            let _a_root = cljrs_runtime::env::gc_roots::root_value(&a);
            let b = cljrs_runtime::tiered::eval(&form, &mut beta).unwrap();
            let _b_root = cljrs_runtime::env::gc_roots::root_value(&b);
            let map = cljrs_value::MapValue::empty()
                .assoc(
                    Value::keyword(cljrs_value::Keyword::qualified("alpha", "x")),
                    Value::Long(3),
                )
                .assoc(
                    Value::keyword(cljrs_value::Keyword::qualified("beta", "x")),
                    Value::Long(7),
                );
            let args = vec![Value::Map(map)];
            let _args_root = cljrs_runtime::env::gc_roots::root_values(&args);
            assert_eq!(
                cljrs_runtime::env::apply::apply_value(&a, args.clone(), &mut alpha).unwrap(),
                Value::Long(3)
            );
            assert_eq!(
                cljrs_runtime::env::apply::apply_value(&b, args.clone(), &mut beta).unwrap(),
                Value::Long(7)
            );
        })
        .unwrap()
        .join()
        .unwrap();
}
