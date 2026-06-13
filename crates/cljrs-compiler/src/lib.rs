#![allow(clippy::result_large_err)]
#![allow(clippy::arc_with_non_send_sync)]

//! Program analysis and optimization for clojurust.
//!
//! This crate provides:
//! - **IR** — intermediate representation (ANF/SSA) for analysis
//! - **Escape analysis** — track value flow and identify non-escaping allocations
//! - **Code generation** — Cranelift-based native code for JIT and AOT
//!
//! ANF lowering and escape analysis run in pure Rust (`cljrs_ir::lower`);
//! `codegen.rs` consumes the resulting `IrFunction` structs directly.

pub mod aot;
pub mod codegen;
pub mod escape;
pub mod ir;
pub mod rt_abi;
pub mod typeinfer;
