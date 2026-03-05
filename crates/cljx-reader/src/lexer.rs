// CljxError embeds NamedSource<String> for miette diagnostics, which is
// unavoidably large. Suppress the false-positive for every returning function.
#![allow(clippy::result_large_err)]

use std::sync::Arc;

use miette::NamedSource;

use cljx_types::error::{CljxError, CljxResult};
use cljx_types::span::Span;

use crate::token::Token;

// ─── Character classification ─────────────────────────────────────────────────

/// Returns `true` if `ch` is a valid constituent character for a symbol or
/// keyword.  Defined *negatively*: everything that isn't a delimiter, whitespace,
/// or special syntax character is a symbol constituent.
fn is_symbol_char(ch: char) -> bool {
    !matches!(
        ch,
        ' ' | '\t'
            | '\n'
            | '\r'
            | ','
            | '('
            | ')'
            | '['
            | ']'
            | '{'
            | '}'
            | '"'
            | ';'
            | '`'
            | '~'
            | '^'
            | '@'
            | '#'
            | '\\'
            | ':'
    )
}

/// Returns `true` if `ch` can *start* a symbol (not a digit, not `+`/`-` when
/// the following char is a digit — but the caller handles the `+`/`-` case).
fn is_symbol_start(ch: char) -> bool {
    is_symbol_char(ch) && !ch.is_ascii_digit()
}

// ─── Lexer ───────────────────────────────────────────────────────────────────

pub struct Lexer {
    source: Arc<String>,
    file: Arc<String>,
    pos: usize, // byte offset, always on a char boundary
    line: u32,  // 1-based
    col: u32,   // 1-based byte offset from line start
}

impl Lexer {
    pub fn new(source: String, file: String) -> Self {
        Self {
            source: Arc::new(source),
            file: Arc::new(file),
            pos: 0,
            line: 1,
            col: 1,
        }
    }

    // ── Public getters ────────────────────────────────────────────────────

    pub fn source(&self) -> &Arc<String> {
        &self.source
    }

    pub fn file(&self) -> &Arc<String> {
        &self.file
    }

    // ── Low-level helpers ─────────────────────────────────────────────────

