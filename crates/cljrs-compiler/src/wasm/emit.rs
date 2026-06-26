//! WebAssembly emitter (scaffold).
//!
//! Walks a [`reloop::Structured`] tree and lowers each IR [`Inst`] to wasm,
//! producing a module via `wasm-encoder` (the intended encoder dependency,
//! added when this is implemented).  Until then [`emit_function`] returns
//! [`WasmError::Unimplemented`] and this module documents the lowering plan and
//! exposes the per-`Inst` dispatch skeleton so the contract is reviewable.
//!
//! # Lowering plan
//!
//! ## Function signature
//!
//! Visible params map to wasm params by [`abi::WasmValType::for_repr`] of each
//! param's inferred [`Repr`].  A region-parameterised variant
//! ([`IrFunction::takes_region_param`]) appends one trailing `i32` region
//! handle; a poll function ([`IrFunction::takes_state_param`]) uses the fixed
//! `(state_ptr: i32, out_ptr: i32) -> i32` shape instead.  This mirrors
//! [`IrFunction::abi_param_count`] on the native side.
//!
//! ## Values → locals
//!
//! Each IR [`VarId`] becomes a wasm local of the type given by its inferred
//! [`Repr`] (`crate::typeinfer`).  SSA `phi` nodes are resolved on the way in:
//! each predecessor writes the phi's local before branching, so no φ instruction
//! is emitted — the structured arms simply `local.set` the join local.
//!
//! ## Instructions → wasm
//!
//! | IR construct | wasm lowering |
//! |---|---|
//! | [`Inst::Const`] scalar | `i64.const` / `f64.const` (unboxed) or `call rt_const_*` (boxed) |
//! | [`Inst::CallKnown`] arith on `Long`/`Double` | native `i64.add`/`f64.add`/… per `typeinfer` |
//! | [`Inst::CallKnown`] boxed | `call rt_<op>` bridge (see [`abi::RT_IMPORTS`]) |
//! | [`Inst::Alloc*`] | spill operands to a scratch array in linear memory, `call rt_alloc_*` |
//! | [`Inst::RegionStart`] | `call rt_region_start`, keep handle in an `i32` local |
//! | [`Inst::RegionAlloc`] | `call rt_region_alloc_*` with the handle leading |
//! | [`Inst::RegionEnd`] | `call rt_region_end` |
//! | [`Inst::RegionParam`] | bind the trailing `i32` param local |
//! | [`Inst::CallWithRegion`] | `call`/`return_call` passing the region handle trailing |
//! | [`Inst::CallDirect`] | `call $fn` (or `return_call` in tail position when [`WasmBackend::tail_calls`]) |
//! | [`Inst::Call`] | per-call-site inline cache: `call rt_call_ic` |
//! | [`Terminator::Branch`] | structured `if` from the relooper |
//! | [`Terminator::RecurJump`] | `br` to the enclosing `loop` label |
//! | [`Terminator::Return`] | `return` |
//! | [`Inst::Throw`] / `TryCatchFinally` | `throw`/`try`/`catch` when [`WasmBackend::exceptions`], else an `rt_abi` error path |
//!
//! ## GC + safepoints
//!
//! `rt_safepoint` is called at function entry and at every loop back-edge
//! (`Structured::Continue`), matching the native backend.  The GC heap lives in
//! the module's linear memory; emitted code holds no raw GC pointers across
//! suspension because constants are materialized through `rt_abi`.

use crate::ir::{IrFunction, Repr};

use super::abi::WasmValType;
use super::reloop::Structured;
use super::{WasmBackend, WasmError};

/// Emit a wasm function body for `func` given its structured control flow.
///
/// Scaffold: validates the structured tree can be walked and computes the wasm
/// signature, then returns [`WasmError::Unimplemented`] for the encoder step.
pub fn emit_function(
    func: &IrFunction,
    structured: &Structured,
    cfg: &WasmBackend,
) -> Result<Vec<u8>, WasmError> {
    // Compute the wasm signature now so the ABI is exercised even while the
    // encoder is stubbed (keeps the region/poll param accounting honest).
    let sig = function_signature(func);
    let _ = (structured, cfg, sig);

    // Walk the tree once to surface unsupported nodes early; a real emitter
    // would thread a `wasm_encoder::Function` here instead of `&mut ()`.
    walk(structured)?;

    Err(WasmError::Unimplemented(
        "wasm-encoder emission (front half — reloop + signature + tree walk — is wired)",
    ))
}

