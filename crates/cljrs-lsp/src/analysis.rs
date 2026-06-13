//! Orchestrates per-document analysis: recovery → parse → diagnostics + symbols.
//!
//! This is the single seam every document update flows through. A future
//! semantic tier (hover, completion, go-to-definition) would extend [`run`] with
//! an optional `&cljrs_eval::GlobalEnv` without changing the backend.

use cljrs_reader::Parser;
use tower_lsp::lsp_types::{Diagnostic, DocumentSymbol};

use crate::diagnostics;
use crate::line_index::{LineIndex, OffsetEncoding};
use crate::recovery;
use crate::symbols;

/// The result of analyzing one document version.
#[derive(Default)]
pub struct Analysis {
    pub diagnostics: Vec<Diagnostic>,
    pub symbols: Vec<DocumentSymbol>,
}

/// Analyze `text`. `uri` is used only for error provenance.
pub fn run(text: &str, uri: &str, enc: OffsetEncoding) -> Analysis {
    let li = LineIndex::new(text);
    let split = recovery::split_top_level(text, uri);

    let mut diagnostics = Vec::new();
    let mut symbols = Vec::new();

    for chunk in &split.chunks {
        let slice = &text[chunk.start..chunk.end];
        let mut parser = Parser::new(slice.to_string(), uri.to_string());
        match parser.parse_all() {
            Ok(forms) => symbols::collect(&forms, chunk.start, &li, enc, &mut symbols),
            Err(err) => {
                let fallback_len = chunk.end - chunk.start;
                if let Some(d) =
                    diagnostics::from_read_error(&err, chunk.start, fallback_len, &li, enc)
                {
                    diagnostics.push(d);
                }
            }
        }
    }

    // Lexer-fatal errors carry absolute spans (delta 0).
    if let Some(err) = &split.lex_error
        && let Some(d) = diagnostics::from_read_error(err, 0, text.len(), &li, enc)
    {
        diagnostics.push(d);
    }

    Analysis {
        diagnostics,
        symbols,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_file_has_no_diagnostics() {
        let a = run("(ns x)\n(defn f [a] a)", "<t>", OffsetEncoding::Utf8);
        assert!(a.diagnostics.is_empty());
        assert_eq!(a.symbols.len(), 2);
    }

    #[test]
    fn reports_multiple_independent_errors() {
        // A stray closer and an unclosed list are independent top-level errors.
        let a = run("(a)\n)\n(def x", "<t>", OffsetEncoding::Utf8);
        assert_eq!(a.diagnostics.len(), 2);
    }

    #[test]
    fn symbols_survive_a_bad_form() {
        // Middle form is broken; the good forms still yield symbols.
        let a = run("(def a 1)\n)\n(defn b [] 2)", "<t>", OffsetEncoding::Utf8);
        assert_eq!(a.diagnostics.len(), 1);
        let names: Vec<_> = a.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn diagnostic_range_points_at_stray_closer() {
        let a = run("(a)\n)\n", "<t>", OffsetEncoding::Utf8);
        assert_eq!(a.diagnostics.len(), 1);
        let r = a.diagnostics[0].range;
        assert_eq!(r.start.line, 1);
        assert_eq!(r.start.character, 0);
    }

    #[test]
    fn unterminated_string_reports_lex_error() {
        let a = run("(def x 1)\n\"oops", "<t>", OffsetEncoding::Utf8);
        assert_eq!(a.symbols.len(), 1);
        assert_eq!(a.diagnostics.len(), 1);
        assert!(a.diagnostics[0].message.contains("unterminated"));
    }
}
