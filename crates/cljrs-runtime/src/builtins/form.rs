// ── form_to_value ─────────────────────────────────────────────────────────────

use crate::env::error::{EvalError, EvalResult};
use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind};
use cljrs_value::regex::Pattern;
use cljrs_value::value::SetValue;
use cljrs_value::{
    Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value,
};

// ── anon fn expansion ─────────────────────────────────────────────────────────

/// Expand `#(...)` to `(fn* [p__1 p__2 ... & rest__] ...)`.
pub fn expand_anon_fn(body: &[Form], span: cljrs_types::span::Span) -> Form {
    let mut max_pos: usize = 0;
    let mut has_rest = false;
    find_pct_refs(body, &mut max_pos, &mut has_rest);

    let s = &span;
    let mut params: Vec<Form> = (1..=max_pos)
        .map(|i| Form::new(FormKind::Symbol(format!("p__{i}")), s.clone()))
        .collect();
    if has_rest {
        params.push(Form::new(FormKind::Symbol("&".into()), s.clone()));
        params.push(Form::new(FormKind::Symbol("rest__".into()), s.clone()));
    }

    let new_body = rewrite_pct_refs(body, s.clone());

    // Wrap the rewritten body forms back into a single call expression.
    // #(f a b) → (fn* [params] (f a b)), not (fn* [params] f a b).
    let body_expr = Form::new(FormKind::List(new_body), s.clone());

    Form::new(
        FormKind::List(vec![
            Form::new(FormKind::Symbol("fn*".into()), s.clone()),
            Form::new(FormKind::Vector(params), s.clone()),
            body_expr,
        ]),
        span,
    )
}

fn find_pct_refs(forms: &[Form], max_pos: &mut usize, has_rest: &mut bool) {
    for form in forms {
        find_pct_refs_form(form, max_pos, has_rest);
    }
}

fn find_pct_refs_form(form: &Form, max_pos: &mut usize, has_rest: &mut bool) {
    match &form.kind {
        FormKind::Symbol(s) if (s == "%" || s == "%1") && *max_pos < 1 => {
            *max_pos = 1;
        }
        FormKind::Symbol(s) if s == "%&" => {
            *has_rest = true;
        }
        FormKind::Symbol(s) if s.starts_with('%') => {
            if let Ok(n) = s[1..].parse::<usize>()
                && n > *max_pos
            {
                *max_pos = n;
            }
        }
        FormKind::List(c) | FormKind::Vector(c) | FormKind::Set(c) | FormKind::Map(c) => {
            find_pct_refs(c, max_pos, has_rest);
        }
        // Reader-macro sugar (`@%`, `#'%`, `'%`, `` `% ``, `~%`, `~@%`, `#tag %`)
        // wraps a single inner form; it must be scanned the same as any
        // other nested form so `%` refs under sugar aren't missed.
        FormKind::Quote(inner)
        | FormKind::SyntaxQuote(inner)
        | FormKind::Unquote(inner)
        | FormKind::UnquoteSplice(inner)
        | FormKind::Deref(inner)
        | FormKind::Var(inner)
        | FormKind::TaggedLiteral(_, inner) => {
            find_pct_refs_form(inner, max_pos, has_rest);
        }
        FormKind::Meta(meta, inner) => {
            find_pct_refs_form(meta, max_pos, has_rest);
            find_pct_refs_form(inner, max_pos, has_rest);
        }
        FormKind::ReaderCond { clauses, .. } => {
            find_pct_refs(clauses, max_pos, has_rest);
        }
        _ => {}
    }
}

fn rewrite_pct_refs(forms: &[Form], span: cljrs_types::span::Span) -> Vec<Form> {
    forms
        .iter()
        .map(|f| rewrite_pct_form(f, span.clone()))
        .collect()
}

