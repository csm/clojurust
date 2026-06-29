//! AOT Clojure → WebAssembly backend (scaffold).
//!
//! This module is the **second backend** for the shared `cljrs-ir` IR, parallel
//! to the Cranelift backend in [`crate::codegen`].  Where Cranelift lowers an
//! [`IrFunction`] CFG to native object code, this backend lowers the *same*
//! regionalized IR to a WebAssembly module that the browser JITs to native.
//!
//! # Why a separate backend at all
//!
//! A wasm module cannot generate and execute native machine code at runtime —
//! there is no `mmap(PROT_EXEC)` inside the sandbox — so the Cranelift JIT
//! (`cljrs-jit`) cannot run in a browser.  The browser deployment story is
//! therefore *ahead-of-time*: compile each Clojure function to wasm bytecode at
//! build time and ship it.  At runtime the tiers invert relative to native:
//!
//! ```text
//!   tree-walk  →  IR-interp        (dynamic: eval, REPL, freshly-required ns, macros)
//!   AOT-wasm                       (baked at build time; browser JITs it to native)
//! ```
//!
//! So the IR interpreter (`cljrs-eval::ir_interp`) remains *on board* in the
//! wasm bundle as the dynamic-code tier; AOT-wasm is the frozen top tier.  No
//! in-sandbox JIT/OSR hooks are installed.
//!
//! # What is reused unchanged
//!
//! Everything upstream of code generation is backend-agnostic and shared with
//! the Cranelift path:
//!
//! - ANF/SSA lowering (`cljrs_ir::lower`)
//! - Escape analysis + region inference (`cljrs_ir::lower::{escape, regionalize}`)
//! - Scalar representation inference (`crate::typeinfer`)
//! - The `rt_abi` runtime bridge contract (`crate::rt_abi`) — see [`abi`]
//!
//! Because escape analysis and region promotion are *properties of the IR*
//! (`Inst::Region*`, [`IrFunction::takes_region_param`]), bump allocation comes
//! along for free: a region is a linear-memory arena, a region handle is an
//! `i32` offset, and a region-parameterised variant simply takes that `i32` as
//! a hidden trailing parameter.  See [`abi`] for the region ABI contract.
//!
//! # What is new here (the only wasm-specific work)
//!
//! - [`reloop`] — recover structured control flow from the IR's arbitrary CFG
//!   via dominator-tree structuring (Ramsey's "Beyond Relooper").  wasm has only
//!   `block`/`loop`/`if` + labeled `br`, no `goto`.  Cranelift wants the raw
//!   CFG, so this pass is wasm-private and lives *here*, not in shared lowering.
//!   Clojure source yields reducible CFGs, so only the cheap relooper is
//!   required (no node-splitting, no dispatch variable).
//! - [`emit`] — walk the structured tree + per-`Inst` lowering to a
//!   `wasm-encoder` module, with `rt_abi` symbols declared as wasm imports.
//!
//! # Status
//!
//! The module structure, public API, the full ABI/region contract, and the
//! [`reloop`] relooper (complete for reducible CFGs — straight-line, `if`/`cond`
//! diamonds, sequential/nested merges, and `loop`/`recur` loops) are in place.
//! The [`emit`] emitter produces real, `wasmparser`-validated modules — both
//! single-function ([`compile_function`]) and multi-function
//! ([`compile_bundle`]) — for a growing subset of the IR: scalar constants,
//! `LoadLocal`, boxed arithmetic/comparison, collection + region allocation,
//! calls (`CallDirect`/`CallWithRegion`/`Call`), and all control flow.  Closures,
//! globals, string/keyword/symbol constants, and async still return
//! [`WasmError::Unsupported`].  See [`emit`]'s module docs for the full status.

pub mod abi;
pub mod emit;
pub mod reloop;

use crate::ir::IrFunction;

