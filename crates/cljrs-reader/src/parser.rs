// CljxError embeds NamedSource<String> for miette diagnostics, which is
// unavoidably large. Suppress the false-positive for every returning function.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use miette::NamedSource;

use cljrs_types::error::{CljxError, CljxResult};
use cljrs_types::span::Span;

use crate::form::{Form, FormKind};
use crate::lexer::Lexer;
use crate::token::Token;

// ─── Parser ───────────────────────────────────────────────────────────────────

pub struct Parser {
    lexer: Lexer,
    peeked: Option<(Token, Span)>,
}

impl Parser {
    pub fn new(source: String, file: String) -> Self {
        Self {
            lexer: Lexer::new(source, file),
            peeked: None,
        }
    }

    // ── Public API ────────────────────────────────────────────────────────

    /// Return the next form, skipping `#_` discards.  Returns `None` at EOF.
    pub fn parse_one(&mut self) -> CljxResult<Option<Form>> {
        loop {
            if matches!(self.peek_tok()?, Token::Eof) {
                return Ok(None);
            }
            if let Some(form) = self.parse_raw()? {
                return Ok(Some(form)); // #_ discard — loop for next form
            }
        }
    }

    /// Parse all forms until EOF, returning them as a `Vec`.
    pub fn parse_all(&mut self) -> CljxResult<Vec<Form>> {
        let mut forms = Vec::new();
        while let Some(form) = self.parse_one()? {
            forms.push(form);
        }
        Ok(forms)
    }

    // ── Lookahead helpers ─────────────────────────────────────────────────

    /// Ensure the one-token lookahead is populated.
    fn fill(&mut self) -> CljxResult<()> {
        if self.peeked.is_none() {
            let pair = self.lexer.next_token()?;
            self.peeked = Some(pair);
        }
        Ok(())
    }

    /// Consume and return the next token.
    fn bump(&mut self) -> CljxResult<(Token, Span)> {
        self.fill()?;
        Ok(self.peeked.take().unwrap())
    }

    /// Clone the next token without consuming it.
    fn peek_tok(&mut self) -> CljxResult<Token> {
        self.fill()?;
        Ok(self.peeked.as_ref().unwrap().0.clone())
    }

    /// Clone the span of the next token without consuming it.
    fn peek_span(&mut self) -> CljxResult<Span> {
        self.fill()?;
        Ok(self.peeked.as_ref().unwrap().1.clone())
    }

    // ── Error construction ────────────────────────────────────────────────

    fn make_error(&self, msg: impl Into<String>, span: Span) -> CljxError {
        CljxError::ReadError {
            message: msg.into(),
            span: Some(miette::SourceSpan::from(span)),
            src: NamedSource::new(
                (**self.lexer.file()).clone(),
                (**self.lexer.source()).clone(),
            ),
        }
    }

    // ── Span utilities ────────────────────────────────────────────────────

    fn merged_span(&self, start: &Span, end: &Span) -> Span {
        Span::new(
            Arc::clone(&start.file),
            start.start,
            end.end,
            start.line,
            start.col,
        )
    }

    // ── Core parsing ──────────────────────────────────────────────────────