fn rewrite_pct_form(form: &Form, span: cljrs_types::span::Span) -> Form {
    match &form.kind {
        FormKind::Symbol(s) if s == "%" || s == "%1" => {
            Form::new(FormKind::Symbol("p__1".into()), span)
        }
        FormKind::Symbol(s) if s == "%&" => Form::new(FormKind::Symbol("rest__".into()), span),
        FormKind::Symbol(s) if s.starts_with('%') => {
            if let Ok(n) = s[1..].parse::<usize>() {
                Form::new(FormKind::Symbol(format!("p__{n}")), span)
            } else {
                form.clone()
            }
        }
        FormKind::List(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::List(rewritten), span)
        }
        FormKind::Vector(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Vector(rewritten), span)
        }
        FormKind::Set(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Set(rewritten), span)
        }
        FormKind::Map(c) => {
            let rewritten = rewrite_pct_refs(c, span.clone());
            Form::new(FormKind::Map(rewritten), span)
        }
        FormKind::Quote(inner) => Form::new(
            FormKind::Quote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::SyntaxQuote(inner) => Form::new(
            FormKind::SyntaxQuote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Unquote(inner) => Form::new(
            FormKind::Unquote(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::UnquoteSplice(inner) => Form::new(
            FormKind::UnquoteSplice(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Deref(inner) => Form::new(
            FormKind::Deref(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Var(inner) => Form::new(
            FormKind::Var(Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::TaggedLiteral(tag, inner) => Form::new(
            FormKind::TaggedLiteral(tag.clone(), Box::new(rewrite_pct_form(inner, span.clone()))),
            span,
        ),
        FormKind::Meta(meta, inner) => Form::new(
            FormKind::Meta(
                Box::new(rewrite_pct_form(meta, span.clone())),
                Box::new(rewrite_pct_form(inner, span.clone())),
            ),
            span,
        ),
        FormKind::ReaderCond { splicing, clauses } => Form::new(
            FormKind::ReaderCond {
                splicing: *splicing,
                clauses: rewrite_pct_refs(clauses, span.clone()),
            },
            span,
        ),
        _ => form.clone(),
    }
}

// ── auto-resolved identifiers ─────────────────────────────────────────────────

/// Replace every `AutoKeyword` / `AutoSymbol` in a form tree with its
/// fully-qualified `Keyword` / `Symbol`, resolving an `alias/name` prefix
/// through `env`'s namespace alias table.
///
/// Every boundary that turns a form into data or into IR - `quote`, the `'`
/// sugar, and `lower_arity` - must call this first, so no tier below sees an
/// unresolved identifier. Errors when an alias is not registered in `env`.
pub fn resolve_auto_forms(form: &Form, env: &crate::env::env::Env) -> EvalResult<Form> {
    let resolve_seq = |items: &Vec<Form>| -> EvalResult<Vec<Form>> {
        items.iter().map(|f| resolve_auto_forms(f, env)).collect()
    };
    let resolve_box =
        |inner: &Form| -> EvalResult<Box<Form>> { Ok(Box::new(resolve_auto_forms(inner, env)?)) };
    let qualify = |name: &str| -> EvalResult<String> {
        env.globals
            .resolve_auto_keyword(&env.current_ns, name)
            .map_err(EvalError::Runtime)
    };

    let kind = match &form.kind {
        FormKind::AutoKeyword(s) => FormKind::Keyword(qualify(s)?),
        FormKind::AutoSymbol(s) => FormKind::Symbol(qualify(s)?),

        FormKind::List(items) => FormKind::List(resolve_seq(items)?),
        FormKind::Vector(items) => FormKind::Vector(resolve_seq(items)?),
        FormKind::Map(items) => FormKind::Map(resolve_seq(items)?),
        FormKind::Set(items) => FormKind::Set(resolve_seq(items)?),
        FormKind::AnonFn(items) => FormKind::AnonFn(resolve_seq(items)?),
        FormKind::ReaderCond { splicing, clauses } => FormKind::ReaderCond {
            splicing: *splicing,
            clauses: resolve_seq(clauses)?,
        },

        FormKind::Quote(inner) => FormKind::Quote(resolve_box(inner)?),
        FormKind::SyntaxQuote(inner) => FormKind::SyntaxQuote(resolve_box(inner)?),
        FormKind::Unquote(inner) => FormKind::Unquote(resolve_box(inner)?),
        FormKind::UnquoteSplice(inner) => FormKind::UnquoteSplice(resolve_box(inner)?),
        FormKind::Deref(inner) => FormKind::Deref(resolve_box(inner)?),
        FormKind::Var(inner) => FormKind::Var(resolve_box(inner)?),
        FormKind::TaggedLiteral(tag, inner) => {
            FormKind::TaggedLiteral(tag.clone(), resolve_box(inner)?)
        }
        FormKind::Meta(meta, inner) => FormKind::Meta(resolve_box(meta)?, resolve_box(inner)?),

        _ => return Ok(form.clone()),
    };
    Ok(Form::new(kind, form.span.clone()))
}

/// True for values that carry metadata (the JVM's `IObj`).
///
/// Scalars silently drop an annotation rather than growing a `WithMeta`
/// wrapper no other tier expects.
pub fn supports_meta(value: &Value) -> bool {
    matches!(
        value.unwrap_meta(),
        Value::List(_)
            | Value::Vector(_)
            | Value::Map(_)
            | Value::Set(_)
            | Value::Queue(_)
            | Value::Symbol(_)
            | Value::Cons(_)
            | Value::LazySeq(_)
            | Value::Fn(_)
            | Value::Macro(_)
            | Value::BoundFn(_)
            | Value::NativeFunction(_)
            | Value::TypeInstance(_)
    )
}

/// The map a `^meta` annotation denotes, expanding the reader shorthands
/// (`^:kw` → `{:kw true}`, `^Sym` / `^"Str"` → `{:tag …}`).
///
/// `general` resolves the non-shorthand case — a map form, or an expression
/// yielding one — and is the *only* difference between the two positions an
/// annotation can appear in: inside `quote` it is [`form_to_value`], so
/// `'^{:x (+ 1 2)} [1]` keeps the unevaluated list; outside it evaluates, so
/// `^{:x (+ 1 2)} [1]` carries `{:x 3}`.
///
/// Both positions share this one table so they cannot drift apart.
pub fn expand_meta_annotation(
    meta: &Form,
    general: &mut dyn FnMut(&Form) -> EvalResult<Value>,
) -> EvalResult<Value> {
    let entry = |k: Value, v: Value| Value::Map(MapValue::empty().assoc(k, v));
    let expanded = match &meta.kind {
        // `^:kw` → `{:kw true}`. An auto-keyword resolves against the reading
        // namespace, which only `general` knows how to do in either position.
        FormKind::Keyword(_) => entry(form_to_value(meta)?, Value::Bool(true)),
        FormKind::AutoKeyword(_) => entry(general(meta)?, Value::Bool(true)),
        // `^Sym` → `{:tag Sym}`, `^"Str"` → `{:tag "Str"}`. The tag is the
        // symbol/string itself, never the value the symbol resolves to.
        FormKind::Symbol(_) | FormKind::Str(_) => {
            entry(Value::keyword(Keyword::parse("tag")), form_to_value(meta)?)
        }
        // An annotation the macro expander rebuilt out of a value is already
        // data: `value_to_form` quotes it so that neither position analyses it
        // a second time.
        FormKind::Quote(inner) => form_to_value(inner)?,
        _ => general(meta)?,
    };
    // `^42 [1]` is a malformed annotation, not metadata `42` — the JVM reader
    // rejects it outright rather than attaching a non-map.
    match expanded {
        v @ (Value::Map(_) | Value::Nil) => Ok(v),
        other => Err(EvalError::Runtime(format!(
            "Metadata must be Symbol, Keyword, String or Map, got {}",
            other.type_name()
        ))),
    }
}

/// Assoc every entry of `overlay` onto `base`, as the reader does for stacked
/// metadata (`^:a ^:b x`): both survive, the outer annotation wins a clash.
pub fn merge_meta_values(base: &Value, overlay: &Value) -> Value {
    match (base, overlay) {
        (Value::Map(base), Value::Map(overlay)) => {
            let mut merged = base.clone();
            overlay.for_each(|k, v| merged = merged.assoc(k.clone(), v.clone()));
            Value::Map(merged)
        }
        _ => overlay.clone(),
    }
}

/// Attach an expanded `^meta` annotation to the value it annotates.
///
/// A value that cannot carry metadata keeps none, and an annotation stacked on
/// one already present merges with it, the outer one winning a clash. Shared by
/// the quoted and evaluated positions so they attach identically.
pub fn attach_meta(value: Value, annotation: Value) -> Value {
    if matches!(annotation, Value::Nil) || !supports_meta(&value) {
        return value;
    }
    let merged = match value.get_meta() {
        Some(existing) => merge_meta_values(existing, &annotation),
        None => annotation,
    };
    value.with_meta(merged)
}

/// Convert a `Form` to its literal `Value` without evaluating.
/// Used by `quote` and macro expansion.
///
/// Every sibling sequence is run through [`expand_reader_conds`] first, so a
/// `#?@` splice contributes its elements to the enclosing collection rather
/// than reaching the leaf arm.
///
/// Errors when the form cannot denote a value: a map literal whose expansion
/// has odd length, or a `#?@` splice in a position that has no sibling
/// sequence to splice into.
/// The list value `form_to_value` builds for a `FormKind::List`, over elements
/// that are already converted.
///
/// `macroexpand_1` converts a call's parts itself so that `&form` and the
/// macro's arguments can share one conversion; this keeps how a list form
/// becomes a value in one place rather than two.
pub fn values_to_list(items: impl IntoIterator<Item = Value>) -> Value {
    Value::List(GcPtr::new(PersistentList::from_iter(items)))
}

pub fn form_to_value(form: &Form) -> EvalResult<Value> {
    let value = match &form.kind {
        FormKind::Nil => Value::Nil,
        FormKind::Bool(b) => Value::Bool(*b),
        FormKind::Int(n) => Value::Long(*n),
        FormKind::Float(f) => Value::Double(*f),
        FormKind::Symbolic(f) => Value::Double(*f),
        FormKind::Str(s) => Value::string(s.clone()),
        FormKind::Char(c) => Value::Char(*c),
        FormKind::BigInt(s) => crate::builtins::parse_bigint(s).unwrap_or(Value::Nil),
        FormKind::BigDecimal(s) => crate::builtins::parse_bigdecimal(s).unwrap_or(Value::Nil),
        FormKind::Ratio(s) => crate::builtins::parse_ratio(s).unwrap_or(Value::Nil),

        FormKind::Symbol(s) => Value::symbol(Symbol::parse(s)),
        FormKind::Keyword(s) => Value::keyword(Keyword::parse(s)),
        FormKind::AutoKeyword(s) => Value::keyword(Keyword::simple(s.as_str())),
        FormKind::AutoSymbol(s) => Value::symbol(Symbol::parse(s)),
        FormKind::Regex(s) => match Pattern::new(s.as_str()) {
            Ok(pattern) => Value::Pattern(GcPtr::new(pattern)),
            Err(_) => Value::Nil, // should already have been caught
        },

        FormKind::List(forms) => {
            let items = forms_to_values(&expand_reader_conds_cow(forms))?;
            values_to_list(items)
        }
        FormKind::Vector(forms) => {
            let items = forms_to_values(&expand_reader_conds_cow(forms))?;
            Value::Vector(GcPtr::new(PersistentVector::from_iter(items)))
        }
        FormKind::Map(forms) => {
            let forms = expand_pairs(forms).map_err(|_| map_literal_arity_error())?;
            let mut m = MapValue::empty();
            for [k, v] in forms.as_chunks::<2>().0 {
                m = m.assoc(form_to_value(k)?, form_to_value(v)?);
            }
            Value::Map(m)
        }
        FormKind::Set(forms) => {
            let mut s = PersistentHashSet::empty();
            for f in expand_reader_conds_cow(forms).iter() {
                s = s.conj(form_to_value(f)?);
            }
            Value::Set(SetValue::Hash(GcPtr::new(s)))
        }

        // `'x` → the form x as a data value.
        FormKind::Quote(inner) => reader_form_value("quote", inner)?,
        FormKind::SyntaxQuote(inner) => reader_form_value("syntax-quote", inner)?,
        FormKind::Unquote(inner) => reader_form_value("unquote", inner)?,
        FormKind::UnquoteSplice(inner) => reader_form_value("unquote-splicing", inner)?,
        FormKind::Deref(inner) => reader_form_value("deref", inner)?,
        FormKind::Var(inner) => reader_form_value("var", inner)?,
        FormKind::Meta(meta, inner) => {
            let value = form_to_value(inner)?;
            // Inside `quote` the annotation is data: nothing in it is evaluated.
            let m = expand_meta_annotation(meta, &mut form_to_value)?;
            attach_meta(value, m)
        }
        FormKind::AnonFn(body) => {
            // Expand #(...) to (fn* [...] ...) so it round-trips correctly through quote.
            let expanded = expand_anon_fn(body, form.span.clone());
            form_to_value(&expanded)?
        }
        FormKind::TaggedLiteral(tag, inner) => match tag.as_str() {
            "uuid" => {
                if let FormKind::Str(s) = &inner.kind
                    && let Ok(u) = uuid::Uuid::parse_str(s)
                {
                    Value::uuid(u.as_u128())
                } else {
                    form_to_value(inner)?
                }
            }
            _ => form_to_value(inner)?,
        },
        FormKind::ReaderCond {
            splicing: false,
            clauses,
        } => match select_reader_cond(clauses) {
            Some(selected) => form_to_value(selected)?,
            None => Value::Nil,
        },
        // Splices are consumed by the enclosing sibling sequence; reaching a
        // leaf means there was none.
        FormKind::ReaderCond { splicing: true, .. } => {
            return Err(EvalError::Runtime(
                "splicing reader conditional not in a sequence context".into(),
            ));
        }
    };
    Ok(value)
}

fn forms_to_values(forms: &[Form]) -> EvalResult<Vec<Value>> {
    forms.iter().map(form_to_value).collect()
}

/// `(<sym> <inner>)` — the list encoding of a reader-sugar form as data.
fn reader_form_value(sym: &str, inner: &Form) -> EvalResult<Value> {
    Ok(Value::List(GcPtr::new(PersistentList::from_iter([
        Value::symbol(Symbol::simple(sym)),
        form_to_value(inner)?,
    ]))))
}

fn map_literal_arity_error() -> EvalError {
    EvalError::Runtime("map literal must have an even number of forms".into())
}

/// The reader-conditional feature keys this runtime answers to. `:rust` is
/// the historical key; `:cljrs` is the dialect's own name, the one portable
/// libraries write alongside `:clj`, `:cljs`, `:cljr` and `:lpy` (malli keys
/// its branches `:cljrs`). Both select the same platform branch.
pub fn is_platform_feature(k: &str) -> bool {
    k == "rust" || k == "cljrs"
}

/// Resolve a `#?(...)` reader conditional to the selected branch form, or
/// `None` if no platform (`:rust` / `:cljrs`) or `:default` clause is present.
///
/// Clauses are tried in source order and the first match wins, as in
/// Clojure: `:default` always matches, so `#?(:default :a :cljrs :b)` selects
/// `:a`. This is the single selection rule; every evaluator path calls it.
pub fn select_reader_cond(clauses: &[Form]) -> Option<&Form> {
    clauses
        .as_chunks::<2>()
        .0
        .iter()
        .find(|[feature, _]| {
            matches!(&feature.kind, FormKind::Keyword(k) if is_platform_feature(k) || k == "default")
        })
        .map(|[_, branch]| branch)
}

/// Expand reader conditionals in a flat slice of forms.
///
/// - Non-splicing `#?(...)`: replaced by the selected branch (or removed if none).
/// - Splicing `#?@(...)`: selected branch must be a vector/list; its elements
///   are inlined.  If no branch matches, the splice is removed (empty).
pub fn expand_reader_conds(forms: &[Form]) -> Vec<Form> {
    let mut out = Vec::with_capacity(forms.len());
    for form in forms {
        match &form.kind {
            FormKind::ReaderCond {
                splicing: true,
                clauses,
            } => {
                if let Some(selected) = select_reader_cond(clauses) {
                    match &selected.kind {
                        FormKind::Vector(elems) | FormKind::List(elems) => {
                            // Recursively expand any nested reader conditionals
                            // within the spliced elements.
                            let expanded_elems = expand_reader_conds(elems);
                            out.extend(expanded_elems);
                        }
                        // Non-sequence branch: inline it as a single element.
                        _ => out.push(selected.clone()),
                    }
                }
                // No matching branch → splice nothing (empty).
            }
            FormKind::ReaderCond {
                splicing: false,
                clauses,
            } => {
                if let Some(selected) = select_reader_cond(clauses) {
                    out.push(selected.clone());
                }
                // No matching branch → omit.
            }
            _ => out.push(form.clone()),
        }
    }
    out
}

/// Expand `#?`/`#?@` reader conditionals in a sibling-form slice, borrowing the
/// input unchanged when none are present.
///
/// Callers that validate element structure (e.g. map key/value parity) must do
/// so on the returned slice.
pub fn expand_reader_conds_cow(forms: &[Form]) -> std::borrow::Cow<'_, [Form]> {
    if forms
        .iter()
        .any(|f| matches!(f.kind, FormKind::ReaderCond { .. }))
    {
        std::borrow::Cow::Owned(expand_reader_conds(forms))
    } else {
        std::borrow::Cow::Borrowed(forms)
    }
}

/// A pair-structured sibling slice was left with an odd number of forms.
/// Carries that length; callers phrase the error for their own construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OddArity(pub usize);

/// Expand reader conditionals in a slice that must hold key/value or
/// binding/init *pairs*, then enforce even parity on the expansion.
///
/// Callers that chunk siblings by two - map literals, `let*`/`loop*`/`binding`
/// vectors - resolve them here before chunking.
pub fn expand_pairs(forms: &[Form]) -> Result<std::borrow::Cow<'_, [Form]>, OddArity> {
    let expanded = expand_reader_conds_cow(forms);
    if expanded.len().is_multiple_of(2) {
        Ok(expanded)
    } else {
        Err(OddArity(expanded.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cljrs_reader::Parser;

    fn parse_anon_fn(src: &str) -> Form {
        let mut parser = Parser::new(src.to_string(), "<test>".to_string());
        let form = parser.parse_one().unwrap().unwrap();
        let FormKind::AnonFn(body) = form.kind else {
            panic!("expected AnonFn, got {:?}", form.kind);
        };
        expand_anon_fn(&body, form.span)
    }

    fn arity(expanded: &Form) -> usize {
        let FormKind::List(parts) = &expanded.kind else {
            panic!("expected (fn* [...] ...)");
        };
        let FormKind::Vector(params) = &parts[1].kind else {
            panic!("expected param vector");
        };
        params.len()
    }

    #[test]
    fn deref_sugar_counts_as_one_arg() {
        // #(:x @%) must expand to (fn* [p__1] (:x (deref p__1))), arity 1 —
        // not 0, as if % under `@` were invisible to the arg scanner.
        let expanded = parse_anon_fn("#(:x @%)");
        assert_eq!(arity(&expanded), 1);

        let FormKind::List(parts) = &expanded.kind else {
            unreachable!()
        };
        let FormKind::List(body) = &parts[2].kind else {
            panic!("expected body list");
        };
        let FormKind::Deref(inner) = &body[1].kind else {
            panic!("expected deref form, got {:?}", body[1].kind);
        };
        assert_eq!(inner.kind, FormKind::Symbol("p__1".to_string()));
    }

    #[test]
    fn var_and_meta_sugar_also_scanned() {
        assert_eq!(arity(&parse_anon_fn("#(#'%)")), 1);
        assert_eq!(arity(&parse_anon_fn("#(^:x %)")), 1);
        assert_eq!(arity(&parse_anon_fn("#('%)")), 1);
    }

    // ── reader-conditional expansion (#?@ splice) ───────────────────────────

    fn parse_one_form(src: &str) -> Form {
        let mut p = Parser::new(src.to_string(), "<test>".to_string());
        p.parse_one().unwrap().unwrap()
    }

    fn vec_elems(src: &str) -> Vec<Form> {
        match parse_one_form(src).kind {
            FormKind::Vector(v) => v,
            other => panic!("expected vector literal, got {other:?}"),
        }
    }

    fn kinds(forms: &[Form]) -> Vec<FormKind> {
        forms.iter().map(|f| f.kind.clone()).collect()
    }

    /// Expand the elements of a `[...]` literal and return their kinds.
    fn expand_src(src: &str) -> Vec<FormKind> {
        kinds(&expand_reader_conds(&vec_elems(src)))
    }

    #[test]
    fn splice_rust_branch_flattens_into_parent() {
        assert_eq!(
            expand_src("[:x #?@(:rust [:a :b]) :y]"),
            kinds(&vec_elems("[:x :a :b :y]"))
        );
    }

    #[test]
    fn splice_falls_back_to_default_branch() {
        assert_eq!(
            expand_src("[:x #?@(:default [:a :b]) :y]"),
            kinds(&vec_elems("[:x :a :b :y]"))
        );
    }

    #[test]
    fn splice_with_no_matching_branch_is_removed_not_nil() {
        // `:clj` does not match under cljrs; the splice contributes nothing.
        // Regression: eval'ing the conditional on its own used to leave a `nil`.
        assert_eq!(
            expand_src("[:x #?@(:clj [:a :b]) :y]"),
            kinds(&vec_elems("[:x :y]"))
        );
    }

    #[test]
    fn non_splicing_conditional_selects_single_form() {
        assert_eq!(
            expand_src("[:x #?(:rust :a :default :b) :y]"),
            kinds(&vec_elems("[:x :a :y]"))
        );
    }

    #[test]
    fn nested_conditionals_inside_spliced_branch_expand() {
        assert_eq!(
            expand_src("[#?@(:rust [#?(:rust :a :default :z) :b])]"),
            kinds(&vec_elems("[:a :b]"))
        );
    }

    #[test]
    fn cow_borrows_when_no_conditional_present() {
        let forms = vec_elems("[:x :y :z]");
        assert!(matches!(
            expand_reader_conds_cow(&forms),
            std::borrow::Cow::Borrowed(_)
        ));
    }

    #[test]
    fn cow_owns_when_conditional_present() {
        let forms = vec_elems("[:x #?@(:rust [:a]) :y]");
        assert!(matches!(
            expand_reader_conds_cow(&forms),
            std::borrow::Cow::Owned(_)
        ));
    }

    fn kw_list(names: &[String]) -> String {
        names
            .iter()
            .map(|n| format!(":{n}"))
            .collect::<Vec<_>>()
            .join(" ")
    }

    // Model-based oracle. An `Item` is one position in a form sequence: a plain
    // keyword, a non-splicing conditional, or a splicing conditional. Each knows
    // both how to render itself as source and what it must contribute after
    // expansion under the `:rust` selection rule - an independent reimplementation
    // of `select_reader_cond` the production code is checked against.

    #[derive(Clone, Debug)]
    enum Item {
        Plain(String),
        NonSplice(Vec<(String, String)>),
        Splice(Vec<(String, Vec<String>)>),
    }

    // First matching clause in source order wins; `:default` always matches.
    fn select_model<T>(branches: &[(String, T)]) -> Option<&T> {
        branches
            .iter()
            .find(|(k, _)| is_platform_feature(k) || k == "default")
            .map(|(_, v)| v)
    }

    fn item_src(it: &Item) -> String {
        match it {
            Item::Plain(n) => format!(":{n}"),
            Item::NonSplice(brs) => {
                let inner = brs
                    .iter()
                    .map(|(k, n)| format!(":{k} :{n}"))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("#?({inner})")
            }
            Item::Splice(brs) => {
                let inner = brs
                    .iter()
                    .map(|(k, vs)| format!(":{k} [{}]", kw_list(vs)))
                    .collect::<Vec<_>>()
                    .join(" ");
                format!("#?@({inner})")
            }
        }
    }

    fn item_expected(it: &Item) -> Vec<String> {
        match it {
            Item::Plain(n) => vec![n.clone()],
            Item::NonSplice(brs) => select_model(brs).cloned().into_iter().collect(),
            Item::Splice(brs) => select_model(brs).cloned().unwrap_or_default(),
        }
    }

    fn key_strat() -> impl proptest::strategy::Strategy<Value = String> {
        proptest::prop_oneof![
            proptest::strategy::Just("rust".to_string()),
            proptest::strategy::Just("cljrs".to_string()),
            proptest::strategy::Just("clj".to_string()),
            proptest::strategy::Just("cljs".to_string()),
            proptest::strategy::Just("default".to_string()),
        ]
    }

    fn item_strat() -> impl proptest::strategy::Strategy<Value = Item> {
        use proptest::collection::vec as pvec;
        use proptest::strategy::Strategy;
        proptest::prop_oneof![
            "[a-z]{1,4}".prop_map(Item::Plain),
            pvec((key_strat(), "[a-z]{1,4}".prop_map(|s| s)), 1..4).prop_map(Item::NonSplice),
            pvec((key_strat(), pvec("[a-z]{1,4}", 0..3)), 1..4).prop_map(Item::Splice),
        ]
    }

    fn seq_src(items: &[Item]) -> String {
        format!(
            "[{}]",
            items.iter().map(item_src).collect::<Vec<_>>().join(" ")
        )
    }

    fn seq_expected_src(items: &[Item]) -> String {
        let names: Vec<String> = items.iter().flat_map(item_expected).collect();
        format!("[{}]", kw_list(&names))
    }

    proptest::proptest! {
        /// Splicing `#?@(:rust xs)` at a position equals inlining `xs` there.
        #[test]
        fn prop_splice_equals_inline(
            pre in proptest::collection::vec("[a-z]{1,4}", 0..4),
            mid in proptest::collection::vec("[a-z]{1,4}", 0..4),
            suf in proptest::collection::vec("[a-z]{1,4}", 0..4),
        ) {
            let spliced = format!("[{} #?@(:rust [{}]) {}]",
                                  kw_list(&pre), kw_list(&mid), kw_list(&suf));
            let inlined = format!("[{} {} {}]",
                                  kw_list(&pre), kw_list(&mid), kw_list(&suf));
            proptest::prop_assert_eq!(expand_src(&spliced), kinds(&vec_elems(&inlined)));
        }

        /// Expansion is idempotent: a second pass changes nothing.
        #[test]
        fn prop_expand_idempotent(
            pre in proptest::collection::vec("[a-z]{1,4}", 0..4),
            mid in proptest::collection::vec("[a-z]{1,4}", 0..4),
        ) {
            let src = format!("[{} #?@(:rust [{}])]", kw_list(&pre), kw_list(&mid));
            let once = expand_reader_conds(&vec_elems(&src));
            let twice = expand_reader_conds(&once);
            proptest::prop_assert_eq!(kinds(&once), kinds(&twice));
        }

        /// With no reader conditionals, expansion is the identity.
        #[test]
        fn prop_no_conditional_is_identity(
            xs in proptest::collection::vec("[a-z]{1,4}", 0..8),
        ) {
            let src = format!("[{}]", kw_list(&xs));
            proptest::prop_assert_eq!(expand_src(&src), kinds(&vec_elems(&src)));
        }

        /// Expansion of an arbitrary interleaving of plain / non-splicing /
        /// splicing conditionals (mixed keys) equals the model's expected output.
        #[test]
        fn prop_model_matches_expand(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            proptest::prop_assert_eq!(
                expand_src(&seq_src(&items)),
                kinds(&vec_elems(&seq_expected_src(&items)))
            );
        }

        /// No `ReaderCond` node survives expansion at the top level.
        #[test]
        fn prop_no_reader_cond_survives(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            let expanded = expand_reader_conds(&vec_elems(&seq_src(&items)));
            let none_survive = expanded
                .iter()
                .all(|f| !matches!(f.kind, FormKind::ReaderCond { .. }));
            proptest::prop_assert!(none_survive);
        }

        /// The expanded length equals the sum of each item's contributed count.
        #[test]
        fn prop_length_law(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            let expected_len: usize = items.iter().map(|it| item_expected(it).len()).sum();
            let got = expand_reader_conds(&vec_elems(&seq_src(&items)));
            proptest::prop_assert_eq!(got.len(), expected_len);
        }
    }

    // ── quoted containers: splice equals inline ─────────────────────────────
    //
    // One generated position in a container. Element names are assigned by
    // index so every element is distinct, which keeps generated set literals
    // legal and makes map keys unambiguous.

    #[derive(Clone, Debug)]
    enum Slot {
        /// A plain element; contributes 1.
        Plain,
        /// `#?(…)`; contributes 1 when the branch key matches, else 0.
        NonSplice { matches: bool },
        /// `#?@(…)`; contributes `n` when the branch key matches, else 0.
        Splice { matches: bool, n: usize },
    }

    fn slot_strat() -> impl proptest::strategy::Strategy<Value = Slot> {
        use proptest::strategy::{Just, Strategy};
        proptest::prop_oneof![
            Just(Slot::Plain),
            proptest::bool::ANY.prop_map(|matches| Slot::NonSplice { matches }),
            (proptest::bool::ANY, 0usize..3).prop_map(|(matches, n)| Slot::Splice { matches, n }),
        ]
    }

    /// Render `slots` twice: once with the conditionals written out, once with
    /// the selected branches already inlined. Both share one name counter, so
    /// corresponding elements carry identical names.
    fn render_slots(slots: &[Slot]) -> (String, String) {
        let mut next = 0usize;
        let mut name = || {
            let n = next;
            next += 1;
            format!(":k{n}")
        };
        let (mut written, mut inlined) = (Vec::new(), Vec::new());
        for slot in slots {
            match *slot {
                Slot::Plain => {
                    let k = name();
                    written.push(k.clone());
                    inlined.push(k);
                }
                Slot::NonSplice { matches } => {
                    let k = name();
                    let key = if matches { "rust" } else { "clj" };
                    written.push(format!("#?(:{key} {k})"));
                    if matches {
                        inlined.push(k);
                    }
                }
                Slot::Splice { matches, n } => {
                    let ks: Vec<String> = (0..n).map(|_| name()).collect();
                    let key = if matches { "rust" } else { "clj" };
                    written.push(format!("#?@(:{key} [{}])", ks.join(" ")));
                    if matches {
                        inlined.extend(ks);
                    }
                }
            }
        }
        (written.join(" "), inlined.join(" "))
    }

    fn quoted_value(src: &str) -> EvalResult<Value> {
        form_to_value(&parse_one_form(src))
    }

    /// Read `src` and convert it to a value, collapsing the read-time and
    /// convert-time rejections into one channel: a map literal is rejected by
    /// the reader when its parity is statically decidable and by
    /// `form_to_value` when a reader conditional deferred it.
    fn read_to_value(src: &str) -> Result<Value, String> {
        let mut p = Parser::new(src.to_string(), "<test>".to_string());
        let form = p
            .parse_one()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "no form".to_string())?;
        form_to_value(&form).map_err(|e| e.to_string())
    }

    proptest::proptest! {
        /// `expand_pairs` is exactly `expand_reader_conds` plus an even-length
        /// gate: it accepts precisely the expansions that can be chunked by two,
        /// and never alters what expansion produced.
        #[test]
        fn prop_expand_pairs_is_expansion_plus_parity_gate(
            items in proptest::collection::vec(item_strat(), 0..6),
        ) {
            let forms = vec_elems(&seq_src(&items));
            let expanded = expand_reader_conds(&forms);
            match expand_pairs(&forms) {
                Ok(got) => {
                    proptest::prop_assert!(expanded.len().is_multiple_of(2));
                    proptest::prop_assert_eq!(kinds(&got), kinds(&expanded));
                }
                Err(OddArity(len)) => {
                    proptest::prop_assert!(!expanded.len().is_multiple_of(2));
                    proptest::prop_assert_eq!(len, expanded.len());
                }
            }
        }

        /// The regression property for the quoted-data path: in EVERY container,
        /// a quoted form containing reader conditionals denotes the same value as
        /// the same container with the selected branches written out by hand.
        /// A container that drops, reorders, or nil-fills a splice fails here.
        #[test]
        fn prop_quoted_splice_equals_inline_in_every_container(
            slots in proptest::collection::vec(slot_strat(), 0..5),
        ) {
            let (written, inlined) = render_slots(&slots);
            for (open, close) in [("[", "]"), ("#{", "}"), ("(", ")"), ("{", "}")] {
                // A map needs an even number of forms to denote a value at all;
                // that case is the subject of its own property below.
                let expanded_len = expand_reader_conds(&vec_elems(&format!("[{written}]"))).len();
                if open == "{" && !expanded_len.is_multiple_of(2) {
                    continue;
                }
                let got = quoted_value(&format!("{open}{written}{close}"));
                let want = quoted_value(&format!("{open}{inlined}{close}"));
                proptest::prop_assert_eq!(
                    got.map_err(|e| e.to_string()),
                    want.map_err(|e| e.to_string()),
                    "container {}{}", open, close
                );
            }
        }

        /// A map denotes a value exactly when its EXPANSION has even length -
        /// the written parity says nothing, and it makes no difference whether
        /// the reader or the evaluator is the layer that rejects it. Truncating
        /// the odd tail (the old behaviour) would yield a value here instead.
        #[test]
        fn prop_map_denotes_a_value_iff_expansion_is_even(
            slots in proptest::collection::vec(slot_strat(), 0..5),
        ) {
            let (written, _) = render_slots(&slots);
            let expanded_len = expand_reader_conds(&vec_elems(&format!("[{written}]"))).len();
            let got = read_to_value(&format!("{{{written}}}"));
            proptest::prop_assert_eq!(got.is_ok(), expanded_len.is_multiple_of(2), "{:?}", got);
        }

        /// Nesting a container inside a quoted container does not change the
        /// values the inner splice contributes.
        #[test]
        fn prop_quoted_splice_survives_nesting(
            slots in proptest::collection::vec(slot_strat(), 0..4),
        ) {
            let (written, inlined) = render_slots(&slots);
            let got = quoted_value(&format!("[:head [{written}] :tail]"));
            let want = quoted_value(&format!("[:head [{inlined}] :tail]"));
            proptest::prop_assert_eq!(
                got.map_err(|e| e.to_string()),
                want.map_err(|e| e.to_string())
            );
        }
    }

    // ── examples from the PR #294 review ────────────────────────────────────

    #[test]
    fn quoted_map_splices_into_key_value_positions() {
        assert_eq!(
            quoted_value("'{:a 1 #?@(:rust [:b 2]) :c 3}").unwrap(),
            quoted_value("'{:a 1 :b 2 :c 3}").unwrap()
        );
    }

    #[test]
    fn quoted_set_splices_its_elements() {
        assert_eq!(
            quoted_value("'#{1 #?@(:rust [2 3]) 4}").unwrap(),
            quoted_value("'#{1 2 3 4}").unwrap()
        );
    }

    #[test]
    fn quoted_map_with_odd_expansion_errors_rather_than_truncating() {
        let err = quoted_value("{:a 1 #?@(:rust [:b]) :c 3}").unwrap_err();
        assert!(err.to_string().contains("even number of forms"), "{err}");
    }

    #[test]
    fn uuid_tagged_literal_reads_as_a_uuid_value() {
        let got = quoted_value("#uuid \"f81d4fae-7dec-11d0-a765-00a0c91e6bf6\"").unwrap();
        assert_eq!(
            got,
            Value::uuid(0xf81d4fae_7dec_11d0_a765_00a0c91e6bf6_u128)
        );
    }

    #[test]
    fn splice_without_a_sibling_sequence_errors() {
        let err = quoted_value("#?@(:rust [1 2])").unwrap_err();
        assert!(err.to_string().contains("sequence context"), "{err}");
    }
}