/// The wasm function signature for `func`: `(params, results)`.
///
/// Honors the hidden trailing region param and the poll-function ABI, mirroring
/// [`IrFunction::abi_param_count`].
pub fn function_signature(func: &IrFunction) -> (Vec<WasmValType>, Vec<WasmValType>) {
    if func.takes_state_param() {
        // Poll ABI: (state_ptr: i32, out_ptr: i32) -> i32 (Poll discriminant).
        return (
            vec![WasmValType::I32, WasmValType::I32],
            vec![WasmValType::I32],
        );
    }

    let mut params: Vec<WasmValType> = func
        .params
        .iter()
        .enumerate()
        .map(|(i, _)| {
            let repr = func.seed_reprs.get(i).copied().unwrap_or(Repr::Boxed);
            WasmValType::for_repr(repr)
        })
        .collect();

    if func.takes_region_param() {
        // Hidden trailing region handle.
        params.push(WasmValType::I32);
    }

    // Every Clojure function yields a single value (boxed unless a return-repr
    // pass proves otherwise); the scaffold returns the universal boxed i32.
    (params, vec![WasmValType::I32])
}

/// Walk the structured tree, returning an error for nodes the emitter scaffold
/// does not yet handle.  This is where the `wasm-encoder` `Function` builder
/// will be threaded; for now it only validates structure.
fn walk(node: &Structured) -> Result<(), WasmError> {
    match node {
        Structured::Simple { next, .. } => walk(next),
        Structured::If {
            then_arm,
            else_arm,
            next,
            ..
        } => {
            walk(then_arm)?;
            walk(else_arm)?;
            walk(next)
        }
        Structured::Loop { body, next, .. } => {
            walk(body)?;
            walk(next)
        }
        Structured::Break(_)
        | Structured::Continue(_)
        | Structured::Return(_)
        | Structured::Unreachable
        | Structured::Nil => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Block, Const, Inst, Terminator, VarId};
    use std::sync::Arc;

    fn region_variant() -> IrFunction {
        // A function whose entry binds a RegionParam → takes a trailing i32.
        let mut f = IrFunction::new(Some(Arc::from("rv")), None);
        let p = f.fresh_var();
        f.params = vec![(Arc::from("x"), p)];
        let rp = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::RegionParam(rp)],
            terminator: Terminator::Return(p),
        });
        f
    }

    #[test]
    fn region_variant_gets_trailing_i32_param() {
        let f = region_variant();
        assert!(f.takes_region_param());
        let (params, results) = function_signature(&f);
        // visible x (boxed i32) + hidden region handle (i32)
        assert_eq!(params, vec![WasmValType::I32, WasmValType::I32]);
        assert_eq!(results, vec![WasmValType::I32]);
    }

    #[test]
    fn long_hinted_param_is_i64() {
        let mut f = IrFunction::new(Some(Arc::from("g")), None);
        let p = f.fresh_var();
        f.params = vec![(Arc::from("n"), p)];
        f.seed_reprs = vec![Repr::Long];
        let v = f.fresh_var();
        let b = f.fresh_block();
        f.blocks.push(Block {
            id: b,
            phis: vec![],
            insts: vec![Inst::Const(v, Const::Long(0))],
            terminator: Terminator::Return(VarId(0)),
        });
        let (params, _) = function_signature(&f);
        assert_eq!(params, vec![WasmValType::I64]);
    }

    #[test]
    fn emit_is_stubbed_but_front_half_runs() {
        let f = region_variant();
        let structured = super::super::reloop::reloop(&f).expect("reloop");
        let err = emit_function(&f, &structured, &WasmBackend::default()).unwrap_err();
        assert!(matches!(err, WasmError::Unimplemented(_)));
    }
}
