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

    /// True when this form, used as a variadic `&` rest pattern, selects
    /// Clojure's keyword-argument convention.
    ///
    /// A map-shaped rest pattern (`[& {:keys [a b]}]`) does not destructure the
    /// rest *sequence*; the trailing arguments are first normalized into a map
    /// (`Value::from_kwargs_rest`) and the pattern is applied to that.  Every
    /// execution tier has to make this call the same way — the tree-walker in
    /// `bind_fn_params`, and both lowerers on their way to `KnownFn::KwargsMap`
    /// — so they share this predicate rather than three copies of the match.
    /// Issue #368 was those copies disagreeing.
    pub fn is_kwargs_rest_pattern(&self) -> bool {
        matches!(self.kind, FormKind::Map(_))
    }

    /// The annotated form with every `^meta` wrapper removed.
    ///
    /// Returns `self` when the form carries no metadata. Stacked metadata
    /// (`^:a ^:b x`) is peeled down to the innermost form.
    pub fn unmeta(&self) -> &Form {
        let mut form = self;
        while let FormKind::Meta(_, inner) = &form.kind {
            form = inner;
        }
        form
    }

    /// True when an evaluated-position `^meta` annotation on this form becomes
    /// *runtime* metadata on the value it produces.
    ///
    /// Only a form that constructs an `IObj` qualifies — a collection literal
    /// or a function. Every other form (a call, a symbol, `quote`, `if`, `do`)
    /// takes the annotation as a compile-time hint and evaluates to an
    /// unannotated value, so `(meta ^{:a 1} (list 1))` and `(meta ^{:a 1} x)`
    /// are both `nil`.
    ///
    /// Every execution tier consults this one predicate: the tree-walker in
    /// `interp::eval`, and IR lowering in `lower::anf` for the JIT and AOT
    /// paths. A tier that disagreed would make `meta` depend on how hot the
    /// code got.
    ///
    /// True when the value this form denotes *as data* can carry metadata.
    ///
    /// Inside `quote` every form is a literal, so whether an annotation lands
    /// is a question about the form and needs no runtime test: `'^{:a 1} [1]`
    /// carries it, `'^{:a 1} 42` cannot. Mirrors `supports_meta` in
    /// `cljrs-runtime` over the values `form_to_value` produces — a reader
    /// macro (`'x`, `@x`, `#'x`, `` `x ``) denotes a list, and `#(…)` denotes
    /// the `fn*` list it expands to.
    pub fn quoted_value_supports_meta(&self) -> bool {
        match &self.kind {
            FormKind::List(_)
            | FormKind::Vector(_)
            | FormKind::Map(_)
            | FormKind::Set(_)
            | FormKind::Symbol(_)
            | FormKind::AutoSymbol(_)
            | FormKind::AnonFn(_)
            | FormKind::Quote(_)
            | FormKind::SyntaxQuote(_)
            | FormKind::Unquote(_)
            | FormKind::UnquoteSplice(_)
            | FormKind::Deref(_)
            | FormKind::Var(_) => true,
            FormKind::Meta(_, inner) => inner.quoted_value_supports_meta(),
            _ => false,
        }
    }

    /// Inside `quote` the rule does not apply: there the annotation is data and
    /// lands on any value that can carry it.
    pub fn takes_runtime_meta(&self) -> bool {
        match &self.kind {
            FormKind::Vector(_) | FormKind::Map(_) | FormKind::Set(_) | FormKind::AnonFn(_) => true,
            // A list is a call, except when it *is* a function form.
            FormKind::List(parts) => matches!(
                parts.first().map(|f| &f.kind),
                Some(FormKind::Symbol(s)) if s == "fn" || s == "fn*"
            ),
            // Metadata stacks: `^:a ^:b [1]` annotates the vector twice.
            FormKind::Meta(_, inner) => inner.takes_runtime_meta(),
            _ => false,
        }
    }

    /// The `^meta` forms attached to this form, outermost first, together with
    /// the annotated form itself.
    pub fn peel_meta(&self) -> (Vec<&Form>, &Form) {
        let mut metas = Vec::new();
        let mut form = self;
        while let FormKind::Meta(meta, inner) = &form.kind {
            metas.push(meta.as_ref());
            form = inner;
        }
        (metas, form)
    }

    // ── Structural views ──────────────────────────────────────────────────────
    //
    // Every accessor below reports the shape of [`Form::unmeta`], so an
    // annotated form has the same structural shape as the form it annotates.

    /// The symbol name, if this form is a symbol.
    pub fn as_symbol(&self) -> Option<&str> {
        match &self.unmeta().kind {
            FormKind::Symbol(s) => Some(s),
            _ => None,
        }
    }

    /// The keyword name (without the leading `:`), if this form is a keyword.
    pub fn as_keyword(&self) -> Option<&str> {
        match &self.unmeta().kind {
            FormKind::Keyword(k) => Some(k),
            _ => None,
        }
    }

    /// The string contents, if this form is a string literal.
    pub fn as_string(&self) -> Option<&str> {
        match &self.unmeta().kind {
            FormKind::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The elements, if this form is a list.
    pub fn as_list(&self) -> Option<&[Form]> {
        match &self.unmeta().kind {
            FormKind::List(v) => Some(v),
            _ => None,
        }
    }

    /// The elements, if this form is a vector.
    pub fn as_vector(&self) -> Option<&[Form]> {
        match &self.unmeta().kind {
            FormKind::Vector(v) => Some(v),
            _ => None,
        }
    }

    /// The flat key/value pairs, if this form is a map literal.
    pub fn as_map(&self) -> Option<&[Form]> {
        match &self.unmeta().kind {
            FormKind::Map(v) => Some(v),
            _ => None,
        }
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
            | FormKind::AutoKeyword(s)
            | FormKind::AutoSymbol(s) => s.capacity(),

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
    /// `::kw` / `::alias/kw` - the namespace is resolved against the reading
    /// namespace by the evaluator, not by the reader.
    AutoKeyword(String),
    /// A symbol whose namespace is auto-resolved the same way. There is no
    /// surface syntax for one; the reader produces it for a bare symbol key in
    /// an auto-resolved namespaced map (`#::{a 1}`, `#::alias{a 1}`).
    AutoSymbol(String),

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
