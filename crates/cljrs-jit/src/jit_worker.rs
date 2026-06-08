//! Background JIT compilation worker thread.
//!
//! Receives compilation requests on a channel, compiles each `IrFunction` via
//! Cranelift, and atomically publishes the result via
//! `cljrs_eval::jit_state::store_native_fn`.
//!
//! The compiled `JITModule`s are kept in a local `Vec` on the worker thread
//! for the lifetime of the process.  Phase 10.2 will add epoch-tagged
//! unloading at STW safepoints.

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use cljrs_ir::IrFunction;

use crate::jit_compiler::{CompiledFn, compile_jit};

pub(crate) struct CompileRequest {
    pub(crate) arity_id: u64,
    pub(crate) ir_func: Arc<IrFunction>,
}

/// Spawn the background JIT worker thread.
pub(crate) fn start_worker(rx: Receiver<CompileRequest>) {
    std::thread::Builder::new()
        .name("cljrs-jit-worker".into())
        .spawn(move || worker_loop(rx))
        .expect("failed to spawn JIT worker thread");
}

fn worker_loop(rx: Receiver<CompileRequest>) {
    // Keep JITModules alive here so their executable memory is never freed.
    // Function pointers stored in jit_state remain valid for the process lifetime.
    // Phase 10.2 implements proper reclamation.
    let mut live: Vec<CompiledFn> = Vec::new();

    for req in &rx {
        cljrs_logging::feat_debug!("jit", "compiling arity_id={}", req.arity_id);

        match compile_jit(req.arity_id, &req.ir_func) {
            Ok(compiled) => {
                cljrs_logging::feat_debug!(
                    "jit",
                    "compiled  arity_id={} fn_ptr={:p}",
                    req.arity_id,
                    compiled.fn_ptr,
                );
                // Atomically publish the function pointer so future calls
                // on the main thread skip the interpreter.
                cljrs_eval::jit_state::store_native_fn(req.arity_id, compiled.fn_ptr);
                live.push(compiled);
            }
            Err(e) => {
                cljrs_logging::feat_debug!("jit", "compile error arity_id={}: {}", req.arity_id, e,);
                // Don't retry; the function stays at Tier 1.
            }
        }
    }
    // Channel closed (program exit) → drop JITModules.
    drop(live);
}
