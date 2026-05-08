//! Pure-Rust ANF lowering, escape analysis, and region optimization.
//!
//! Replaces the Clojure-based `cljrs.compiler.anf` / `cljrs.compiler.escape` /
//! `cljrs.compiler.optimize` pipeline. No interpreter round-trip; operates
//! directly on `Form` AST nodes and produces `IrFunction` structs.

pub mod anf;
pub mod context;
pub mod escape;
pub mod known;
pub mod optimize;

pub use anf::{LowerError, lower_fn_body};
pub use optimize::optimize;
