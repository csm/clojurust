//! Background JIT compilation worker thread.
//!
//! Receives compilation requests on a channel, compiles each `IrFunction` via
//! Cranelift, hands the module to the epoch-tagged [`code_cache`](crate::code_cache),
//! and atomically publishes the resulting function pointer + epoch via
//! `cljrs_eval::jit_state::store_native_fn`.
//!
//! Ownership of every compiled `JITModule` lives in the `code_cache`, which
//! reclaims superseded modules at stop-the-world safepoints (Phase 10.2).

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use cljrs_ir::IrFunction;

use crate::code_cache;
use crate::jit_compiler::compile_jit;

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
    for req in &rx {
        cljrs_logging::feat_debug!("jit", "compiling arity_id={}", req.arity_id);

        match compile_jit(req.arity_id, &req.ir_func) {
            Ok(compiled) => {
                let fn_ptr = compiled.fn_ptr;
                // Hand ownership of the module to the code cache; it returns the
                // epoch that identifies this code for later reclamation.
                let epoch = code_cache::register(req.arity_id, compiled);
                cljrs_logging::feat_debug!(
                    "jit",
                    "compiled  arity_id={} epoch={} fn_ptr={:p}",
                    req.arity_id,
                    epoch,
                    fn_ptr,
                );
                // Atomically publish the function pointer + epoch so future
                // calls on mutator threads skip the interpreter.
                cljrs_eval::jit_state::store_native_fn(req.arity_id, fn_ptr, epoch);
            }
            Err(e) => {
                cljrs_logging::feat_debug!("jit", "compile error arity_id={}: {}", req.arity_id, e,);
                // Don't retry; the function stays at Tier 1.
            }
        }
    }
}
