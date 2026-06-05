//! Map reader errors to LSP [`Diagnostic`]s.

use cljrs_types::error::CljxError;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity};

use crate::line_index::{LineIndex, OffsetEncoding};

/// Convert a [`CljxError`] into a [`Diagnostic`].
///
/// `delta` is added to the error's byte offset to remap a chunk-local span back
/// to document coordinates (pass `0` for errors whose spans are already
/// absolute, e.g. lexer-fatal errors). `fallback_len` is used when the error
/// carries no span.
pub fn from_read_error(
    err: &CljxError,
    delta: usize,
    fallback_len: usize,
    li: &LineIndex,
    enc: OffsetEncoding,
) -> Option<Diagnostic> {
    let (message, span) = match err {
        CljxError::ReadError { message, span, .. } => (message.clone(), *span),
        // In-memory parsing cannot produce Io/Serialization/Eval errors.
        _ => return None,
    };

    let (start, len) = match span {
        Some(s) => (s.offset() + delta, s.len().max(1)),
        None => (delta, fallback_len.max(1)),
    };

    Some(Diagnostic {
        range: li.range(start, start + len, enc),
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("cljrs".to_string()),
        message,
        ..Default::default()
    })
}
