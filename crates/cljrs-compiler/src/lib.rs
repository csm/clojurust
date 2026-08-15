#![allow(clippy::result_large_err)]
#![allow(clippy::arc_with_non_send_sync)]

//! Compiler backend for clojurust: one package for every code-generating
//! path.
//!
//! This crate provides:
//! - **Escape analysis** — track value flow and identify non-escaping allocations
//! - **Type inference** — scalar representations shared by every backend
//! - **Code generation** — Cranelift-based native code, generic over the
//!   `cranelift_module::Module` backing it
//! - **JIT** (`jit`) — in-process compilation of hot arities into a
//!   `JITModule`, with an epoch-tagged code cache and reclamation
//! - **Native AOT** (`aot`) — whole-program compilation to an object file plus
//!   a generated Rust harness
//! - **WASM AOT** (`wasm`) — the browser backend, behind the default-on
//!   `wasm-aot` feature
//!
//! The JIT and AOT paths are not separate products: they share `typeinfer`,
//! `codegen`, and the `rt_abi` runtime ABI directly, so a change to the
//! calling convention or to representation inference lands in both at once.
//!
//! The IR itself lives in the `cljrs-ir` crate, which both this crate and
//! `cljrs-runtime` depend on. ANF lowering and escape analysis run in pure
//! Rust (`cljrs_ir::lower`); `codegen.rs` consumes the resulting `IrFunction`
//! structs directly.

pub mod aot;
pub mod codegen;
pub mod escape;
pub mod extensions;
pub mod jit;
pub mod rt_abi;
pub mod typeinfer;
#[cfg(feature = "wasm-aot")]
pub mod wasm;
