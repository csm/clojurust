// ── form_to_value ─────────────────────────────────────────────────────────────

use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind};
use cljrs_value::value::SetValue;
use cljrs_value::{
    Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value,
};
use regex::Regex;

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
            _ => {}
        }
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
        _ => form.clone(),
    }
}

/// Convert a `Form` to its literal `Value` without evaluating.
/// Used by `quote` and macro expansion.
pub fn form_to_value(form: &Form) -> Value {
    match &form.kind {
        FormKind::Nil => Value::Nil,
        FormKind::Bool(b) => Value::Bool(*b),
        FormKind::Int(n) => Value::Long(*n),
        FormKind::Float(f) => Value::Double(*f),
        FormKind::Symbolic(f) => Value::Double(*f),
        FormKind::Str(s) => Value::string(s.clone()),
        FormKind::Char(c) => Value::Char(*c),
        FormKind::BigInt(s) => crate::parse_bigint(s).unwrap_or(Value::Nil),
        FormKind::BigDecimal(s) => crate::parse_bigdecimal(s).unwrap_or(Value::Nil),
        FormKind::Ratio(s) => crate::parse_ratio(s).unwrap_or(Value::Nil),

        FormKind::Symbol(s) => Value::symbol(Symbol::parse(s)),
        FormKind::Keyword(s) => Value::keyword(Keyword::parse(s)),
        FormKind::AutoKeyword(s) => Value::keyword(Keyword::simple(s.as_str())),
        FormKind::Regex(s) => match Regex::new(s.as_str()) {
            Ok(pattern) => Value::Pattern(GcPtr::new(pattern)),
            Err(_) => Value::Nil, // should already have been caught
        },

        FormKind::List(forms) => {
            let expanded = expand_reader_conds(forms);
            let items: Vec<Value> = expanded.iter().map(form_to_value).collect();
            Value::List(GcPtr::new(PersistentList::from_iter(items)))
        }
        FormKind::Vector(forms) => {
            let expanded = expand_reader_conds(forms);
            let items: Vec<Value> = expanded.iter().map(form_to_value).collect();
            Value::Vector(GcPtr::new(PersistentVector::from_iter(items)))
        }
        FormKind::Map(forms) => {
            let mut m = MapValue::empty();
            for pair in forms.chunks(2) {
                if pair.len() == 2 {
                    m = m.assoc(form_to_value(&pair[0]), form_to_value(&pair[1]));
                }
            }
            Value::Map(m)
        }
        FormKind::Set(forms) => {
            let s = forms
                .iter()
                .fold(PersistentHashSet::empty(), |s, f| s.conj(form_to_value(f)));
            Value::Set(SetValue::Hash(GcPtr::new(s)))
        }

        FormKind::Quote(inner) => {
            // `'x` → the form x as a data value.
            Value::List(GcPtr::new(PersistentList::from_iter([
                Value::symbol(Symbol::simple("quote")),
                form_to_value(inner),
            ])))
        }
        FormKind::SyntaxQuote(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("syntax-quote")),
            form_to_value(inner),
        ]))),
        FormKind::Unquote(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("unquote")),
            form_to_value(inner),
        ]))),
        FormKind::UnquoteSplice(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("unquote-splicing")),
            form_to_value(inner),
        ]))),
        FormKind::Deref(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("deref")),
            form_to_value(inner),
        ]))),
        FormKind::Var(inner) => Value::List(GcPtr::new(PersistentList::from_iter([
            Value::symbol(Symbol::simple("var")),
            form_to_value(inner),
        ]))),
        FormKind::Meta(_meta, inner) => form_to_value(inner),
        FormKind::AnonFn(body) => {
            // Expand #(...) to (fn* [...] ...) so it round-trips correctly through quote.
            let expanded = expand_anon_fn(body, form.span.clone());
            form_to_value(&expanded)
        }
        FormKind::TaggedLiteral(tag, inner) => match tag.as_str() {
            "uuid" => {
                if let FormKind::Str(s) = &inner.kind {
                    match uuid::Uuid::parse_str(s) {
                        Ok(u) => Value::Uuid(u.as_u128()),
                        Err(_) => form_to_value(inner),
                    }
                } else {
                    form_to_value(inner)
                }
            }
            _ => form_to_value(inner),
        },
        FormKind::ReaderCond {
            splicing: false,
            clauses,
        } => select_reader_cond(clauses).map_or(Value::Nil, form_to_value),
        FormKind::ReaderCond { splicing: true, .. } => Value::Nil, // splice must be handled by parent
    }
}

/// Resolve a `#?(...)` reader conditional to the selected branch form, or
/// `None` if no `:rust` or `:default` clause is present.
pub fn select_reader_cond(clauses: &[Form]) -> Option<&Form> {
    let mut default: Option<&Form> = None;
    let mut i = 0;
    while i + 1 < clauses.len() {
        match &clauses[i].kind {
            FormKind::Keyword(k) if k == "rust" => return Some(&clauses[i + 1]),
            FormKind::Keyword(k) if k == "default" => default = Some(&clauses[i + 1]),
            _ => {}
        }
        i += 2;
    }
    default
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
