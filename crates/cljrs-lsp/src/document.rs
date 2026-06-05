//! Per-document state held in the backend's store.
//!
//! v1 uses FULL text sync, so a plain `String` suffices. The text lives behind
//! this small type so a later switch to INCREMENTAL sync (e.g. a rope) stays
//! localized.

use tower_lsp::lsp_types::DocumentSymbol;

/// One open text document plus its last-computed symbol outline.
pub struct Document {
    pub text: String,
    pub version: i32,
    /// Cached document symbols from the most recent analysis, reused to answer
    /// `textDocument/documentSymbol` without re-parsing.
    pub symbols: Vec<DocumentSymbol>,
}

impl Document {
    pub fn new(text: String, version: i32) -> Self {
        Self {
            text,
            version,
            symbols: Vec::new(),
        }
    }
}
