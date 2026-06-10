//! Background JIT compilation worker thread.
//!
//! Receives compilation requests on a channel, compiles each `IrFunction` via
//! Cranelift, hands the module to the epoch-tagged [`code_cache`](crate::code_cache),
//! and atomically publishes the resulting function pointer + epoch via
//! `cljrs_eval::jit_state`.
//!
//! Two request kinds arrive on the same channel:
//! - whole-function compiles (invocation counter crossed the threshold), and
//! - OSR-entry compiles (a loop back-edge counter crossed the threshold —
//!   Phase 10.4).  The worker builds the OSR-entry variant with
//!   [`cljrs_ir::osr::build_osr_function`], compiles it, and publishes
//!   `(fn_ptr, epoch, live-ins)` via `jit_state::store_osr_fn`; failures are
//!   recorded via `jit_state::mark_osr_failed` so interpreters stop polling.
//!
//! Ownership of every compiled `JITModule` lives in the `code_cache`, which
//! reclaims superseded modules at stop-the-world safepoints (Phase 10.2).

use std::sync::Arc;
use std::sync::mpsc::Receiver;

use cljrs_compiler::typeinfer::Repr;
use cljrs_ir::{BlockId, IrFunction};

use crate::code_cache;
use crate::jit_compiler::compile_jit;

pub(crate) enum CompileRequest {
    /// Whole-function compile for a hot arity.
    Function {
        arity_id: u64,
        ir_func: Arc<IrFunction>,
    },
    /// OSR-entry compile for a hot loop header inside `arity_id`.
    Osr {
        arity_id: u64,
        header: u32,
        ir_func: Arc<IrFunction>,
    },
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
        match req {
            CompileRequest::Function { arity_id, ir_func } => {
                compile_function_request(arity_id, &ir_func)
            }
            CompileRequest::Osr {
                arity_id,
                header,
                ir_func,
            } => compile_osr_request(arity_id, header, &ir_func),
        }
    }
}

/// Map the Tier-1 argument-type profile onto per-parameter specializations
/// (Phase 10.6).  A parameter specializes only when every profiled call saw
/// exactly one scalar class (`PROFILE_LONG` or `PROFILE_DOUBLE`); anything
/// mixed or non-scalar stays boxed.  Returns `None` when nothing would be
/// specialized — the compile is then fully generic and needs no guards.
fn specs_from_profile(arity_id: u64, ir_func: &IrFunction) -> Option<Vec<Repr>> {
    if std::env::var("CLJRS_JIT_NO_SPEC").is_ok() {
        return None;
    }
    if !cljrs_eval::jit_state::specialization_allowed(arity_id) {
        return None;
    }
    let profile = cljrs_eval::jit_state::arg_type_profile(arity_id)?;
    if profile.len() != ir_func.params.len() {
        return None;
    }
    let specs: Vec<Repr> = profile
        .iter()
        .map(|&bits| match bits {
            cljrs_eval::jit_state::PROFILE_LONG => Repr::Long,
            cljrs_eval::jit_state::PROFILE_DOUBLE => Repr::Double,
            _ => Repr::Boxed,
        })
        .collect();
    if specs.iter().all(|s| *s == Repr::Boxed) {
        None
    } else {
        Some(specs)
    }
}

fn compile_function_request(arity_id: u64, ir_func: &IrFunction) {
    let specs = specs_from_profile(arity_id, ir_func).unwrap_or_default();
    cljrs_logging::feat_debug!("jit", "compiling arity_id={} specs={:?}", arity_id, specs);

    // Isolate each compilation: a panic in codegen (e.g. an unsupported IR
    // shape that trips a Cranelift assertion) must not kill the worker
    // thread and silently disable the JIT for the rest of the session.  On
    // panic the function simply stays at Tier 1, exactly like a clean
    // compile error.
    let compiled = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_jit(&format!("__cljrs_jit_{arity_id}"), ir_func, &specs)
    })) {
        Ok(result) => result,
        Err(_) => {
            cljrs_logging::feat_debug!(
                "jit",
                "compile panicked arity_id={}; staying at Tier 1",
                arity_id
            );
            return;
        }
    };

    match compiled {
        Ok(compiled) => {
            let fn_ptr = compiled.fn_ptr;
            // Hand ownership of the module to the code cache; it returns the
            // epoch that identifies this code for later reclamation.
            let epoch = code_cache::register(arity_id, compiled);
            cljrs_logging::feat_debug!(
                "jit",
                "compiled  arity_id={} epoch={} fn_ptr={:p}",
                arity_id,
                epoch,
                fn_ptr,
            );
            // Atomically publish the function pointer + epoch so future
            // calls on mutator threads skip the interpreter.
            cljrs_eval::jit_state::store_native_fn(arity_id, fn_ptr, epoch);
        }
        Err(e) => {
            cljrs_logging::feat_debug!("jit", "compile error arity_id={}: {}", arity_id, e,);
            // Don't retry; the function stays at Tier 1.
        }
    }
}

fn compile_osr_request(arity_id: u64, header: u32, ir_func: &IrFunction) {
    cljrs_logging::feat_debug!(
        "jit",
        "osr compiling arity_id={} header=bb{}",
        arity_id,
        header
    );

    // Same panic isolation as whole-function compiles: on any failure the
    // loop simply keeps running at Tier 1.
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let osr = cljrs_ir::osr::build_osr_function(ir_func, BlockId(header))?;
        let name = format!("__cljrs_jit_{arity_id}_osr{header}");
        // OSR variants are never specialized: their parameters are the loop
        // live-ins transferred from the interpreter register file, for which
        // no type profile exists.
        let compiled = compile_jit(&name, &osr.func, &[])?;
        Ok::<_, String>((compiled, osr.live_ins))
    }));

    match outcome {
        Ok(Ok((compiled, live_ins))) => {
            let fn_ptr = compiled.fn_ptr;
            let epoch = code_cache::register(arity_id, compiled);
            cljrs_logging::feat_debug!(
                "jit",
                "osr compiled arity_id={} header=bb{} epoch={} fn_ptr={:p} live_ins={}",
                arity_id,
                header,
                epoch,
                fn_ptr,
                live_ins.len(),
            );
            cljrs_eval::jit_state::store_osr_fn(arity_id, header, fn_ptr, epoch, live_ins);
        }
        Ok(Err(e)) => {
            cljrs_logging::feat_debug!(
                "jit",
                "osr declined arity_id={} header=bb{}: {}",
                arity_id,
                header,
                e,
            );
            cljrs_eval::jit_state::mark_osr_failed(arity_id, header);
        }
        Err(_) => {
            cljrs_logging::feat_debug!(
                "jit",
                "osr compile panicked arity_id={} header=bb{}; staying at Tier 1",
                arity_id,
                header
            );
            cljrs_eval::jit_state::mark_osr_failed(arity_id, header);
        }
    }
}