    fn peek(&self) -> Option<char> {
        self.source[self.pos..].chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let mut chars = self.source[self.pos..].chars();
        chars.next(); // skip current
        chars.next()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek()?;
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += ch.len_utf8() as u32;
        }
        Some(ch)
    }

    fn span_from(&self, start_pos: usize, start_line: u32, start_col: u32) -> Span {
        Span::new(
            Arc::clone(&self.file),
            start_pos,
            self.pos,
            start_line,
            start_col,
        )
    }

    fn make_error(&self, msg: impl Into<String>, span: Span) -> CljxError {
        CljxError::ReadError {
            message: msg.into(),
            span: Some(miette::SourceSpan::from(span)),
            src: NamedSource::new((*self.file).clone(), (*self.source).clone()),
        }
    }

    /// Consume characters while `is_symbol_char` holds, returning the collected
    /// string.
    fn read_symbol_chars(&mut self) -> String {
        let mut buf = String::new();
        while let Some(ch) = self.peek() {
            if is_symbol_char(ch) {
                buf.push(ch);
                self.advance();
            } else {
                break;
            }
        }
        buf
    }

    // ── Whitespace / comment skipping ─────────────────────────────────────

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.peek() {
                // Shebang: only recognised at the very start of the file.
                Some('#') if self.pos == 0 => {
                    if self.peek_next() == Some('!') {
                        // skip to end of line
                        while let Some(ch) = self.advance() {
                            if ch == '\n' {
                                break;
                            }
                        }
                    } else {
                        break; // '#' is meaningful, stop skipping
                    }
                }
                Some(' ') | Some('\t') | Some('\r') | Some('\n') | Some(',') => {
                    self.advance();
                }
                Some(';') => {
                    while let Some(ch) = self.advance() {
                        if ch == '\n' {
                            break;
                        }
                    }
                }
                _ => break,
            }
        }
    }

    // ── `~` (unquote / unquote-splicing) ─────────────────────────────────

    fn lex_unquote(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume '~'
        if self.peek() == Some('@') {
            self.advance();
            Ok((
                Token::UnquoteSplice,
                self.span_from(start_pos, start_line, start_col),
            ))
        } else {
            Ok((
                Token::Unquote,
                self.span_from(start_pos, start_line, start_col),
            ))
        }
    }

    // ── `#` dispatch ──────────────────────────────────────────────────────

    fn lex_hash(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume '#'
        match self.peek() {
            Some('(') => {
                self.advance();
                Ok((
                    Token::HashFn,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            Some('{') => {
                self.advance();
                Ok((
                    Token::HashSet,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            Some('\'') => {
                self.advance();
                Ok((
                    Token::HashVar,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            Some('_') => {
                self.advance();
                Ok((
                    Token::HashDiscard,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            Some('"') => self.lex_regex(start_pos, start_line, start_col),
            Some('?') => {
                self.advance(); // consume '?'
                if self.peek() == Some('@') {
                    self.advance();
                    Ok((
                        Token::ReaderCondSplice,
                        self.span_from(start_pos, start_line, start_col),
                    ))
                } else {
                    Ok((
                        Token::ReaderCond,
                        self.span_from(start_pos, start_line, start_col),
                    ))
                }
            }
            Some('#') => self.lex_symbolic(start_pos, start_line, start_col),
            Some(c) if is_symbol_start(c) => {
                let name = self.read_symbol_chars();
                Ok((
                    Token::TaggedLiteral(name),
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            other => {
                let span = self.span_from(start_pos, start_line, start_col);
                Err(self.make_error(format!("unknown # dispatch character: {:?}", other), span))
            }
        }
    }

    fn lex_regex(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume opening '"'
        let mut buf = String::new();
        loop {
            match self.advance() {
                None => {
                    let span = self.span_from(start_pos, start_line, start_col);
                    return Err(self.make_error("unterminated regex literal", span));
                }
                Some('"') => break,
                Some('\\') => {
                    // Store escape verbatim (two chars) — no processing.
                    buf.push('\\');
                    match self.advance() {
                        Some(c) => buf.push(c),
                        None => {
                            let span = self.span_from(start_pos, start_line, start_col);
                            return Err(self.make_error("unterminated regex literal", span));
                        }
                    }
                }
                Some(c) => buf.push(c),
            }
        }
        Ok((
            Token::Regex(buf),
            self.span_from(start_pos, start_line, start_col),
        ))
    }

    fn lex_symbolic(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume second '#'
        let name = self.read_symbol_chars();
        match name.as_str() {
            "Inf" | "-Inf" | "NaN" => Ok((
                Token::Symbolic(name),
                self.span_from(start_pos, start_line, start_col),
            )),
            _ => {
                let span = self.span_from(start_pos, start_line, start_col);
                Err(self.make_error(format!("unknown symbolic value: ##{name}"), span))
            }
        }
    }

    // ── String literal ────────────────────────────────────────────────────

    fn lex_string(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume opening '"'
        let mut buf = String::new();
        loop {
            match self.advance() {
                None => {
                    let span = self.span_from(start_pos, start_line, start_col);
                    return Err(self.make_error("unterminated string literal", span));
                }
                Some('"') => break,
                Some('\\') => match self.advance() {
                    Some('n') => buf.push('\n'),
                    Some('t') => buf.push('\t'),
                    Some('r') => buf.push('\r'),
                    Some('b') => buf.push('\x08'),
                    Some('f') => buf.push('\x0C'),
                    Some('\\') => buf.push('\\'),
                    Some('"') => buf.push('"'),
                    Some('u') => {
                        let ch = self.read_unicode_escape(start_pos, start_line, start_col)?;
                        buf.push(ch);
                    }
                    Some(c) => {
                        let span = self.span_from(start_pos, start_line, start_col);
                        return Err(self.make_error(format!("unknown string escape: \\{c}"), span));
                    }
                    None => {
                        let span = self.span_from(start_pos, start_line, start_col);
                        return Err(self.make_error("unterminated string literal", span));
                    }
                },
                Some(c) => buf.push(c),
            }
        }
        Ok((
            Token::Str(buf),
            self.span_from(start_pos, start_line, start_col),
        ))
    }

    /// Read exactly 4 hex digits after `\u` and return the corresponding char.
    fn read_unicode_escape(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<char> {
        let mut hex = String::with_capacity(4);
        for _ in 0..4 {
            match self.advance() {
                Some(c) if c.is_ascii_hexdigit() => hex.push(c),
                Some(c) => {
                    let span = self.span_from(start_pos, start_line, start_col);
                    return Err(self.make_error(
                        format!("invalid \\u escape: expected hex digit, got {c:?}"),
                        span,
                    ));
                }
                None => {
                    let span = self.span_from(start_pos, start_line, start_col);
                    return Err(self.make_error("unterminated \\u escape", span));
                }
            }
        }
        let code = u32::from_str_radix(&hex, 16).unwrap();
        char::from_u32(code).ok_or_else(|| {
            let span = self.span_from(start_pos, start_line, start_col);
            self.make_error(format!("invalid unicode code point: \\u{hex}"), span)
        })
    }

    // ── Character literal `\X` ────────────────────────────────────────────

    fn lex_char_literal(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume '\'

        // Peek ahead at all symbol-constituent chars to figure out the name.
        let rest_start = self.pos;
        let rest: String = self.source[rest_start..]
            .chars()
            .take_while(|&c| c.is_alphanumeric() || c == '-')
            .collect();

        let ch = match rest.as_str() {
            "newline" => {
                self.pos += "newline".len();
                self.col += "newline".len() as u32;
                '\n'
            }
            "space" => {
                self.pos += "space".len();
                self.col += "space".len() as u32;
                ' '
            }
            "tab" => {
                self.pos += "tab".len();
                self.col += "tab".len() as u32;
                '\t'
            }
            "backspace" => {
                self.pos += "backspace".len();
                self.col += "backspace".len() as u32;
                '\x08'
            }
            "formfeed" => {
                self.pos += "formfeed".len();
                self.col += "formfeed".len() as u32;
                '\x0C'
            }
            "return" => {
                self.pos += "return".len();
                self.col += "return".len() as u32;
                '\r'
            }
            _ if rest.starts_with('u') && rest.len() >= 5 => {
                // Try \uXXXX
                let hex_part = &rest[1..5];
                if hex_part.chars().all(|c| c.is_ascii_hexdigit()) {
                    let code = u32::from_str_radix(hex_part, 16).unwrap();
                    let c = char::from_u32(code).ok_or_else(|| {
                        let span = self.span_from(start_pos, start_line, start_col);
                        self.make_error(
                            format!("invalid unicode code point in char literal: \\u{hex_part}"),
                            span,
                        )
                    })?;
                    // advance 5 bytes: 'u' + 4 hex digits
                    self.pos += 5;
                    self.col += 5;
                    c
                } else {
                    let span = self.span_from(start_pos, start_line, start_col);
                    return Err(self.make_error(format!("unknown character name: {rest}"), span));
                }
            }
            _ if rest.len() == 1 => {
                // Single ASCII or first char
                let c = self.source[rest_start..].chars().next().unwrap();
                self.pos += c.len_utf8();
                self.col += c.len_utf8() as u32;
                c
            }
            _ if rest.is_empty() => {
                // Nothing after backslash — try a single non-alphanumeric char
                match self.source[rest_start..].chars().next() {
                    Some(c) => {
                        self.pos += c.len_utf8();
                        self.col += c.len_utf8() as u32;
                        c
                    }
                    None => {
                        let span = self.span_from(start_pos, start_line, start_col);
                        return Err(self.make_error("unexpected end of file after \\", span));
                    }
                }
            }
            _ => {
                let span = self.span_from(start_pos, start_line, start_col);
                return Err(self.make_error(format!("unknown character name: {rest}"), span));
            }
        };

        Ok((
            Token::Char(ch),
            self.span_from(start_pos, start_line, start_col),
        ))
    }

    // ── Keyword ───────────────────────────────────────────────────────────

    fn lex_keyword(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        self.advance(); // consume first ':'
        if self.peek() == Some(':') {
            self.advance(); // consume second ':'
            let name = self.read_symbol_chars();
            if name.is_empty() {
                let span = self.span_from(start_pos, start_line, start_col);
                return Err(self.make_error("empty auto-resolved keyword", span));
            }
            Ok((
                Token::AutoKeyword(name),
                self.span_from(start_pos, start_line, start_col),
            ))
        } else {
            let name = self.read_symbol_chars();
            if name.is_empty() {
                let span = self.span_from(start_pos, start_line, start_col);
                return Err(self.make_error("empty keyword", span));
            }
            Ok((
                Token::Keyword(name),
                self.span_from(start_pos, start_line, start_col),
            ))
        }
    }

    // ── Symbol (and nil/true/false) ────────────────────────────────────────

    fn lex_symbol(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        let name = self.read_symbol_chars();
        let tok = match name.as_str() {
            "nil" => Token::Nil,
            "true" => Token::Bool(true),
            "false" => Token::Bool(false),
            _ => Token::Symbol(name),
        };
        Ok((tok, self.span_from(start_pos, start_line, start_col)))
    }

    // ── Number ────────────────────────────────────────────────────────────

    fn lex_number(
        &mut self,
        start_pos: usize,
        start_line: u32,
        start_col: u32,
    ) -> CljxResult<(Token, Span)> {
        // Optional sign
        let negative = match self.peek() {
            Some('-') => {
                self.advance();
                true
            }
            Some('+') => {
                self.advance();
                false
            }
            _ => false,
        };
        let sign_str = if negative { "-" } else { "" };

        // Integer part (decimal digits)
        let mut int_part = String::new();
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                int_part.push(c);
                self.advance();
            } else {
                break;
            }
        }

        // Hex literal: 0x / 0X  (also -0x…)
        if int_part == "0" && matches!(self.peek(), Some('x') | Some('X')) {
            self.advance(); // consume 'x'/'X'
            let mut hex = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_hexdigit() {
                    hex.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            if hex.is_empty() {
                let span = self.span_from(start_pos, start_line, start_col);
                return Err(self.make_error("expected hex digits after 0x", span));
            }
            let value = u128::from_str_radix(&hex, 16).unwrap_or(u128::MAX);
            let span = self.span_from(start_pos, start_line, start_col);
            return if negative {
                // -0x8000000000000000 == i64::MIN is valid; anything larger overflows.
                if value <= (i64::MAX as u128) + 1 {
                    Ok((Token::Int(0i64.wrapping_sub(value as i64)), span))
                } else {
                    // Store as signed decimal string for BigInt.
                    Ok((Token::BigInt(format!("-{value}")), span))
                }
            } else if value <= i64::MAX as u128 {
                Ok((Token::Int(value as i64), span))
            } else {
                Ok((Token::BigInt(value.to_string()), span))
            };
        }

        // Radix literal: NNrDIGITS
        if matches!(self.peek(), Some('r') | Some('R')) {
            let radix: u32 = int_part.parse().unwrap_or(0);
            self.advance(); // consume 'r'/'R'
            let mut digits = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_alphanumeric() {
                    digits.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            let mut value: u128 = 0;
            for c in digits.chars() {
                let d = c.to_digit(radix).ok_or_else(|| {
                    let span = self.span_from(start_pos, start_line, start_col);
                    self.make_error(format!("invalid digit {c:?} for radix {radix}"), span)
                })?;
                value = value.wrapping_mul(radix as u128).wrapping_add(d as u128);
            }
            if negative {
                // Check if it fits as negative i64
                if value <= (i64::MAX as u128) + 1 {
                    let signed = -(value as i64);
                    return Ok((
                        Token::Int(signed),
                        self.span_from(start_pos, start_line, start_col),
                    ));
                } else {
                    // Store as decimal string with sign
                    return Ok((
                        Token::BigInt(format!("-{value}")),
                        self.span_from(start_pos, start_line, start_col),
                    ));
                }
            } else if value <= i64::MAX as u128 {
                return Ok((
                    Token::Int(value as i64),
                    self.span_from(start_pos, start_line, start_col),
                ));
            } else {
                return Ok((
                    Token::BigInt(value.to_string()),
                    self.span_from(start_pos, start_line, start_col),
                ));
            }
        }

        // BigInt suffix 'N'
        if self.peek() == Some('N') {
            self.advance();
            return Ok((
                Token::BigInt(format!("{sign_str}{int_part}")),
                self.span_from(start_pos, start_line, start_col),
            ));
        }

        // Float: decimal point or exponent
        if matches!(self.peek(), Some('.') | Some('e') | Some('E')) {
            let mut raw = format!("{sign_str}{int_part}");
            if self.peek() == Some('.') {
                raw.push('.');
                self.advance();
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        raw.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            if matches!(self.peek(), Some('e') | Some('E')) {
                raw.push('e');
                self.advance();
                if matches!(self.peek(), Some('+') | Some('-')) {
                    raw.push(self.peek().unwrap());
                    self.advance();
                }
                while let Some(c) = self.peek() {
                    if c.is_ascii_digit() {
                        raw.push(c);
                        self.advance();
                    } else {
                        break;
                    }
                }
            }
            // BigDecimal suffix 'M'
            if self.peek() == Some('M') {
                self.advance();
                return Ok((
                    Token::BigDecimal(raw),
                    self.span_from(start_pos, start_line, start_col),
                ));
            }
            let val: f64 = raw.parse().map_err(|_| {
                let span = self.span_from(start_pos, start_line, start_col);
                self.make_error(format!("invalid float: {raw}"), span)
            })?;
            return Ok((
                Token::Float(val),
                self.span_from(start_pos, start_line, start_col),
            ));
        }

        // Ratio: INT/DIGITS — only if next char after '/' is a digit
        if self.peek() == Some('/') && matches!(self.peek_next(), Some(c) if c.is_ascii_digit()) {
            self.advance(); // consume '/'
            let mut denom = String::new();
            while let Some(c) = self.peek() {
                if c.is_ascii_digit() {
                    denom.push(c);
                    self.advance();
                } else {
                    break;
                }
            }
            return Ok((
                Token::Ratio(format!("{sign_str}{int_part}/{denom}")),
                self.span_from(start_pos, start_line, start_col),
            ));
        }

        // Plain integer
        let full = format!("{sign_str}{int_part}");
        match full.parse::<i64>() {
            Ok(n) => Ok((
                Token::Int(n),
                self.span_from(start_pos, start_line, start_col),
            )),
            Err(_) => {
                // Overflow: store decimal string
                Ok((
                    Token::BigInt(full),
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
        }
    }

    // ── Top-level token dispatch ──────────────────────────────────────────

    pub fn next_token(&mut self) -> CljxResult<(Token, Span)> {
        self.skip_whitespace_and_comments();

        let start_pos = self.pos;
        let start_line = self.line;
        let start_col = self.col;

        let ch = match self.peek() {
            None => {
                return Ok((Token::Eof, self.span_from(start_pos, start_line, start_col)));
            }
            Some(c) => c,
        };

        match ch {
            '(' => {
                self.advance();
                Ok((
                    Token::LParen,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            ')' => {
                self.advance();
                Ok((
                    Token::RParen,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '[' => {
                self.advance();
                Ok((
                    Token::LBracket,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            ']' => {
                self.advance();
                Ok((
                    Token::RBracket,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '{' => {
                self.advance();
                Ok((
                    Token::LBrace,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '}' => {
                self.advance();
                Ok((
                    Token::RBrace,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '\'' => {
                self.advance();
                Ok((
                    Token::Quote,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '`' => {
                self.advance();
                Ok((
                    Token::SyntaxQuote,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '@' => {
                self.advance();
                Ok((
                    Token::Deref,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '^' => {
                self.advance();
                Ok((
                    Token::Meta,
                    self.span_from(start_pos, start_line, start_col),
                ))
            }
            '~' => self.lex_unquote(start_pos, start_line, start_col),
            '#' => self.lex_hash(start_pos, start_line, start_col),
            '"' => self.lex_string(start_pos, start_line, start_col),
            '\\' => self.lex_char_literal(start_pos, start_line, start_col),
            ':' => self.lex_keyword(start_pos, start_line, start_col),
            c if c.is_ascii_digit() => self.lex_number(start_pos, start_line, start_col),
            '+' | '-' if matches!(self.peek_next(), Some(d) if d.is_ascii_digit()) => {
                self.lex_number(start_pos, start_line, start_col)
            }
            c if is_symbol_start(c) => self.lex_symbol(start_pos, start_line, start_col),
            // '+' and '-' alone (or before non-digit) are symbols
            '+' | '-' => self.lex_symbol(start_pos, start_line, start_col),
            c => {
                self.advance();
                let span = self.span_from(start_pos, start_line, start_col);
                Err(self.make_error(format!("unexpected character: {c:?}"), span))
            }
        }
    }
}

impl Iterator for Lexer {
    type Item = CljxResult<(Token, Span)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_token() {
            Ok((Token::Eof, _)) => None,
            result => Some(result),
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn lex_all(src: &str) -> Vec<Token> {
        Lexer::new(src.to_string(), "<test>".to_string())
            .map(|r: CljxResult<(Token, Span)>| r.expect("lex error").0)
            .collect()
    }

    fn lex_one(src: &str) -> Token {
        let mut l = Lexer::new(src.to_string(), "<test>".to_string());
        l.next_token().expect("lex error").0
    }

    fn lex_err(src: &str) -> String {
        let mut l = Lexer::new(src.to_string(), "<test>".to_string());
        loop {
            match l.next_token() {
                Err(CljxError::ReadError { message, .. }) => return message,
                Err(e) => panic!("unexpected error type: {e}"),
                Ok((Token::Eof, _)) => panic!("expected an error but got Eof"),
                Ok(_) => {}
            }
        }
    }

    // ── nil / bool ────────────────────────────────────────────────────────

    #[test]
    fn test_nil() {
        assert_eq!(lex_one("nil"), Token::Nil);
    }

    #[test]
    fn test_bool() {
        assert_eq!(lex_one("true"), Token::Bool(true));
        assert_eq!(lex_one("false"), Token::Bool(false));
    }

    // ── Integers ──────────────────────────────────────────────────────────

    #[test]
    fn test_int_plain() {
        assert_eq!(lex_one("42"), Token::Int(42));
        assert_eq!(lex_one("-42"), Token::Int(-42));
        assert_eq!(lex_one("+42"), Token::Int(42));
        assert_eq!(lex_one("0"), Token::Int(0));
    }

    #[test]
    fn test_bigint_suffix() {
        assert_eq!(lex_one("42N"), Token::BigInt("42".to_string()));
        assert_eq!(lex_one("-42N"), Token::BigInt("-42".to_string()));
    }

    #[test]
    fn test_hex_literal() {
        assert_eq!(lex_one("0xff"), Token::Int(255));
        assert_eq!(lex_one("0xFF"), Token::Int(255));
        assert_eq!(lex_one("0x0"), Token::Int(0));
        assert_eq!(lex_one("0x7FFFFFFFFFFFFFFF"), Token::Int(i64::MAX));
        assert_eq!(lex_one("-0x8000000000000000"), Token::Int(i64::MIN));
        assert_eq!(lex_one("-0xff"), Token::Int(-255));
        // Overflow → BigInt
        match lex_one("0xFFFFFFFFFFFFFFFF") {
            Token::BigInt(_) => {}
            other => panic!("expected BigInt for 0xFFFF…, got {other:?}"),
        }
    }

    #[test]
    fn test_radix() {
        assert_eq!(lex_one("2r1010"), Token::Int(10));
        assert_eq!(lex_one("8r77"), Token::Int(63));
        assert_eq!(lex_one("16rFF"), Token::Int(255));
        assert_eq!(lex_one("16rff"), Token::Int(255));
        assert_eq!(lex_one("36rZ"), Token::Int(35));
    }

    #[test]
    fn test_radix_overflow() {
        // 2^64 fits in u128 but not i64
        let tok = lex_one("10r18446744073709551616");
        match tok {
            Token::BigInt(_) => {}
            other => panic!("expected BigInt, got {other:?}"),
        }
    }

    // ── Floats ────────────────────────────────────────────────────────────

    #[test]
    fn test_floats() {
        assert_eq!(lex_one("3.14"), Token::Float(3.14));
        assert_eq!(lex_one("1e10"), Token::Float(1e10));
        assert_eq!(lex_one("1.5e-3"), Token::Float(1.5e-3));
        assert_eq!(lex_one("-0.5"), Token::Float(-0.5));
    }

    #[test]
    fn test_bigdecimal() {
        assert_eq!(lex_one("3.14M"), Token::BigDecimal("3.14".to_string()));
        assert_eq!(lex_one("1e5M"), Token::BigDecimal("1e5".to_string()));
    }

    // ── Ratio ────────────────────────────────────────────────────────────

    #[test]
    fn test_ratio() {
        assert_eq!(lex_one("3/4"), Token::Ratio("3/4".to_string()));
        assert_eq!(lex_one("-1/2"), Token::Ratio("-1/2".to_string()));
    }

    #[test]
    fn test_ratio_vs_symbol() {
        // "3/foo" should lex as Int(3) then Symbol("/foo") — not a ratio
        let toks = lex_all("3/foo");
        assert_eq!(toks[0], Token::Int(3));
        assert_eq!(toks[1], Token::Symbol("/foo".to_string()));
    }

    // ── Char literals ────────────────────────────────────────────────────

    #[test]
    fn test_char_simple() {
        assert_eq!(lex_one("\\a"), Token::Char('a'));
    }

    #[test]
    fn test_char_named() {
        assert_eq!(lex_one("\\newline"), Token::Char('\n'));
        assert_eq!(lex_one("\\space"), Token::Char(' '));
        assert_eq!(lex_one("\\tab"), Token::Char('\t'));
        assert_eq!(lex_one("\\backspace"), Token::Char('\x08'));
        assert_eq!(lex_one("\\formfeed"), Token::Char('\x0C'));
        assert_eq!(lex_one("\\return"), Token::Char('\r'));
    }

    #[test]
    fn test_char_unicode() {
        assert_eq!(lex_one("\\u0041"), Token::Char('A'));
        assert_eq!(lex_one("\\u00e9"), Token::Char('é'));
    }

    // ── Strings ──────────────────────────────────────────────────────────

    #[test]
    fn test_string_basic() {
        assert_eq!(lex_one("\"hello\""), Token::Str("hello".to_string()));
    }

    #[test]
    fn test_string_escapes() {
        assert_eq!(
            lex_one(r#""\n\t\r\b\f\\\"" "#),
            Token::Str("\n\t\r\x08\x0C\\\"".to_string())
        );
    }

    #[test]
    fn test_string_unicode_escape() {
        assert_eq!(lex_one("\"\\u0041\""), Token::Str("A".to_string()));
    }

    // ── Symbols ──────────────────────────────────────────────────────────

    #[test]
    fn test_symbols() {
        assert_eq!(lex_one("foo"), Token::Symbol("foo".to_string()));
        assert_eq!(lex_one("ns/name"), Token::Symbol("ns/name".to_string()));
        assert_eq!(lex_one("/"), Token::Symbol("/".to_string()));
        assert_eq!(lex_one(".."), Token::Symbol("..".to_string()));
        assert_eq!(lex_one(".method"), Token::Symbol(".method".to_string()));
        assert_eq!(lex_one("+"), Token::Symbol("+".to_string()));
        assert_eq!(lex_one("-"), Token::Symbol("-".to_string()));
        assert_eq!(lex_one("+foo"), Token::Symbol("+foo".to_string()));
    }

    // ── Keywords ─────────────────────────────────────────────────────────

    #[test]
    fn test_keyword() {
        assert_eq!(lex_one(":foo"), Token::Keyword("foo".to_string()));
        assert_eq!(lex_one(":ns/name"), Token::Keyword("ns/name".to_string()));
    }

    #[test]
    fn test_auto_keyword() {
        assert_eq!(lex_one("::foo"), Token::AutoKeyword("foo".to_string()));
        assert_eq!(
            lex_one("::ns/alias"),
            Token::AutoKeyword("ns/alias".to_string())
        );
    }

    // ── Delimiters ───────────────────────────────────────────────────────

    #[test]
    fn test_delimiters() {
        assert_eq!(
            lex_all("([{}])"),
            vec![
                Token::LParen,
                Token::LBracket,
                Token::LBrace,
                Token::RBrace,
                Token::RBracket,
                Token::RParen,
            ]
        );
    }

    // ── Reader macros ────────────────────────────────────────────────────

    #[test]
    fn test_reader_macros() {
        assert_eq!(lex_one("'x"), Token::Quote);
        assert_eq!(lex_one("`x"), Token::SyntaxQuote);
        assert_eq!(lex_one("~x"), Token::Unquote);
        assert_eq!(lex_one("~@x"), Token::UnquoteSplice);
        assert_eq!(lex_one("@x"), Token::Deref);
        assert_eq!(lex_one("^x"), Token::Meta);
    }

    // ── `#` dispatch ─────────────────────────────────────────────────────

    #[test]
    fn test_hash_dispatch() {
        assert_eq!(lex_one("#("), Token::HashFn);
        assert_eq!(lex_one("#{"), Token::HashSet);
        assert_eq!(lex_one("#'"), Token::HashVar);
        assert_eq!(lex_one("#_"), Token::HashDiscard);
        assert_eq!(lex_one("#?"), Token::ReaderCond);
        assert_eq!(lex_one("#?@"), Token::ReaderCondSplice);
    }

    #[test]
    fn test_regex() {
        assert_eq!(lex_one("#\"[a-z]+\""), Token::Regex("[a-z]+".to_string()));
    }

    #[test]
    fn test_symbolic() {
        assert_eq!(lex_one("##Inf"), Token::Symbolic("Inf".to_string()));
        assert_eq!(lex_one("##-Inf"), Token::Symbolic("-Inf".to_string()));
        assert_eq!(lex_one("##NaN"), Token::Symbolic("NaN".to_string()));
    }

    #[test]
    fn test_tagged_literal() {
        assert_eq!(lex_one("#mytag"), Token::TaggedLiteral("mytag".to_string()));
    }

    // ── Multi-token ──────────────────────────────────────────────────────

    #[test]
    fn test_multi_token() {
        let toks = lex_all("(+ 1 2)");
        assert_eq!(
            toks,
            vec![
                Token::LParen,
                Token::Symbol("+".to_string()),
                Token::Int(1),
                Token::Int(2),
                Token::RParen,
            ]
        );
    }

    // ── Whitespace / comments ────────────────────────────────────────────

    #[test]
    fn test_comma_skipped() {
        assert_eq!(lex_all("{,,,}"), vec![Token::LBrace, Token::RBrace]);
    }

    #[test]
    fn test_comment_skipped() {
        assert_eq!(lex_all("; this is a comment\n42"), vec![Token::Int(42)]);
    }

    #[test]
    fn test_shebang_skipped() {
        assert_eq!(lex_all("#!/usr/bin/env cljx\n42"), vec![Token::Int(42)]);
    }

    // ── Span tracking ────────────────────────────────────────────────────

    #[test]
    fn test_span_col() {
        let mut l = Lexer::new("  foo".to_string(), "<test>".to_string());
        let (_tok, span) = l.next_token().unwrap();
        assert_eq!(span.start, 2);
        assert_eq!(span.col, 3);
    }

    #[test]
    fn test_span_newline() {
        let mut l = Lexer::new("a\nb".to_string(), "<test>".to_string());
        l.next_token().unwrap(); // consume 'a'
        let (_tok, span) = l.next_token().unwrap(); // 'b'
        assert_eq!(span.line, 2);
        assert_eq!(span.col, 1);
    }

    // ── Errors ───────────────────────────────────────────────────────────

    #[test]
    fn test_error_unterminated_string() {
        let msg = lex_err("\"unterminated");
        assert!(msg.contains("unterminated string"));
    }

    #[test]
    fn test_error_bad_hash_dispatch() {
        // '#1' is invalid: '1' is not a symbol start and not a special dispatch char
        let msg = lex_err("#1");
        assert!(msg.contains("unknown # dispatch"));
    }

    #[test]
    fn test_error_bad_unicode_escape_in_string() {
        let msg = lex_err("\"\\uGHIJ\"");
        assert!(msg.contains("invalid") || msg.contains("hex"));
    }

    #[test]
    fn test_error_unknown_char_name() {
        let msg = lex_err("\\bogus");
        assert!(msg.contains("unknown character name"));
    }

    #[test]
    fn test_error_unknown_symbolic() {
        let msg = lex_err("##Bogus");
        assert!(msg.contains("unknown symbolic value"));
    }

    #[test]
    fn test_error_bad_string_escape() {
        let msg = lex_err("\"\\q\"");
        assert!(msg.contains("unknown string escape"));
    }
}
