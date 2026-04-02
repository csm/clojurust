//! Re-export IR types from the `cljrs-ir` crate.
//!
//! The IR types were extracted into `cljrs-ir` so that both `cljrs-eval`
//! (IR interpreter) and `cljrs-compiler` (codegen) can depend on them
//! without a circular dependency.

pub use cljrs_ir::*;