/// Errors produced by the wasm backend.
#[derive(Debug)]
pub enum WasmError {
    /// The relooper could not structure this function's control flow yet.
    Reloop(reloop::RelooperError),
    /// An IR construct the emitter does not yet lower to wasm.
    Unsupported(String),
    /// A scaffolded path that is not implemented yet.
    Unimplemented(&'static str),
}

impl std::fmt::Display for WasmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WasmError::Reloop(e) => write!(f, "relooper: {e}"),
            WasmError::Unsupported(what) => write!(f, "unsupported in wasm backend: {what}"),
            WasmError::Unimplemented(what) => write!(f, "not yet implemented: {what}"),
        }
    }
}

impl std::error::Error for WasmError {}

impl From<reloop::RelooperError> for WasmError {
    fn from(e: reloop::RelooperError) -> Self {
        WasmError::Reloop(e)
    }
}

/// WebAssembly feature flags the backend may target.
///
/// Defaults reflect what is broadly shipping in browsers as of the current
/// roadmap; the emitter consults these to decide between, e.g., `return_call`
/// (tail-call proposal) and a trampoline, or the exception-handling proposal
/// for `try`/`catch`.
#[derive(Debug, Clone, Copy)]
pub struct WasmBackend {
    /// Use the wasm tail-call proposal (`return_call`/`return_call_indirect`)
    /// for cross-function tail positions.  When `false`, the emitter must
    /// trampoline.  (`recur` within a function is always a `loop`/`br` and does
    /// not depend on this.)
    pub tail_calls: bool,
    /// Use the wasm exception-handling proposal for `try`/`catch`/`throw`
    /// (`KnownFn::TryCatchFinally`, `Inst::Throw`).  When `false`, the emitter
    /// must thread an explicit error path through `rt_abi`.
    pub exceptions: bool,
}

impl Default for WasmBackend {
    fn default() -> Self {
        Self {
            tail_calls: true,
            exceptions: true,
        }
    }
}

/// Compile a single [`IrFunction`] to a standalone wasm module's worth of
/// bytecode for that function.
///
/// Pipeline: [`reloop::reloop`] to recover structured control flow, then
/// [`emit::emit_function`] to encode it.  The region ABI ([`abi`]) is threaded
/// automatically for region-parameterised variants
/// ([`IrFunction::takes_region_param`]).
///
/// Currently returns [`WasmError::Unimplemented`] from the emitter; the
/// relooping front half runs for the cases [`reloop::reloop`] supports.
pub fn compile_function(func: &IrFunction, cfg: &WasmBackend) -> Result<Vec<u8>, WasmError> {
    let structured = reloop::reloop(func)?;
    emit::emit_function(func, &structured, cfg)
}

/// Compile a bundle of [`IrFunction`]s — each top-level function plus all of its
/// nested [`IrFunction::subfunctions`], flattened — into a single wasm module.
///
/// Every function is exported under its name, and an [`crate::ir::Inst::CallDirect`] /
/// [`crate::ir::Inst::CallWithRegion`] resolves its callee to the bundled
/// function's wasm index.  This is the multi-function entry point behind item 1
/// of `docs/wasm-aot-plan.md`; [`compile_function`] is the single-function
/// special case.
///
/// Pipeline: [`reloop::reloop`] each function, then [`emit::emit_bundle`].
pub fn compile_bundle(funcs: &[&IrFunction], cfg: &WasmBackend) -> Result<Vec<u8>, WasmError> {
    // Flatten each function with its subfunctions (depth-first), mirroring how
    // the Cranelift AOT path declares subfunctions before compiling.
    let mut flat: Vec<&IrFunction> = Vec::new();
    for func in funcs {
        collect_funcs(func, &mut flat);
    }

    let structured: Vec<reloop::Structured> = flat
        .iter()
        .map(|f| reloop::reloop(f))
        .collect::<Result<_, _>>()?;

    let pairs: Vec<(&IrFunction, &reloop::Structured)> =
        flat.iter().copied().zip(structured.iter()).collect();
    emit::emit_bundle(&pairs, cfg)
}

/// Push `func` and all of its (transitive) subfunctions into `out`, depth-first.
fn collect_funcs<'a>(func: &'a IrFunction, out: &mut Vec<&'a IrFunction>) {
    out.push(func);
    for sub in &func.subfunctions {
        collect_funcs(sub, out);
    }
}