    /// Read the next "thing", returning `None` for `#_` discards (and EOF).
    /// Callers that need a real form should loop past `None`.
    fn parse_raw(&mut self) -> CljxResult<Option<Form>> {
        let tok = self.peek_tok()?;
        let span = self.peek_span()?;

        match tok {
            Token::Eof => Ok(None),

            // Unexpected closing delimiters
            Token::RParen | Token::RBracket | Token::RBrace => {
                self.bump()?;
                Err(self.make_error("unexpected closing delimiter", span))
            }

            // ── Atoms ──────────────────────────────────────────────────────────
            Token::Nil => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Nil, span)))
            }
            Token::Bool(b) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Bool(b), span)))
            }
            Token::Int(n) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Int(n), span)))
            }
            Token::BigInt(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::BigInt(s), span)))
            }
            Token::Float(f) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Float(f), span)))
            }
            Token::BigDecimal(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::BigDecimal(s), span)))
            }
            Token::Ratio(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Ratio(s), span)))
            }
            Token::Char(c) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Char(c), span)))
            }
            Token::Str(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Str(s), span)))
            }
            Token::Regex(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Regex(s), span)))
            }
            Token::Symbolic(s) => {
                self.bump()?;
                let val = match s.as_str() {
                    "Inf" => f64::INFINITY,
                    "-Inf" => f64::NEG_INFINITY,
                    "NaN" => f64::NAN,
                    _ => unreachable!("lexer guarantees only Inf/-Inf/NaN"),
                };
                Ok(Some(Form::new(FormKind::Symbolic(val), span)))
            }

            // ── Identifiers ────────────────────────────────────────────────────
            Token::Symbol(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Symbol(s), span)))
            }
            Token::Keyword(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::Keyword(s), span)))
            }
            Token::AutoKeyword(s) => {
                self.bump()?;
                Ok(Some(Form::new(FormKind::AutoKeyword(s), span)))
            }

            // ── Collections ────────────────────────────────────────────────────
            Token::LParen => {
                self.bump()?;
                let (forms, close) = self.parse_seq_forms(Token::RParen, span.clone(), "list")?;
                Ok(Some(Form::new(
                    FormKind::List(forms),
                    self.merged_span(&span, &close),
                )))
            }
            Token::LBracket => {
                self.bump()?;
                let (forms, close) =
                    self.parse_seq_forms(Token::RBracket, span.clone(), "vector")?;
                Ok(Some(Form::new(
                    FormKind::Vector(forms),
                    self.merged_span(&span, &close),
                )))
            }
            Token::LBrace => {
                self.bump()?;
                let (forms, close) = self.parse_seq_forms(Token::RBrace, span.clone(), "map")?;
                if forms.len() % 2 != 0 {
                    return Err(
                        self.make_error("map literal must have an even number of forms", span)
                    );
                }
                Ok(Some(Form::new(
                    FormKind::Map(forms),
                    self.merged_span(&span, &close),
                )))
            }
            Token::HashSet => {
                self.bump()?;
                let (forms, close) = self.parse_seq_forms(Token::RBrace, span.clone(), "set")?;
                Ok(Some(Form::new(
                    FormKind::Set(forms),
                    self.merged_span(&span, &close),
                )))
            }
            Token::HashFn => {
                self.bump()?;
                let (forms, close) =
                    self.parse_seq_forms(Token::RParen, span.clone(), "anonymous function")?;
                Ok(Some(Form::new(
                    FormKind::AnonFn(forms),
                    self.merged_span(&span, &close),
                )))
            }

            // ── Wrapping reader macros ─────────────────────────────────────────
            Token::Quote => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "quoted form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::Quote(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::SyntaxQuote => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "syntax-quoted form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::SyntaxQuote(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::Unquote => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "unquoted form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::Unquote(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::UnquoteSplice => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "unquote-spliced form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::UnquoteSplice(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::Deref => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "deref form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::Deref(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::HashVar => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "var form")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::Var(Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
            Token::Meta => {
                self.bump()?;
                let meta = self.require_form(span.clone(), "meta form")?;
                let target = self.require_form(span.clone(), "annotated form")?;
                let end = target.span.clone();
                Ok(Some(Form::new(
                    FormKind::Meta(Box::new(meta), Box::new(target)),
                    self.merged_span(&span, &end),
                )))
            }

            // ── `#_` discard ───────────────────────────────────────────────────
            Token::HashDiscard => {
                self.bump()?;
                if matches!(self.peek_tok()?, Token::Eof) {
                    return Err(self.make_error("unexpected end of file after #_", span));
                }
                self.parse_raw()?; // consume & discard next form (may itself be None)
                Ok(None)
            }

            // ── Reader conditionals ────────────────────────────────────────────
            Token::ReaderCond => {
                self.bump()?;
                let form = self.parse_reader_cond(false, span)?;
                Ok(Some(form))
            }
            Token::ReaderCondSplice => {
                self.bump()?;
                let form = self.parse_reader_cond(true, span)?;
                Ok(Some(form))
            }

            // ── Tagged literal ─────────────────────────────────────────────────
            Token::TaggedLiteral(tag) => {
                self.bump()?;
                let inner = self.require_form(span.clone(), "tagged literal value")?;
                let end = inner.span.clone();
                Ok(Some(Form::new(
                    FormKind::TaggedLiteral(tag, Box::new(inner)),
                    self.merged_span(&span, &end),
                )))
            }
        }
    }

    /// Read forms until `closing` is encountered, returning the forms and the
    /// closing delimiter's span.  `open_span` is used in the unclosed-delimiter
    /// error message.
    fn parse_seq_forms(
        &mut self,
        closing: Token,
        open_span: Span,
        name: &str,
    ) -> CljxResult<(Vec<Form>, Span)> {
        let mut forms = Vec::new();
        loop {
            let tok = self.peek_tok()?;
            if tok == Token::Eof {
                return Err(self.make_error(format!("unclosed {name}"), open_span));
            }
            if tok == closing {
                let (_, close_span) = self.bump()?;
                return Ok((forms, close_span));
            }
            if let Some(form) = self.parse_raw()? {
                forms.push(form); // #_ discard — skip
            }
        }
    }

    /// Like `parse_one` but errors (pointing at `macro_span`) when EOF is
    /// reached before a form is found.
    fn require_form(&mut self, macro_span: Span, what: &str) -> CljxResult<Form> {
        loop {
            if matches!(self.peek_tok()?, Token::Eof) {
                return Err(self.make_error(
                    format!("unexpected end of file; expected {what}"),
                    macro_span,
                ));
            }
            if let Some(form) = self.parse_raw()? {
                return Ok(form); // #_ discard — keep looking
            }
        }
    }

    /// Parse the body of a reader conditional (`#?` or `#?@`).
    /// Expects `(` to be the next token; errors otherwise.
    fn parse_reader_cond(&mut self, splicing: bool, start: Span) -> CljxResult<Form> {
        let next = self.peek_tok()?;
        if next != Token::LParen {
            let span = self.peek_span()?;
            return Err(self.make_error(
                "reader conditional requires `(` immediately after `#?`",
                span,
            ));
        }
        let (_, open_span) = self.bump()?; // consume `(`
        let (clauses, close_span) =
            self.parse_seq_forms(Token::RParen, open_span.clone(), "reader conditional")?;
        if clauses.len() % 2 != 0 {
            return Err(self.make_error(
                "reader conditional must have an even number of clauses",
                open_span,
            ));
        }
        Ok(Form::new(
            FormKind::ReaderCond { splicing, clauses },
            self.merged_span(&start, &close_span),
        ))
    }
}

