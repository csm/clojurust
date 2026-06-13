//! Pure-Rust ANF lowering, escape analysis, and region optimization.
//!
//! No interpreter round-trip; operates directly on `Form` AST nodes and
//! produces `IrFunction` structs.

pub mod anf;
pub mod context;
pub mod escape;
pub mod inline;
pub mod known;
pub mod optimize;
pub mod regionalize;

pub use anf::{LowerError, lower_fn_body, lower_fn_body_destructured};
pub use escape::{
    AnalysisResult, EscapeContext, EscapeState, ExternalDefn, UseInfo, UseKind, analyze,
};
pub use inline::inline;
pub use optimize::{optimize, optimize_with_externals};

/// Build an inter-procedural escape-analysis context for the entire IR tree
/// rooted at `ir_func`.  Pass the result to [`analyze`] (as `Some(&ctx)`) to
/// enable cross-function closure-call resolution.
pub fn make_analysis_context(ir_func: &IrFunction) -> EscapeContext {
    escape::make_context(ir_func)
}

use crate::IrFunction;
