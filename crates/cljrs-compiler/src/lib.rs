#![allow(clippy::result_large_err)]

//! Program analysis and optimization for clojurust.
//!
//! This crate provides:
//! - **IR** — intermediate representation (ANF/SSA) for analysis
//! - **ANF lowering** — convert `Form` AST to IR instructions
//! - **Escape analysis** — track value flow and identify non-escaping allocations
//!
//! Currently used to generate optimization hints for the interpreter.
//! In Phase 10/11, this IR will be the input to Cranelift-based JIT/AOT
//! code generation.

pub mod anf;
pub mod aot;
pub mod codegen;
pub mod escape;
pub mod ir;
pub mod rt_abi;
