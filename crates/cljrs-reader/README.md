# cljrs-reader

Lexer (tokenizer) and recursive-descent parser for the clojurust language.
Turns raw source text into a `Form` AST that the evaluator and compiler consume.

**Phase:** 2 — lexer and parser fully implemented.

---

## File layout

```
src/
  lib.rs      — module declarations and re-exports
  token.rs    — Token enum: one variant per Clojure lexical form
  lexer.rs    — Lexer struct: byte-oriented, UTF-8-safe tokenizer
  form.rs     — Form struct + FormKind enum: the reader AST
  parser.rs   — Parser struct: recursive-descent parser + Iterator impl
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
| `Symbol(String)` | `foo`, `ns/name`, `/`, `..`, `x#` | trailing `#` (auto-gensym) is part of the symbol |
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

    pub fn source(&self) -> &Arc<String>
    pub fn file(&self) -> &Arc<String>
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

#### `#` mid-symbol (auto-gensym)

`#` is a *non-terminating* macro character: a leading `#` always triggers
`#`-dispatch (`#(`, `#{`, `#'`, …), but a `#` encountered while a symbol or
keyword token is already in progress is just appended to that token instead
of ending it. This is what makes `x#` lex as the single symbol `Symbol("x#")`
rather than `Symbol("x")` followed by a stray dispatch token — required for
auto-gensym symbols inside syntax-quote (`` `(let [x# 1] x#) ``), which
`cljrs-interp::syntax_quote` resolves to a unique `x__N__auto__` symbol per
syntax-quote form.

---

### `form::Form` / `form::FormKind`

The reader AST. Every `Form` carries a `Span` for diagnostics.

`PartialEq` on `Form` ignores spans — equality tests compare only `FormKind`.

```rust
#[derive(Debug, Clone)]
pub struct Form {
    pub kind: FormKind,
    pub span: Span,
}

impl Form {
    pub fn new(kind: FormKind, span: Span) -> Self

    /// Total heap bytes owned by this form tree, excluding the Form struct itself.
    /// Used by GcSize implementations to accurately track memory pressure.
    pub fn heap_size(&self) -> usize
}

impl FormKind {
    /// Heap bytes owned by this node and its children (recursive).
    pub fn heap_size(&self) -> usize
}

#[derive(Debug, Clone, PartialEq)]
pub enum FormKind {
    // Atoms
    Nil,
    Bool(bool),
    Int(i64),
    BigInt(String),
    Float(f64),
    BigDecimal(String),
    Ratio(String),
    Char(char),
    Str(String),
    Regex(String),
    Symbolic(f64),        // ##Inf→INFINITY  ##-Inf→NEG_INFINITY  ##NaN→NAN

    // Identifiers
    Symbol(String),
    Keyword(String),
    AutoKeyword(String),

    // Collections
    List(Vec<Form>),
    Vector(Vec<Form>),
    Map(Vec<Form>),       // flat key/value pairs; length always even
    Set(Vec<Form>),

    // Wrapping reader macros
    Quote(Box<Form>),
    SyntaxQuote(Box<Form>),
    Unquote(Box<Form>),
    UnquoteSplice(Box<Form>),
    Deref(Box<Form>),
    Var(Box<Form>),                      // #'symbol
    Meta(Box<Form>, Box<Form>),          // raw meta-form, annotated-form

    // Dispatch forms
    AnonFn(Vec<Form>),                   // #(...)
    TaggedLiteral(String, Box<Form>),    // #tag form

    // Reader conditionals — all branches kept; evaluator filters by :rust
    // clauses is flat: [keyword, form, keyword, form, …]
    ReaderCond { splicing: bool, clauses: Vec<Form> },
}
```

---

### `parser::Parser`

A recursive-descent parser that consumes `(Token, Span)` pairs from a `Lexer`
and produces `Form` nodes.

```rust
pub struct Parser { /* private */ }

impl Parser {
    /// Create a parser for `source` text labelled with `file`.
    pub fn new(source: String, file: String) -> Self

    /// Return the next form (skipping `#_` discards).
    /// Returns `Ok(None)` at EOF.
    pub fn parse_one(&mut self) -> CljxResult<Option<Form>>

    /// Parse all forms until EOF.
    pub fn parse_all(&mut self) -> CljxResult<Vec<Form>>
}

impl Iterator for Parser {
    type Item = CljxResult<Form>;
    // Yields None at EOF, Err on parse error.
}
```

#### `#_` discard semantics

`#_` consumes itself plus the next form and produces nothing. Discards can be
chained: `[#_ #_ 1 2 3]` → `[2, 3]` (outer `#_` discards the `#_ 1` group,
leaving `2` and `3`).

#### Reader conditionals

All branches of `#?(…)` and `#?@(…)` are parsed and stored as
`FormKind::ReaderCond { splicing, clauses }` with a flat `clauses` vec.  The
evaluator is responsible for filtering by `:rust`.

---

## Error construction

On any read or parse error the crate produces a `CljxError::ReadError`
containing the offending `Span` and the full source text, which miette uses to
render a pointed diagnostic in the terminal.

---

## Re-exports from `lib.rs`

```rust
pub use form::{Form, FormKind};
pub use lexer::Lexer;
pub use parser::Parser;
pub use token::Token;
```

---

## Dependencies

| Crate | Role |
|-------|------|
| `cljrs-types` (workspace) | `Span`, `CljxError`, `CljxResult` |
| `miette` (workspace) | `NamedSource` used in error construction |
