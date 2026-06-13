//! Language Server Protocol implementation for clojurust (`.cljrs` / `.cljc`).
//!
//! v1 is syntactic only — it uses [`cljrs_reader`] for **parse diagnostics** and
//! a **document-symbol outline**, with no evaluator involvement. All analysis
//! flows through [`analysis::run`], the seam where a future semantic tier
//! (hover, completion, go-to-definition) would plug in.
//!
//! See [`backend::Backend`] for the server and [`backend::run_stdio`] /
//! [`backend::run_stdio_blocking`] for the entry points.

// CljxError embeds NamedSource<String> for miette, which is unavoidably large.
#![allow(clippy::result_large_err)]

mod analysis;
mod backend;
mod diagnostics;
mod document;
mod line_index;
mod recovery;
mod symbols;

pub use backend::{Backend, run_stdio, run_stdio_blocking};
pub use line_index::{LineIndex, OffsetEncoding};
