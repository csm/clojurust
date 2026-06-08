//! In-process JIT tier for clojurust (Phase 10.1).
//!
//! This crate is the fourth execution tier described in [`docs/jit-plan.md`].
//! It reuses the shared Cranelift codegen from `cljrs-compiler` (the same
//! lowering that drives AOT) but targets a [`JITModule`] instead of an object
//! file, so hot function arities are compiled to native code **in process**.
//!
//! ## How it plugs in
//!
//! The hot dispatch path lives in `cljrs-eval` ([`cljrs_eval::jit_state`]),
//! below this crate in the dependency graph.  It cannot call into `cljrs-jit`
//! directly, so [`init`] registers two function-pointer hooks:
//!
//! - **compile hook** — sends `(arity_id, IrFunction, n_params)` to a
//!   background worker thread.  The worker compiles via `JITModule`, then
//!   atomically publishes the finalized code pointer back into the shared
//!   `JitState`.  Until then, calls keep running the interpreter — no stall.
//! - **invoke hook** — calls a finalized `extern "C"` code pointer with a slice
//!   of boxed `*const Value` arguments, surfacing any thrown exception.
//!
//! ## Runtime bridge & constants
//!
//! Every `rt_*` symbol (`cljrs_compiler::rt_abi::rt_symbols`) is registered with
//! the `JITBuilder` so emitted calls resolve in-process.  Constants are
//! materialized through `rt_const_*` runtime calls (the AOT strategy), so the
//! emitted code embeds no `GcPtr`s — which keeps both conservative stack
//! scanning and (future) code unloading tractable.

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, OnceLock};

use cranelift_jit::{JITBuilder, JITModule};
use cranelift_module::{Module, default_libcall_names};

use cljrs_compiler::codegen::{Compiler, new_compiler_from_module};
use cljrs_compiler::rt_abi;
use cljrs_eval::jit_state;
use cljrs_ir::IrFunction;
use cljrs_value::Value;

/// Fixed symbol name for the single function compiled into each JIT module.
/// Each arity gets its own [`JITModule`], so there is never a name collision.
const JIT_ENTRY_NAME: &str = "__cljrs_jit_entry";

/// Maximum arity the Phase 10.1 native call ABI supports.  Higher arities fall
/// back to the interpreter (they are vanishingly rare in practice).
const MAX_JIT_ARITY: usize = 8;

// ── Background worker ────────────────────────────────────────────────────────

/// A unit of work for the compilation thread.
struct Job {
    arity_id: u64,
    ir: Arc<IrFunction>,
    n_params: u8,
}

static SENDER: OnceLock<Mutex<Sender<Job>>> = OnceLock::new();

/// Initialize the JIT backend: register the hooks and spawn the compile worker.
///
/// Idempotent — safe to call from every entry point (CLI `run`, `repl`,
/// `eval`).  Has no effect unless the JIT is also enabled
/// ([`cljrs_eval::jit_state::set_enabled`]); enabling without calling `init`
/// simply means arities are counted but never compiled.
pub fn init() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        // The invoke hook is a plain function — no thread required.
        jit_state::register_invoke_hook(invoke_hook);

        let (tx, rx) = mpsc::channel::<Job>();
        let _ = SENDER.set(Mutex::new(tx));

        std::thread::Builder::new()
            .name("cljrs-jit".to_string())
            .spawn(move || worker(rx))
            .expect("failed to spawn cljrs-jit worker thread");

        jit_state::register_compile_hook(compile_hook);
        cljrs_logging::feat_debug!("jit", "JIT backend initialized");
    });
}

/// Compile-hook entry point (runs on the mutator thread): enqueue a job.
fn compile_hook(arity_id: u64, ir: Arc<IrFunction>, n_params: u8) {
    if let Some(lock) = SENDER.get()
        && let Ok(tx) = lock.lock()
        && tx
            .send(Job {
                arity_id,
                ir,
                n_params,
            })
            .is_err()
    {
        // Worker is gone; never retry this arity.
        jit_state::mark_failed(arity_id);
    }
}

