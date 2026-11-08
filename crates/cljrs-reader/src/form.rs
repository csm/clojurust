use std::mem;
use std::sync::{Arc, Mutex, OnceLock};

use cljrs_types::span::Span;

/// A parsed Clojure form with its source location.
///
/// `PartialEq` ignores spans (and the expansion cache) so test assertions can
/// compare forms without constructing exact span values.
#[derive(Debug)]
pub struct Form {
    pub kind: FormKind,
    pub span: Span,
    /// Memoized macro expansion of this form when it is a macro call site.
    ///
    /// Owned by the form so it lives exactly as long as the call site does:
    /// a function body or `loop` that evaluates the same form repeatedly
    /// expands its macros once, while a freshly read form (a new REPL input)
    /// always starts empty. See [`ExpansionCache`].
    pub expansion: ExpansionCache,
}

impl Clone for Form {
    fn clone(&self) -> Self {
        // Only a list can be a macro call site. Its clones share one cache
        // slot: `(fn ...)` copies its body into every closure it creates, and
        // an expansion made while running one closure must serve the next.
        let expansion = if matches!(self.kind, FormKind::List(_)) {
            self.expansion.share()
        } else {
            ExpansionCache::default()
        };
        Self {
            kind: self.kind.clone(),
            span: self.span.clone(),
            expansion,
        }
    }
}

