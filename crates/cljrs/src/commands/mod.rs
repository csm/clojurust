//! Internal command implementations for the `cljrs` binary.
//!
//! Each module owns one top-level subcommand: its clap arguments, its
//! `run` entry point, and any helpers that exist only to serve it.
//! [`crate::cli`] wires the argument types into the top-level `Commands` enum
//! and dispatches to the `run` functions; anything two subcommands share lives
//! in [`crate::session`] or [`crate::native`] instead.

pub mod build_native;
pub mod compile;
pub mod deps;
pub mod eval;
pub mod ir;
pub mod lsp;
pub mod nrepl;
pub mod repl;
pub mod run;
pub mod test;
pub mod trust;
