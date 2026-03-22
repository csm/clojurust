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
pub mod ir_convert;
pub mod rt_abi;

// ── Embedded Clojure compiler sources ───────────────────────────────────────

/// Clojure source for the IR builder namespace.
pub const COMPILER_IR_SOURCE: &str = include_str!("clojure/compiler/ir.cljrs");

/// Clojure source for the known function resolution namespace.
pub const COMPILER_KNOWN_SOURCE: &str = include_str!("clojure/compiler/known.cljrs");

/// Clojure source for the ANF lowering namespace.
pub const COMPILER_ANF_SOURCE: &str = include_str!("clojure/compiler/anf.cljrs");

/// Clojure source for the escape analysis namespace.
pub const COMPILER_ESCAPE_SOURCE: &str = include_str!("clojure/compiler/escape.cljrs");

/// Clojure source for the optimization pass namespace.
pub const COMPILER_OPTIMIZE_SOURCE: &str = include_str!("clojure/compiler/optimize.cljrs");

/// Register all compiler Clojure source files as builtin sources in the
/// given `GlobalEnv`, so that `require` can load them without filesystem access.
pub fn register_compiler_sources(globals: &std::sync::Arc<cljrs_eval::env::GlobalEnv>) {
    globals.register_builtin_source("cljrs.compiler.ir", COMPILER_IR_SOURCE);
    globals.register_builtin_source("cljrs.compiler.known", COMPILER_KNOWN_SOURCE);
    globals.register_builtin_source("cljrs.compiler.anf", COMPILER_ANF_SOURCE);
    globals.register_builtin_source("cljrs.compiler.escape", COMPILER_ESCAPE_SOURCE);
    globals.register_builtin_source("cljrs.compiler.optimize", COMPILER_OPTIMIZE_SOURCE);
}