impl Form {
    pub fn new(kind: FormKind, span: Span) -> Self {
        Self {
            kind,
            span,
            expansion: ExpansionCache::default(),
        }
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

    /// True when an evaluated-position `^meta` annotation on this form becomes
    /// *runtime* metadata on the value it produces.
    ///
    /// Only a form that constructs an `IObj` qualifies — a collection literal
    /// (the empty list `()` included) or a function. Every other form (a call,
    /// a symbol, `quote`, `if`, `do`) takes the annotation as a compile-time
    /// hint and evaluates to an unannotated value, so `(meta ^{:a 1} (list 1))`
    /// and `(meta ^{:a 1} x)` are both `nil`.
    ///
    /// Every execution tier consults this one predicate: the tree-walker in
    /// `interp::eval`, and IR lowering in `lower::anf` for the JIT and AOT
    /// paths. A tier that disagreed would make `meta` depend on how hot the
    /// code got.
    ///
    /// Inside `quote` the rule does not apply: there the annotation is data and
    /// lands on any value that can carry it (see
    /// [`Form::quoted_value_supports_meta`]).
    pub fn takes_runtime_meta(&self) -> bool {
        match &self.kind {
            FormKind::Vector(_) | FormKind::Map(_) | FormKind::Set(_) | FormKind::AnonFn(_) => true,
            // `()` has no head, so it is not a call: it is the empty-list
            // literal, and attaches exactly as `[]`, `{}` and `#{}` do.
            FormKind::List(parts) if parts.is_empty() => true,
            // Any other list is a call, except when it *is* a function form.
            FormKind::List(_) => self.is_fn_form(),
            // Metadata stacks: `^:a ^:b [1]` annotates the vector twice.
            FormKind::Meta(_, inner) => inner.takes_runtime_meta(),
            _ => false,
        }
    }

    /// True when this form is a function form: a list headed by `fn` or `fn*`.
    pub fn is_fn_form(&self) -> bool {
        match &self.kind {
            FormKind::List(parts) => matches!(
                parts.first().map(|f| &f.kind),
                Some(FormKind::Symbol(s)) if s == "fn" || s == "fn*"
            ),
            _ => false,
        }
    }

    /// True when this form, used as a `^meta` annotation or a `defn` attr-map,
    /// requests `:async`.
    ///
    /// Handles the keyword shorthand `^:async` and an explicit map such as
    /// `^{:async true}` or `{:async true}`; a `false` or `nil` value does not
    /// request it.
    pub fn requests_async(&self) -> bool {
        match &self.kind {
            FormKind::Keyword(k) => k == "async",
            FormKind::Map(entries) => entries.chunks(2).any(|kv| {
                matches!(&kv[0].kind, FormKind::Keyword(k) if k == "async")
                    && !matches!(
                        kv.get(1).map(|f| &f.kind),
                        None | Some(FormKind::Bool(false)) | Some(FormKind::Nil)
                    )
            }),
            _ => false,
        }
    }

    /// True when this form is an anonymous function that asks to be `^:async`,
    /// in any spelling: an annotation on the whole form,
    /// `^:async (fn [..] ..)`, or one accepted by [`Form::fn_args_request_async`]
    /// on its arguments. Stacked annotations are searched in full.
    ///
    /// Only the tree-walker can build an async closure; IR lowering refuses a
    /// body containing one of these, so every tier agrees on what calling it
    /// returns.
    pub fn is_async_fn_form(&self) -> bool {
        let (metas, inner) = self.peel_meta();
        if !inner.is_fn_form() {
            return false;
        }
        if metas.iter().any(|m| m.requests_async()) {
            return true;
        }
        let FormKind::List(parts) = &inner.kind else {
            return false;
        };
        Form::fn_args_request_async(&parts[1..])
    }

    /// True when the arguments of an `fn`/`fn*` form (everything after the
    /// head) request `:async` through metadata on the first argument,
    /// `(fn ^:async [..] ..)` / `(fn ^:async name [..] ..)`, or on a single-arity
    /// params vector that follows a name, `(fn name ^:async [..] ..)`. The last
    /// is also what `(defn name ^:async [..] ..)` becomes once `defn` rebuilds
    /// its `fn` arguments. Stacked annotations are searched in full.
    pub fn fn_args_request_async(args: &[Form]) -> bool {
        let requests = |f: &Form| f.peel_meta().0.iter().any(|m| m.requests_async());
        let Some(first) = args.first() else {
            return false;
        };
        if requests(first) {
            return true;
        }
        first.as_symbol().is_some() && args.get(1).is_some_and(requests)
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

// ── Macro-expansion cache ─────────────────────────────────────────────────────

/// One memoized macro expansion: valid while the call site resolves to the
/// same macro (`macro_id`) and is evaluated in the same namespace (`ns`).
#[derive(Debug)]
pub struct CachedExpansion {
    /// Identity of the macro function that produced `form` (its first arity's
    /// process-unique id). Redefining the macro mints a new id, so a stale
    /// expansion is never reused.
    pub macro_id: u64,
    /// Namespace the call site was expanded in; `::kw` and syntax-quote
    /// resolution depend on it.
    pub ns: Arc<str>,
    /// The expansion. Held in an `Arc` so its own nested call sites keep a
    /// stable address and therefore their own caches across evaluations.
    pub form: Arc<Form>,
}

/// Per-call-site slot holding the last macro expansion of a [`Form`].
///
/// The tree-walking evaluator otherwise re-expands every macro call on every
/// evaluation (converting the forms to values, running the macro, converting
/// the result back), which dominated the cost of e.g. a `when` inside a hot
/// `loop`. Clojure itself expands each call site once, at compile time, so
/// reusing the expansion matches its semantics.
///
/// Clones of a list form share the slot (see `Form::clone`), so a function
/// body copied into many closures expands each call site once in total. The
/// shared cell is allocated lazily, on first store or first clone, so a form
/// that is parsed and evaluated once pays nothing beyond an empty word.
///
/// A form whose `kind` is mutated in place after cloning must not be
/// evaluated as a macro call afterwards, since it would share its source's
/// expansion; no evaluator path does this (rewrites build fresh forms).
#[derive(Default)]
pub struct ExpansionCache(OnceLock<Arc<ExpansionSlot>>);

type ExpansionSlot = Mutex<Option<Arc<CachedExpansion>>>;

impl ExpansionCache {
    /// The cached expansion, if one was made by `macro_id` in `ns`.
    pub fn get(&self, macro_id: u64, ns: &str) -> Option<Arc<Form>> {
        let slot = self.0.get()?.lock().unwrap_or_else(|e| e.into_inner());
        match slot.as_ref() {
            Some(c) if c.macro_id == macro_id && c.ns.as_ref() == ns => Some(c.form.clone()),
            _ => None,
        }
    }

    /// Record `form` as the expansion made by `macro_id` in `ns`, replacing
    /// any previous entry. Returns the stored `Arc` so the caller evaluates
    /// the very form the cache holds.
    pub fn store(&self, macro_id: u64, ns: Arc<str>, form: Form) -> Arc<Form> {
        let form = Arc::new(form);
        let entry = Arc::new(CachedExpansion {
            macro_id,
            ns,
            form: form.clone(),
        });
        *self.slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(entry);
        form
    }

    /// Drop any cached expansion (for every form sharing this slot).
    pub fn clear(&self) {
        if let Some(slot) = self.0.get() {
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
    }

    /// A handle to the same slot, for a clone of the owning form.
    fn share(&self) -> Self {
        Self(OnceLock::from(self.slot().clone()))
    }

    fn slot(&self) -> &Arc<ExpansionSlot> {
        self.0.get_or_init(Default::default)
    }
}

impl std::fmt::Debug for ExpansionCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ExpansionCache")
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
    /// All branches are kept; the runtime picks one with
    /// `cljrs_runtime::builtins::form::select_reader_cond` (feature keys
    /// `:cljrs`/legacy `:rust`, then `:default`, first match in clause order).
    /// Never compare the feature keyword by hand.
    /// `clauses` is flat: `[keyword, form, keyword, form, …]`.
    ReaderCond {
        splicing: bool,
        clauses: Vec<Form>,
    },
}

fn vec_heap_size(forms: &[Form]) -> usize {
    mem::size_of_val(forms) + forms.iter().map(|f| f.heap_size()).sum::<usize>()
}

#[cfg(test)]
mod expansion_cache_tests {
    use super::*;

    fn span() -> Span {
        Span::new(Arc::new("<t>".to_string()), 0, 0, 1, 1)
    }

    fn list() -> Form {
        Form::new(
            FormKind::List(vec![Form::new(FormKind::Symbol("m".into()), span())]),
            span(),
        )
    }

    #[test]
    fn empty_until_stored() {
        assert!(list().expansion.get(1, "user").is_none());
    }

    #[test]
    fn hit_requires_same_macro_and_namespace() {
        let site = list();
        site.expansion
            .store(7, Arc::from("user"), Form::new(FormKind::Int(1), span()));
        assert!(site.expansion.get(7, "user").is_some());
        assert!(site.expansion.get(8, "user").is_none());
        assert!(site.expansion.get(7, "other").is_none());
    }

    #[test]
    fn clones_of_a_list_share_the_slot() {
        let site = list();
        let copy = site.clone();
        copy.expansion
            .store(7, Arc::from("user"), Form::new(FormKind::Int(1), span()));
        assert!(site.expansion.get(7, "user").is_some());
        site.expansion.clear();
        assert!(copy.expansion.get(7, "user").is_none());
    }

    #[test]
    fn non_list_clones_do_not_allocate_a_slot() {
        let sym = Form::new(FormKind::Symbol("x".into()), span());
        let copy = sym.clone();
        assert!(sym.expansion.0.get().is_none());
        assert!(copy.expansion.0.get().is_none());
    }
}
