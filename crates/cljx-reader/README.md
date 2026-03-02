# cljx-reader

Lexer (tokenizer) and, in later phases, parser for the clojurust language.
Turns raw source text into a stream of `(Token, Span)` pairs that the evaluator
and compiler consume.

**Phase:** 2 (Lexer) — lexer fully implemented; parser (`Form` AST) planned for Phase 3.

---

## File layout

```
src/
  lib.rs      — module declarations and re-exports of Lexer + Token
  token.rs    — Token enum: one variant per Clojure lexical form
  lexer.rs    — Lexer struct: byte-oriented, UTF-8-safe tokenizer
```

---

## Public API

### `token::Token`

Every distinct lexical form the reader can produce:

| Variant | Clojure source | Notes |
|---------|---------------|-------|
| `Nil` | `nil` | |
| `Bool(bool)` | `true` / `false` | |
| `Int(i64)` | `42`, `-7`, `16rFF`, `2r1010` | decimal or radix literal that fits `i64` |
| `BigInt(String)` | `42N`, overflowing radix | decimal digits; sign included when negative |
| `Float(f64)` | `3.14`, `1e10`, `1.5e-3` | |
| `BigDecimal(String)` | `3.14M` | raw text without trailing `M` |
| `Ratio(String)` | `3/4`, `-1/2` | full text including `/` |
| `Char(char)` | `\a`, `\newline`, `\u0041` | named chars and `\uXXXX` resolved |
| `Str(String)` | `"hello\n"` | escape sequences fully processed |
| `Symbol(String)` | `foo`, `ns/name`, `/`, `..` | |
| `Keyword(String)` | `:foo`, `:ns/name` | leading `:` stripped |
| `AutoKeyword(String)` | `::foo`, `::ns/alias` | leading `::` stripped |
| `LParen` / `RParen` | `(` / `)` | |
| `LBracket` / `RBracket` | `[` / `]` | |
| `LBrace` / `RBrace` | `{` / `}` | |
| `Quote` | `'` | |
| `SyntaxQuote` | `` ` `` | |
| `Unquote` | `~` | |
| `UnquoteSplice` | `~@` | |
| `Deref` | `@` | |
| `Meta` | `^` | |
| `HashFn` | `#(` | |
| `HashSet` | `#{` | |
| `HashVar` | `#'` | |
| `HashDiscard` | `#_` | |
| `Regex(String)` | `#"[a-z]+"` | raw pattern; no escape processing |
| `ReaderCond` | `#?` | |
| `ReaderCondSplice` | `#?@` | |
| `Symbolic(String)` | `##Inf`, `##NaN` | stores suffix after `##` |
| `TaggedLiteral(String)` | `#inst`, `#uuid` | stores tag name without `#` |
| `Eof` | — | end-of-file sentinel |

### `lexer::Lexer`

A byte-oriented, UTF-8-safe tokenizer. Tracks byte position, 1-based line, and
1-based byte column so every token carries a precise `Span`.

```rust
pub struct Lexer { /* private */ }

impl Lexer {
    /// Create a new lexer for `source` text from `file` (path or `"<repl>"`).
    pub fn new(source: String, file: String) -> Self

    /// Return the next `(Token, Span)` pair.
    /// Returns `Ok((Token::Eof, _))` at end of input.
    /// Returns `Err(CljxError::ReadError { … })` on invalid input.
    pub fn next_token(&mut self) -> CljxResult<(Token, Span)>
}

impl Iterator for Lexer {
    type Item = CljxResult<(Token, Span)>;
    // Yields None when next_token returns Token::Eof.
}
```

#### Whitespace and comment handling

- ASCII spaces, tabs, carriage returns, newlines, and commas are skipped.
- `;` through end-of-line is a line comment.
- `#!` at the very start of the file (byte offset 0) is a shebang; the rest of
  that line is skipped.

#### Number parsing rules

- `+`/`-` are only routed to the number path when immediately followed by an
  ASCII digit; otherwise they lex as symbols.
- `3/foo` lexes as `Int(3)` then `Symbol("/foo")`, not a ratio — the `/` is only
  consumed as part of a ratio when the character immediately after it is a digit.
- Radix literals: `NNrDIGITS` where `NN` is 2–36. Overflow of `i64` yields
  `BigInt`.

#### Error construction

On any lex error the lexer produces a `CljxError::ReadError` containing the
offending `Span` and the full source text, which miette uses to render a
pointed diagnostic in the terminal.

---

## Re-exports from `lib.rs`

```rust
pub use lexer::Lexer;
pub use token::Token;
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljx-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `miette` (workspace) | `NamedSource` used in error construction |
