use std::mem;

use cljrs_types::span::Span;

/// A parsed Clojure form with its source location.
///
/// `PartialEq` ignores spans so test assertions can compare forms without
/// constructing exact span values.
#[derive(Debug, Clone)]
pub struct Form {
    pub kind: FormKind,
    pub span: Span,
}

impl Form {
    pub fn new(kind: FormKind, span: Span) -> Self {
        Self { kind, span }
    }

    /// Total heap bytes owned by this form tree (excluding the `Form` itself).
    pub fn heap_size(&self) -> usize {
        mem::size_of::<FormKind>() + self.kind.heap_size()
    }
}

impl FormKind {
    /// Heap bytes owned by this node and all children.
    pub fn heap_size(&self) -> usize {
        match self {
            // Inline scalars — no heap.
            FormKind::Nil
            | FormKind::Bool(_)
            | FormKind::Int(_)
            | FormKind::Float(_)
            | FormKind::Char(_)
            | FormKind::Symbolic(_) => 0,

            // String payloads.
            FormKind::BigInt(s)
            | FormKind::BigDecimal(s)
            | FormKind::Ratio(s)
            | FormKind::Str(s)
            | FormKind::Regex(s)
            | FormKind::Symbol(s)
            | FormKind::Keyword(s)
            | FormKind::AutoKeyword(s) => s.capacity(),

            // Vec<Form> — Vec overhead + recursive children.
            FormKind::List(v)
            | FormKind::Vector(v)
            | FormKind::Map(v)
            | FormKind::Set(v)
            | FormKind::AnonFn(v) => vec_heap_size(v),

            // Box<Form> — one Form on heap.
            FormKind::Quote(f)
            | FormKind::SyntaxQuote(f)
            | FormKind::Unquote(f)
            | FormKind::UnquoteSplice(f)
            | FormKind::Deref(f)
            | FormKind::Var(f) => mem::size_of::<Form>() + f.heap_size(),

            // Two Box<Form>.
            FormKind::Meta(a, b) => mem::size_of::<Form>() * 2 + a.heap_size() + b.heap_size(),

            // String + Box<Form>.
            FormKind::TaggedLiteral(s, f) => s.capacity() + mem::size_of::<Form>() + f.heap_size(),

            FormKind::ReaderCond { clauses, .. } => vec_heap_size(clauses),
        }
    }
}

impl PartialEq for Form {
    fn eq(&self, other: &Self) -> bool {
        self.kind == other.kind
    }
}

/// The payload of a `Form` node.
#[derive(Debug, Clone, PartialEq)]
pub enum FormKind {
    // ── Atoms ─────────────────────────────────────────────────────────────────
    Nil,
    Bool(bool),
    Int(i64),
    BigInt(String),
    Float(f64), // NaN != NaN per IEEE 754 — acceptable for AST equality
    BigDecimal(String),
    Ratio(String),
    Char(char),
    Str(String),
    Regex(String),
    /// `##Inf` → `INFINITY`, `##-Inf` → `NEG_INFINITY`, `##NaN` → `NAN`
    Symbolic(f64),

    // ── Identifiers ───────────────────────────────────────────────────────────
    Symbol(String),
    Keyword(String),
    AutoKeyword(String),

    // ── Collections ───────────────────────────────────────────────────────────
    List(Vec<Form>),
    Vector(Vec<Form>),
    /// Flat key/value pairs; length is always even.
    Map(Vec<Form>),
    Set(Vec<Form>),

    // ── Wrapping reader macros ────────────────────────────────────────────────
    Quote(Box<Form>),
    SyntaxQuote(Box<Form>),
    Unquote(Box<Form>),
    UnquoteSplice(Box<Form>),
    Deref(Box<Form>),
    /// `#'symbol`
    Var(Box<Form>),
    /// `^meta-form annotated-form` — raw meta form kept as-is; evaluator
    /// expands shorthand (`:kw` → `{:kw true}`, `Sym` → `{:tag Sym}`).
    Meta(Box<Form>, Box<Form>),

    // ── Dispatch forms ────────────────────────────────────────────────────────
    /// `#(…)` anonymous function literal
    AnonFn(Vec<Form>),
    /// `#tag form` tagged literal
    TaggedLiteral(String, Box<Form>),

    // ── Reader conditionals ───────────────────────────────────────────────────
    /// All branches are kept; the evaluator filters by `:rust`.
    /// `clauses` is flat: `[keyword, form, keyword, form, …]`.
    ReaderCond {
        splicing: bool,
        clauses: Vec<Form>,
    },
}

fn vec_heap_size(forms: &[Form]) -> usize {
    mem::size_of_val(forms) + forms.iter().map(|f| f.heap_size()).sum::<usize>()
}
