/// A single lexical token produced by the clojurust lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum Token {
    // ── Atoms ────────────────────────────────────────────────────────────────
    /// The literal `nil`.
    Nil,
    /// `true` or `false`.
    Bool(bool),
    /// Decimal or radix integer that fits in `i64`.
    Int(i64),
    /// `N`-suffix integer or one that overflows `i64`; stores decimal digits
    /// without any sign or suffix (sign is implicit via the original `-`).
    BigInt(String),
    /// IEEE-754 double.
    Float(f64),
    /// `M`-suffix decimal; stores the raw text without the trailing `M`.
    BigDecimal(String),
    /// Rational literal `3/4`; stores the full text including the slash.
    Ratio(String),
    /// Character literal `\a`, `\newline`, `\u0041`, …
    Char(char),
    /// String literal with escape sequences fully processed.
    Str(String),

    // ── Identifiers ──────────────────────────────────────────────────────────
    /// A symbol: `foo`, `ns/name`, `/`, `..`
    Symbol(String),
    /// A keyword (`:foo`); the leading colon is stripped, so stores `"foo"`.
    Keyword(String),
    /// An auto-resolved keyword (`::foo`); stores `"foo"` (leading `::` stripped).
    AutoKeyword(String),

    // ── Delimiters ───────────────────────────────────────────────────────────
    LParen,
    RParen,
    LBracket,
    RBracket,
    LBrace,
    RBrace,

    // ── Reader macros ────────────────────────────────────────────────────────
    /// `'`
    Quote,
    /// `` ` ``
    SyntaxQuote,
    /// `~`
    Unquote,
    /// `~@`
    UnquoteSplice,
    /// `@`
    Deref,
    /// `^`
    Meta,

    // ── `#` dispatch ─────────────────────────────────────────────────────────
    /// `#(`
    HashFn,
    /// `#{`
    HashSet,
    /// `#'`
    HashVar,
    /// `#_`
    HashDiscard,
    /// `#"…"` — raw regex pattern, no escape processing.
    Regex(String),
    /// `#?`
    ReaderCond,
    /// `#?@`
    ReaderCondSplice,
    /// `##Inf` / `##-Inf` / `##NaN`; stores the suffix after `##`.
    Symbolic(String),
    /// `#tag` — tagged literal; stores the symbol name without the leading `#`.
    TaggedLiteral(String),

    /// End-of-file sentinel.
    Eof,
}