/// Background compilation loop.  Owns the live [`JITModule`]s so their
/// executable memory stays mapped for the rest of the process (code unloading
/// is Phase 10.2).
fn worker(rx: mpsc::Receiver<Job>) {
    // Keep every compiled module alive.
    let mut live: Vec<Compiler<JITModule>> = Vec::new();

    for job in rx {
        match compile_one(&job.ir, job.n_params) {
            Ok((compiler, code)) => {
                live.push(compiler);
                jit_state::publish_code(job.arity_id, code);
                cljrs_logging::feat_trace!("jit", "compiled arity {} -> {:p}", job.arity_id, code);
            }
            Err(e) => {
                cljrs_logging::feat_debug!(
                    "jit",
                    "JIT compilation failed for arity {}: {}",
                    job.arity_id,
                    e
                );
                jit_state::mark_failed(job.arity_id);
            }
        }
    }
}

// ── Compilation ──────────────────────────────────────────────────────────────

/// Build a fresh `JITModule` with every `rt_*` symbol registered.
fn make_jit_module() -> Result<JITModule, String> {
    let mut builder = JITBuilder::new(default_libcall_names())
        .map_err(|e| format!("failed to create JITBuilder: {e}"))?;
    for (name, ptr) in rt_abi::rt_symbols() {
        builder.symbol(name, ptr);
    }
    Ok(JITModule::new(builder))
}

/// Compile a single IR function to native code, returning the owning compiler
/// (which must be kept alive) and the finalized entry pointer.
fn compile_one(ir: &IrFunction, n_params: u8) -> Result<(Compiler<JITModule>, *const u8), String> {
    let module = make_jit_module()?;
    let ptr_type = module.target_config().pointer_type();
    let mut compiler = new_compiler_from_module(module, ptr_type).map_err(|e| format!("{e:?}"))?;

    let func_id = compiler
        .declare_function(JIT_ENTRY_NAME, n_params as usize)
        .map_err(|e| format!("declare failed: {e:?}"))?;
    compiler
        .compile_function(ir, func_id)
        .map_err(|e| format!("codegen failed: {e:?}"))?;

    compiler
        .module_mut()
        .finalize_definitions()
        .map_err(|e| format!("finalize failed: {e}"))?;
    let code = compiler.module_mut().get_finalized_function(func_id);

    Ok((compiler, code))
}

// ── Native invocation ────────────────────────────────────────────────────────

/// Invoke-hook entry point: call finalized native code and surface its result.
///
/// `args` are stable `*const Value` pointers (rooted by the caller).  The
/// `rt_*` bridge clones values out of these, so they need only stay valid for
/// the call.  A `(throw ...)` inside the body unwinds to the function's
/// `Unreachable` terminator (returning nil) and stashes the value in
/// `rt_abi`'s thread-local; we surface it here as `Err`.
fn invoke_hook(code: *const u8, args: &[*const Value]) -> Result<Value, Value> {
    let ret = unsafe { call_native(code, args) };

    if let Some(thrown) = rt_abi::take_pending_exception_value() {
        return Err(thrown);
    }

    if ret.is_null() {
        return Ok(Value::Nil);
    }
    Ok(unsafe { (*ret).clone() })
}

