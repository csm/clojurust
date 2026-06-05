//! Top-level form recovery via the lexer.
//!
//! The reader's [`Parser`](cljrs_reader::Parser) aborts on the first syntax
//! error and yields no partial forms. To still report *multiple* errors and
//! produce document symbols for the well-formed parts of a buffer, we split the
//! source into top-level form "chunks" using only the [`Lexer`], then parse each
//! chunk independently with a fresh parser (see [`crate::analysis`]).
//!
//! The lexer already skips strings, regexes, char literals, line comments and
//! commas, so counting bracket depth over its delimiter tokens is reliable.
//!
//! ## Boundary detection
//!
//! A top-level unit is exactly one form. Counting *base forms* (atoms and
//! balanced bracket groups) completed at depth 0, a unit needs `1 + (number of
//! `^` metadata prefixes)` base forms: every reader-macro prefix is unary
//! (wraps one base) except `^meta target`, which needs two. Prefixes always
//! precede their targets, so the count converges incrementally and the unit
//! closes when `base_completions >= bases_needed` at depth 0.

use cljrs_reader::{Lexer, Token};
use cljrs_types::error::CljxError;

/// A half-open byte range `[start, end)` into the source covering one top-level
/// unit (or an isolated stray closing delimiter).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    pub start: usize,
    pub end: usize,
}

/// The result of splitting a buffer into top-level chunks.
pub struct SplitResult {
    pub chunks: Vec<Chunk>,
    /// A lexer-fatal error (unterminated string, bad escape, unknown `#`
    /// dispatch). When present, chunking stopped at this point; chunks before
    /// it are still valid. The error's span is absolute (into the full source).
    pub lex_error: Option<CljxError>,
}

/// Split `source` into top-level form chunks. `file` is used only for the
/// lexer's error provenance.
pub fn split_top_level(source: &str, file: &str) -> SplitResult {
    let mut lexer = Lexer::new(source.to_string(), file.to_string());
    let mut chunks = Vec::new();
    let mut lex_error = None;

    let mut depth: i32 = 0;
    let mut in_unit = false;
    let mut chunk_start: usize = 0;
    let mut bases_needed: i32 = 1;
    let mut base_completions: i32 = 0;

    loop {
        let (tok, span) = match lexer.next_token() {
            Ok(pair) => pair,
            Err(e) => {
                lex_error = Some(e);
                break;
            }
        };

        if matches!(tok, Token::Eof) {
            if in_unit {
                // Unbalanced / dangling tail — emit it so the parser reports it.
                chunks.push(Chunk {
                    start: chunk_start,
                    end: source.len(),
                });
            }
            break;
        }

        if !in_unit {
            in_unit = true;
            chunk_start = span.start;
            bases_needed = 1;
            base_completions = 0;
        }

        match tok {
            Token::LParen | Token::LBracket | Token::LBrace | Token::HashFn | Token::HashSet => {
                depth += 1;
            }
            Token::RParen | Token::RBracket | Token::RBrace => {
                if depth == 0 {
                    // Stray closing delimiter at top level. Flush any pending
                    // partial unit, isolate the stray as its own chunk (the
                    // parser will report "unexpected closing delimiter"), and
                    // resync.
                    if span.start > chunk_start {
                        chunks.push(Chunk {
                            start: chunk_start,
                            end: span.start,
                        });
                    }
                    chunks.push(Chunk {
                        start: span.start,
                        end: span.end,
                    });
                    in_unit = false;
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    base_completions += 1;
                }
            }
            // `^` needs an extra base form (the metadata) before its target.
            Token::Meta => bases_needed += 1,
            // Unary reader-macro prefixes: part of the unit, no base produced.
            Token::Quote
            | Token::SyntaxQuote
            | Token::Unquote
            | Token::UnquoteSplice
            | Token::Deref
            | Token::HashVar
            | Token::HashDiscard
            | Token::TaggedLiteral(_)
            | Token::ReaderCond
            | Token::ReaderCondSplice => {}
            // Everything else is an atom — a base form when at top level.
            _ => {
                if depth == 0 {
                    base_completions += 1;
                }
            }
        }

        if depth == 0 && in_unit && base_completions >= bases_needed {
            chunks.push(Chunk {
                start: chunk_start,
                end: span.end,
            });
            in_unit = false;
        }
    }

    SplitResult { chunks, lex_error }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(source: &str) -> Vec<&str> {
        split_top_level(source, "<test>")
            .chunks
            .iter()
            .map(|c| &source[c.start..c.end])
            .collect()
    }

    #[test]
    fn multiple_balanced_forms() {
        assert_eq!(ranges("(a) (b) (c)"), vec!["(a)", "(b)", "(c)"]);
    }

    #[test]
    fn top_level_atoms() {
        assert_eq!(ranges("1 :kw foo"), vec!["1", ":kw", "foo"]);
    }

    #[test]
    fn quote_groups_with_target() {
        assert_eq!(ranges("'(a b) `c"), vec!["'(a b)", "`c"]);
        assert_eq!(ranges("''x"), vec!["''x"]);
    }

    #[test]
    fn metadata_groups_two_bases() {
        assert_eq!(ranges("^{:m 1} x"), vec!["^{:m 1} x"]);
        assert_eq!(ranges("^:private (def y 1)"), vec!["^:private (def y 1)"]);
    }

    #[test]
    fn reader_conditional_is_one_chunk() {
        assert_eq!(ranges("#?(:rust 1 :clj 2)"), vec!["#?(:rust 1 :clj 2)"]);
    }

    #[test]
    fn discard_is_its_own_chunk() {
        // `#_ x` is discarded; `y` stands alone.
        assert_eq!(ranges("#_ x y"), vec!["#_ x", "y"]);
        assert_eq!(ranges("#_(a b) c"), vec!["#_(a b)", "c"]);
    }

    #[test]
    fn stray_closer_is_isolated() {
        let cs = ranges("(a) ) (b)");
        assert_eq!(cs, vec!["(a)", ")", "(b)"]);
    }

    #[test]
    fn unclosed_at_eof_is_trailing_chunk() {
        assert_eq!(ranges("(def x"), vec!["(def x"]);
    }

    #[test]
    fn brackets_in_strings_and_comments_ignored() {
        assert_eq!(ranges("(a \"x ) y\") (b)"), vec!["(a \"x ) y\")", "(b)"]);
        assert_eq!(ranges("(a) ; ) ) )\n(b)"), vec!["(a)", "(b)"]);
    }

    #[test]
    fn lexer_fatal_error_stops_but_keeps_prior() {
        let res = split_top_level("(a) \"unterminated", "<test>");
        assert!(res.lex_error.is_some());
        assert_eq!(res.chunks.len(), 1);
        assert_eq!(res.chunks[0], Chunk { start: 0, end: 3 });
    }

    #[test]
    fn anon_fn_and_set_literals() {
        assert_eq!(ranges("#(+ % 1) #{1 2}"), vec!["#(+ % 1)", "#{1 2}"]);
    }
}
