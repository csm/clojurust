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
//! - [`reloop`] — recover structured control flow from the IR's arbitrary CFG.
//!   wasm has only `block`/`loop`/`if` + labeled `br`, no `goto`.  Cranelift
//!   wants the raw CFG, so this pass is wasm-private and lives *here*, not in
//!   shared lowering.  Clojure source yields reducible CFGs, so only the cheap
//!   relooper is required (no node-splitting, no dispatch variable).
//! - [`emit`] — walk the structured tree + per-`Inst` lowering to a
//!   `wasm-encoder` module, with `rt_abi` symbols declared as wasm imports.
//!
//! # Status
//!
//! **Scaffold.**  The module structure, public API, the relooper data model
//! (with trivial cases implemented), and the full ABI/region contract are in
//! place.  The `wasm-encoder` emitter and the relooper's loop/join structuring
//! are stubbed and return [`WasmError::Unimplemented`].

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
