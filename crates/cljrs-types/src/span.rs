use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// A half-open byte range `[start, end)` within a named source file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Span {
    /// File path on disk, or `"<repl>"` for interactive input.
    pub file: Arc<String>,
    /// Inclusive start byte offset.
    pub start: usize,
    /// Exclusive end byte offset.
    pub end: usize,
    /// 1-based line number of `start`.
    pub line: u32,
    /// 1-based column number of `start` (in bytes).
    pub col: u32,
}

impl Span {
    pub fn new(file: Arc<String>, start: usize, end: usize, line: u32, col: u32) -> Self {
        Self {
            file,
            start,
            end,
            line,
            col,
        }
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl From<&Span> for miette::SourceSpan {
    fn from(span: &Span) -> Self {
        miette::SourceSpan::new(miette::SourceOffset::from(span.start), span.len())
    }
}

impl From<Span> for miette::SourceSpan {
    fn from(span: Span) -> Self {
        (&span).into()
    }
}
