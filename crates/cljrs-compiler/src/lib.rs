#![allow(clippy::result_large_err)]

//! Program analysis and optimization for clojurust.
//!
//! This crate provides:
//! - **IR** — intermediate representation (ANF/SSA) for analysis
//! - **ANF lowering** — convert `Form` AST to IR instructions (Rust + Clojure front-ends)
//! - **Escape analysis** — track value flow and identify non-escaping allocations
//! - **IR conversion** — Clojure Value data → Rust IR types
//!
//! The Clojure front-end (`cljrs.compiler.anf`, `cljrs.compiler.escape`) produces
//! IR as plain Clojure data. The `ir_convert` module translates this back to the
//! Rust `IrFunction` structs that `codegen.rs` consumes.

pub mod aot;
pub mod codegen;
pub mod ir;
pub mod rt_abi;

/// Register all compiler Clojure source files as builtin sources in the
/// given `GlobalEnv`, so that `require` can load them without filesystem access.
pub fn register_compiler_sources(globals: &std::sync::Arc<cljrs_env::env::GlobalEnv>) {
    globals.register_builtin_source("cljrs.compiler.ir", cljrs_ir::COMPILER_IR_SOURCE);
    globals.register_builtin_source("cljrs.compiler.known", cljrs_ir::COMPILER_KNOWN_SOURCE);
    globals.register_builtin_source("cljrs.compiler.anf", cljrs_ir::COMPILER_ANF_SOURCE);
    globals.register_builtin_source("cljrs.compiler.escape", cljrs_ir::COMPILER_ESCAPE_SOURCE);
    globals.register_builtin_source("cljrs.compiler.optimize", cljrs_ir::COMPILER_OPTIMIZE_SOURCE);
}