// ─── Iterator ─────────────────────────────────────────────────────────────────

impl Iterator for Parser {
    type Item = CljxResult<Form>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.parse_one() {
            Ok(Some(form)) => Some(Ok(form)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cljrs_types::{error::CljxError, span::Span};

    use super::*;

    // ── Helpers ───────────────────────────────────────────────────────────────

    fn dummy_span() -> Span {
        Span::new(Arc::new("<test>".to_string()), 0, 0, 1, 1)
    }

    /// Construct a Form with a dummy span (for use in assertions only).
    fn f(kind: FormKind) -> Form {
        Form::new(kind, dummy_span())
    }

    fn parse_all(src: &str) -> Vec<Form> {
        Parser::new(src.to_string(), "<test>".to_string())
            .parse_all()
            .unwrap_or_else(|e| panic!("parse error: {e}"))
    }

    fn parse1(src: &str) -> Form {
        Parser::new(src.to_string(), "<test>".to_string())
            .parse_one()
            .unwrap_or_else(|e| panic!("parse error: {e}"))
            .expect("expected a form but got EOF")
    }

    fn parse_err(src: &str) -> String {
        let mut p = Parser::new(src.to_string(), "<test>".to_string());
        match p.parse_all() {
            Err(CljxError::ReadError { message, .. }) => message,
            Err(e) => panic!("unexpected error type: {e:?}"),
            Ok(forms) => panic!("expected a parse error but got: {forms:?}"),
        }
    }

    // ── Atoms ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_nil() {
        assert_eq!(parse1("nil").kind, FormKind::Nil);
    }

    #[test]
    fn test_bool() {
        assert_eq!(parse1("true").kind, FormKind::Bool(true));
        assert_eq!(parse1("false").kind, FormKind::Bool(false));
    }

    #[test]
    fn test_int() {
        assert_eq!(parse1("42").kind, FormKind::Int(42));
        assert_eq!(parse1("-7").kind, FormKind::Int(-7));
    }

    #[test]
    fn test_bigint() {
        assert_eq!(parse1("42N").kind, FormKind::BigInt("42".to_string()));
    }

    #[test]
    fn test_float() {
        assert_eq!(parse1("3.14").kind, FormKind::Float(3.14));
        assert_eq!(parse1("1e10").kind, FormKind::Float(1e10));
    }

    #[test]
    fn test_bigdecimal() {
        assert_eq!(
            parse1("3.14M").kind,
            FormKind::BigDecimal("3.14".to_string())
        );
    }

    #[test]
    fn test_ratio() {
        assert_eq!(parse1("3/4").kind, FormKind::Ratio("3/4".to_string()));
        assert_eq!(parse1("-1/2").kind, FormKind::Ratio("-1/2".to_string()));
    }

    #[test]
    fn test_char() {
        assert_eq!(parse1("\\a").kind, FormKind::Char('a'));
        assert_eq!(parse1("\\newline").kind, FormKind::Char('\n'));
    }

    #[test]
    fn test_str() {
        assert_eq!(parse1("\"hello\"").kind, FormKind::Str("hello".to_string()));
    }

    #[test]
    fn test_regex() {
        assert_eq!(
            parse1("#\"[a-z]+\"").kind,
            FormKind::Regex("[a-z]+".to_string())
        );
    }

    #[test]
    fn test_symbolic() {
        assert!(matches!(
            parse1("##Inf").kind,
            FormKind::Symbolic(f) if f == f64::INFINITY
        ));
        assert!(matches!(
            parse1("##-Inf").kind,
            FormKind::Symbolic(f) if f == f64::NEG_INFINITY
        ));
        // NaN != NaN per IEEE 754
        assert!(matches!(
            parse1("##NaN").kind,
            FormKind::Symbolic(f) if f.is_nan()
        ));
    }

    #[test]
    fn test_symbol() {
        assert_eq!(parse1("foo").kind, FormKind::Symbol("foo".to_string()));
    }

    #[test]
    fn test_keyword() {
        assert_eq!(parse1(":foo").kind, FormKind::Keyword("foo".to_string()));
    }

    #[test]
    fn test_auto_keyword() {
        assert_eq!(
            parse1("::foo").kind,
            FormKind::AutoKeyword("foo".to_string())
        );
    }

    // ── Collections ───────────────────────────────────────────────────────────

    #[test]
    fn test_empty_list() {
        assert_eq!(parse1("()").kind, FormKind::List(vec![]));
    }

    #[test]
    fn test_list() {
        assert_eq!(
            parse1("(1 2 3)").kind,
            FormKind::List(vec![
                f(FormKind::Int(1)),
                f(FormKind::Int(2)),
                f(FormKind::Int(3)),
            ])
        );
    }

    #[test]
    fn test_vector() {
        assert_eq!(
            parse1("[1 2]").kind,
            FormKind::Vector(vec![f(FormKind::Int(1)), f(FormKind::Int(2))])
        );
    }

    #[test]
    fn test_map() {
        assert_eq!(
            parse1("{:a 1}").kind,
            FormKind::Map(vec![
                f(FormKind::Keyword("a".to_string())),
                f(FormKind::Int(1)),
            ])
        );
    }

    #[test]
    fn test_set() {
        assert_eq!(
            parse1("#{1 2}").kind,
            FormKind::Set(vec![f(FormKind::Int(1)), f(FormKind::Int(2))])
        );
    }

    // ── Nested ────────────────────────────────────────────────────────────────

    #[test]
    fn test_nested() {
        assert_eq!(
            parse1("(+ [1 2] {:a 3})").kind,
            FormKind::List(vec![
                f(FormKind::Symbol("+".to_string())),
                f(FormKind::Vector(vec![
                    f(FormKind::Int(1)),
                    f(FormKind::Int(2)),
                ])),
                f(FormKind::Map(vec![
                    f(FormKind::Keyword("a".to_string())),
                    f(FormKind::Int(3)),
                ])),
            ])
        );
    }

    // ── Reader macros ─────────────────────────────────────────────────────────

    #[test]
    fn test_quote() {
        assert_eq!(
            parse1("'foo").kind,
            FormKind::Quote(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    #[test]
    fn test_syntax_quote() {
        assert_eq!(
            parse1("`foo").kind,
            FormKind::SyntaxQuote(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    #[test]
    fn test_unquote() {
        assert_eq!(
            parse1("~foo").kind,
            FormKind::Unquote(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    #[test]
    fn test_unquote_splice() {
        assert_eq!(
            parse1("~@foo").kind,
            FormKind::UnquoteSplice(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    #[test]
    fn test_deref() {
        assert_eq!(
            parse1("@foo").kind,
            FormKind::Deref(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    #[test]
    fn test_var() {
        assert_eq!(
            parse1("#'foo").kind,
            FormKind::Var(Box::new(f(FormKind::Symbol("foo".to_string()))))
        );
    }

    // ── Meta ──────────────────────────────────────────────────────────────────

    #[test]
    fn test_meta_map() {
        assert_eq!(
            parse1("^{:a 1} foo").kind,
            FormKind::Meta(
                Box::new(f(FormKind::Map(vec![
                    f(FormKind::Keyword("a".to_string())),
                    f(FormKind::Int(1)),
                ]))),
                Box::new(f(FormKind::Symbol("foo".to_string()))),
            )
        );
    }

    #[test]
    fn test_meta_keyword() {
        assert_eq!(
            parse1("^:kw foo").kind,
            FormKind::Meta(
                Box::new(f(FormKind::Keyword("kw".to_string()))),
                Box::new(f(FormKind::Symbol("foo".to_string()))),
            )
        );
    }

    #[test]
    fn test_meta_symbol() {
        assert_eq!(
            parse1("^Sym foo").kind,
            FormKind::Meta(
                Box::new(f(FormKind::Symbol("Sym".to_string()))),
                Box::new(f(FormKind::Symbol("foo".to_string()))),
            )
        );
    }

    // ── Anonymous function ────────────────────────────────────────────────────

    #[test]
    fn test_anon_fn() {
        assert_eq!(
            parse1("#(+ % 1)").kind,
            FormKind::AnonFn(vec![
                f(FormKind::Symbol("+".to_string())),
                f(FormKind::Symbol("%".to_string())),
                f(FormKind::Int(1)),
            ])
        );
    }

    // ── #_ discard ────────────────────────────────────────────────────────────

    #[test]
    fn test_discard_simple() {
        let forms = parse_all("#_foo bar");
        assert_eq!(forms.len(), 1);
        assert_eq!(forms[0].kind, FormKind::Symbol("bar".to_string()));
    }

    #[test]
    fn test_discard_in_vector() {
        assert_eq!(
            parse1("[1 #_2 3]").kind,
            FormKind::Vector(vec![f(FormKind::Int(1)), f(FormKind::Int(3))])
        );
    }

    #[test]
    fn test_discard_chained() {
        // #_ #_ 1 2 3  →  outer #_ discards (#_ 1), then 2 and 3 remain
        let forms = parse_all("#_ #_ 1 2 3");
        assert_eq!(forms.len(), 2);
        assert_eq!(forms[0].kind, FormKind::Int(2));
        assert_eq!(forms[1].kind, FormKind::Int(3));
    }

    // ── Reader conditionals ───────────────────────────────────────────────────

    #[test]
    fn test_reader_cond() {
        assert_eq!(
            parse1("#?(:rust 1 :clj 2)").kind,
            FormKind::ReaderCond {
                splicing: false,
                clauses: vec![
                    f(FormKind::Keyword("rust".to_string())),
                    f(FormKind::Int(1)),
                    f(FormKind::Keyword("clj".to_string())),
                    f(FormKind::Int(2)),
                ],
            }
        );
    }

    #[test]
    fn test_reader_cond_splice() {
        assert_eq!(
            parse1("#?@(:rust [1 2])").kind,
            FormKind::ReaderCond {
                splicing: true,
                clauses: vec![
                    f(FormKind::Keyword("rust".to_string())),
                    f(FormKind::Vector(vec![
                        f(FormKind::Int(1)),
                        f(FormKind::Int(2)),
                    ])),
                ],
            }
        );
    }

    // ── Tagged literal ────────────────────────────────────────────────────────

    #[test]
    fn test_tagged_literal() {
        assert_eq!(
            parse1("#inst \"2024-01-01\"").kind,
            FormKind::TaggedLiteral(
                "inst".to_string(),
                Box::new(f(FormKind::Str("2024-01-01".to_string()))),
            )
        );
    }

    // ── Span tracking ─────────────────────────────────────────────────────────

    #[test]
    fn test_span_col_offset() {
        let form = parse1("  42");
        assert_eq!(form.span.start, 2);
        assert_eq!(form.span.col, 3);
    }

    #[test]
    fn test_span_multiline() {
        let forms = parse_all("a\nb");
        assert_eq!(forms[0].span.line, 1);
        assert_eq!(forms[1].span.line, 2);
    }

    // ── parse_all ─────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_all_multiple() {
        let forms = parse_all("1 2 3");
        assert_eq!(forms.len(), 3);
        assert_eq!(forms[0].kind, FormKind::Int(1));
        assert_eq!(forms[1].kind, FormKind::Int(2));
        assert_eq!(forms[2].kind, FormKind::Int(3));
    }

    // ── Errors ────────────────────────────────────────────────────────────────

    #[test]
    fn test_err_unclosed_list() {
        let msg = parse_err("(1 2");
        assert!(msg.contains("unclosed") || msg.contains("list"), "{msg}");
    }

    #[test]
    fn test_err_unexpected_close() {
        let msg = parse_err(")");
        assert!(msg.contains("unexpected"), "{msg}");
    }

    #[test]
    fn test_err_odd_map() {
        let msg = parse_err("{:a}");
        assert!(msg.contains("even") || msg.contains("map"), "{msg}");
    }

    #[test]
    fn test_err_reader_cond_non_list() {
        // #?[ is invalid; reader cond must be followed by (
        let msg = parse_err("#?[1 2]");
        assert!(
            msg.contains('(') || msg.contains("reader conditional"),
            "{msg}"
        );
    }

    #[test]
    fn test_err_odd_reader_cond_clauses() {
        let msg = parse_err("#?(:cljx)");
        assert!(
            msg.contains("even") || msg.contains("reader conditional"),
            "{msg}"
        );
    }
}
