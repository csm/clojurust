#![allow(clippy::result_large_err)]
#![allow(clippy::type_complexity)]
//! Tree-walking interpreter: special forms, macro expansion, destructuring,
//! and function application.
//!
//! Environment construction lives in [`crate::runtime`]; build one with
//! `Runtime::builder().execution_mode(ExecutionMode::TreeWalk)`.

pub mod apply;
pub mod arity;
mod capture;
pub mod destructure;
pub mod eval;
pub mod macros;
pub mod special;
pub mod syntax_quote;
#[cfg(not(target_arch = "wasm32"))]
pub mod versioned;
mod virtualize;