/// Transmute `code` to the correct `extern "C"` arity and call it.
///
/// # Safety
/// `code` must point to finalized native code compiled with
/// `declare_function(_, args.len())`, i.e. an
/// `extern "C" fn(*const Value, ...) -> *const Value` taking exactly
/// `args.len()` pointer parameters, and `args.len()` must be `<= MAX_JIT_ARITY`.
unsafe fn call_native(code: *const u8, args: &[*const Value]) -> *const Value {
    use std::mem::transmute;
    type P = *const Value;
    unsafe {
        match args.len() {
            0 => (transmute::<*const u8, extern "C" fn() -> P>(code))(),
            1 => (transmute::<*const u8, extern "C" fn(P) -> P>(code))(args[0]),
            2 => (transmute::<*const u8, extern "C" fn(P, P) -> P>(code))(args[0], args[1]),
            3 => (transmute::<*const u8, extern "C" fn(P, P, P) -> P>(code))(
                args[0], args[1], args[2],
            ),
            4 => (transmute::<*const u8, extern "C" fn(P, P, P, P) -> P>(code))(
                args[0], args[1], args[2], args[3],
            ),
            5 => (transmute::<*const u8, extern "C" fn(P, P, P, P, P) -> P>(code))(
                args[0], args[1], args[2], args[3], args[4],
            ),
            6 => (transmute::<*const u8, extern "C" fn(P, P, P, P, P, P) -> P>(code))(
                args[0], args[1], args[2], args[3], args[4], args[5],
            ),
            7 => (transmute::<*const u8, extern "C" fn(P, P, P, P, P, P, P) -> P>(code))(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6],
            ),
            8 => (transmute::<*const u8, extern "C" fn(P, P, P, P, P, P, P, P) -> P>(code))(
                args[0], args[1], args[2], args[3], args[4], args[5], args[6], args[7],
            ),
            // Excluded at queue time, but stay defensive.
            _ => std::ptr::null(),
        }
    }
}

/// Largest arity the JIT will compile (used by the eligibility check above the
/// seam, exposed for tests / callers that want to mirror the bound).
pub const fn max_arity() -> usize {
    MAX_JIT_ARITY
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use cljrs_gc::GcPtr;
    use cljrs_reader::Parser;

    fn parse_body(src: &str) -> Vec<cljrs_reader::Form> {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let mut forms = Vec::new();
        while let Ok(Some(form)) = parser.parse_one() {
            forms.push(form);
        }
        forms
    }

    fn lower(name: &str, params: &[Arc<str>], src: &str) -> IrFunction {
        let name = name.to_string();
        let params = params.to_vec();
        let body = parse_body(src);
        std::thread::Builder::new()
            .stack_size(8 * 1024 * 1024)
            .spawn(move || {
                let globals = cljrs_stdlib::standard_env();
                let mut env = cljrs_eval::Env::new(globals, "user");
                cljrs_compiler::aot::lower_via_rust(Some(&name), "user", &params, &body, &mut env)
                    .unwrap()
            })
            .unwrap()
            .join()
            .unwrap()
    }

    fn boxed(v: Value) -> *const Value {
        // Leak a GC-managed Value to obtain a stable pointer for the call.
        GcPtr::new(v).get() as *const Value
    }

    /// Compile `(+ a b)` to native code and invoke it through the same call
    /// path the dispatch seam uses — verifies the end-to-end JIT round-trip
    /// (codegen → JITModule finalize → native call → result) deterministically,
    /// without the background worker.
    #[test]
    fn jit_compiles_and_runs_add() {
        let params: Vec<Arc<str>> = vec![Arc::from("a"), Arc::from("b")];
        let ir = lower("add", &params, "(+ a b)");
        assert!(jit_eligible(&ir), "flat arithmetic fn must be JIT-eligible");

        let (_keep, code) = compile_one(&ir, 2).expect("compilation should succeed");

        let args = [boxed(Value::Long(40)), boxed(Value::Long(2))];
        match invoke_hook(code, &args) {
            Ok(Value::Long(n)) => assert_eq!(n, 42),
            other => panic!("expected Long(42), got {other:?}"),
        }
    }

    /// A flat conditional compiles and both branches execute natively.
    #[test]
    fn jit_runs_conditional() {
        let params: Vec<Arc<str>> = vec![Arc::from("x")];
        let ir = lower("pick", &params, "(if (< x 0) 100 200)");
        let (_keep, code) = compile_one(&ir, 1).expect("compilation should succeed");

        let neg = invoke_hook(code, &[boxed(Value::Long(-5))]).unwrap();
        let pos = invoke_hook(code, &[boxed(Value::Long(5))]).unwrap();
        assert_eq!(neg, Value::Long(100));
        assert_eq!(pos, Value::Long(200));
    }

    fn jit_eligible(ir: &IrFunction) -> bool {
        ir.subfunctions.is_empty() && !ir.is_async
    }
}
