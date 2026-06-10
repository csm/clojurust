//! Phase 10.4 — end-to-end OSR: lower a real `loop*`/`recur` from source,
//! build the OSR-entry variant, compile it with Cranelift, and *call* the
//! native code with a mid-loop interpreter state, mimicking exactly what
//! `interpret_ir_with_osr`'s transfer does.
//!
//! Gated to the default GC build like the reclaim integration test: under
//! `no-gc`, `lower_via_rust` runs the escape-blacklist check which can reject
//! otherwise-fine functions.

use std::sync::Arc;

use cljrs_ir::{BlockId, IrFunction, Terminator};
use cljrs_value::Value;

/// Lower a function body to IR on a generously sized stack (Clojure eval is
/// deeply recursive), mirroring the code_cache test helper.
fn build_ir(name: &str, params: &[Arc<str>], body_src: &str) -> IrFunction {
    let name = name.to_string();
    let params = params.to_vec();
    let body_src = body_src.to_string();
    std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn(move || {
            let mut parser = cljrs_reader::Parser::new(body_src, "<osr-test>".to_string());
            let mut forms = Vec::new();
            while let Ok(Some(f)) = parser.parse_one() {
                forms.push(f);
            }
            let globals = cljrs_stdlib::standard_env();
            let mut env = cljrs_eval::Env::new(globals, "user");
            cljrs_compiler::aot::lower_via_rust(Some(&name), "user", &params, &forms, &mut env)
                .expect("lowering should succeed")
        })
        .unwrap()
        .join()
        .unwrap()
}

/// The loop header of the (single) `RecurJump` in the function.
fn find_loop_header(ir: &IrFunction) -> BlockId {
    ir.blocks
        .iter()
        .find_map(|b| match &b.terminator {
            Terminator::RecurJump { target, .. } => Some(*target),
            _ => None,
        })
        .expect("function should contain a RecurJump back-edge")
}

#[test]
fn osr_entry_compiles_and_resumes_natively_mid_loop() {
    let _mutator = cljrs_gc::register_mutator();

    // (fn sum-below [n] (loop [i 0 acc 0] (if (< i n) (recur (+ i 1) (+ acc i)) acc)))
    let ir = build_ir(
        "sum-below",
        &[Arc::from("n")],
        "(loop [i 0 acc 0] (if (< i n) (recur (+ i 1) (+ acc i)) acc))",
    );

    let header = find_loop_header(&ir);
    let osr = cljrs_ir::osr::build_osr_function(&ir, header).expect("transform");
    assert!(
        (1..=8).contains(&osr.live_ins.len()),
        "expected a dispatchable live-in count, got {}",
        osr.live_ins.len()
    );

    let compiled =
        crate::jit_compiler::compile_jit("__cljrs_jit_osr_test", &osr.func).expect("compile");
    let fn_ptr = compiled.fn_ptr;
    let epoch = crate::code_cache::register(0xC0DE_0540, compiled);

    // Mid-loop interpreter state for n=10, paused at i=5: acc = 0+1+2+3+4 = 10.
    // The transform orders live-ins as [header φ dsts (i, acc), then outer (n)].
    let call_args = vec![Value::Long(5), Value::Long(10), Value::Long(10)];

    // Same transfer protocol as ir_interp::try_osr_enter.
    let result = {
        let _jit_frame = cljrs_eval::jit_state::push_jit_frame(epoch);
        let _arg_roots = cljrs_env::gc_roots::root_values(&call_args);
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        let arg_ptrs: Vec<*const Value> = call_args.iter().map(|v| v as *const Value).collect();
        // SAFETY: the OSR entry was compiled with `live_ins.len()` `*const
        // Value` params; all arg pointers are rooted and live for the call.
        let result_ptr = unsafe { cljrs_eval::jit_state::dispatch_jit_call(fn_ptr, &arg_ptrs) };
        unsafe { (*result_ptr).clone() }
    };

    // Remaining iterations add 5+6+7+8+9 → 45 = sum 0..10.
    assert_eq!(result, Value::Long(45));
}
