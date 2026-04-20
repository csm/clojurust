//! Macro expansion pipeline.

use std::sync::Arc;
use cljrs_builtins::form::form_to_value;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_types::span::Span;
use cljrs_value::{Symbol, Value};

use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};

/// Expand a form one step.  Returns the same form if it is not a macro call.
pub fn macroexpand_1(form: &Form, env: &mut Env) -> EvalResult<Form> {
    // Only expand list forms whose head is a macro symbol.
    if let FormKind::List(parts) = &form.kind
        && let Some(FormKind::Symbol(s)) = parts.first().map(|f| &f.kind)
        && let Some(macro_fn) = resolve_macro(s, env)
    {
        // Build &form value (the whole call as a list).
        let form_val = form_to_value(form);
        // Build &env value (local bindings as a map — empty at top level).
        let env_val = {
            let (names, vals) = env.all_local_bindings();
            let mut m = cljrs_value::MapValue::empty();
            for (name, val) in names.iter().zip(vals.iter()) {
                m = m.assoc(Value::symbol(Symbol::simple(name.as_ref())), val.clone());
            }
            Value::Map(m)
        };
        let mut args = vec![form_val, env_val];
        args.extend(parts[1..].iter().map(form_to_value));
        let expanded = crate::apply::call_cljrs_fn(&macro_fn, &args, env)?;
        let dummy = Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
        return value_to_form(&expanded, dummy);
    }
    Ok(form.clone())
}

/// Fully expand a form until the head is no longer a macro.
pub fn macroexpand(form: &Form, env: &mut Env) -> EvalResult<Form> {
    let mut current = form.clone();
    loop {
        let expanded = macroexpand_1(&current, env)?;
        if expanded == current {
            return Ok(current);
        }
        current = expanded;
    }
}

/// Recursively macro-expand all forms in a tree.
///
/// First expands the top-level form, then walks into sub-forms.
/// Special forms like `quote` are not walked into.
pub fn macroexpand_all(form: &Form, env: &mut Env) -> EvalResult<Form> {
    // First, expand the top level.
    let expanded = macroexpand(form, env)?;

    let span = expanded.span.clone();
    let kind = match &expanded.kind {
        FormKind::List(parts) if !parts.is_empty() => {
            // Check if the head is a special form that shouldn't be walked.
            let head_name = match &parts[0].kind {
                FormKind::Symbol(s) => Some(s.as_str()),
                _ => None,
            };
            match head_name {
                // quote: don't expand inside quoted forms
                Some("quote") => return Ok(expanded),
                // fn*: expand body forms but not the param vector
                Some("fn*") => {
                    let mut new_parts = vec![parts[0].clone()];
                    // fn* can have multiple arities: (fn* ([x] body) ([x y] body2))
                    // or single arity: (fn* [x] body)
                    if parts.len() > 1 {
                        if let FormKind::Vector(_) = &parts[1].kind {
                            // Single arity: (fn* [params] body...)
                            new_parts.push(parts[1].clone()); // params
                            for p in &parts[2..] {
                                new_parts.push(macroexpand_all(p, env)?);
                            }
                        } else {
                            // Multi-arity: (fn* ([params] body) ...)
                            for arity in &parts[1..] {
                                if let FormKind::List(arity_parts) = &arity.kind {
                                    let mut new_arity = Vec::new();
                                    if let Some(params) = arity_parts.first() {
                                        new_arity.push(params.clone()); // param vector
                                    }
                                    for p in arity_parts.iter().skip(1) {
                                        new_arity.push(macroexpand_all(p, env)?);
                                    }
                                    new_parts.push(Form::new(
                                        FormKind::List(new_arity),
                                        arity.span.clone(),
                                    ));
                                } else {
                                    // Name or other token before arities
                                    new_parts.push(arity.clone());
                                }
                            }
                        }
                    }
                    FormKind::List(new_parts)
                }
                // let*, loop*: expand bindings values and body, but not binding names
                Some("let*") | Some("loop*") => {
                    let mut new_parts = vec![parts[0].clone()];
                    if parts.len() > 1 {
                        // Expand binding values (every other form in the vector)
                        if let FormKind::Vector(bindings) = &parts[1].kind {
                            let mut new_bindings = Vec::new();
                            for (i, b) in bindings.iter().enumerate() {
                                if i % 2 == 0 {
                                    new_bindings.push(b.clone()); // binding name
                                } else {
                                    new_bindings.push(macroexpand_all(b, env)?);
                                }
                            }
                            new_parts.push(Form::new(
                                FormKind::Vector(new_bindings),
                                parts[1].span.clone(),
                            ));
                        } else {
                            new_parts.push(parts[1].clone());
                        }
                        for p in &parts[2..] {
                            new_parts.push(macroexpand_all(p, env)?);
                        }
                    }
                    FormKind::List(new_parts)
                }
                // catch/finally inside try: handled naturally by walking
                _ => {
                    // Generic: expand all sub-forms
                    let new_parts = parts
                        .iter()
                        .map(|p| macroexpand_all(p, env))
                        .collect::<EvalResult<Vec<_>>>()?;
                    FormKind::List(new_parts)
                }
            }
        }
        FormKind::Vector(items) => {
            let new_items = items
                .iter()
                .map(|i| macroexpand_all(i, env))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Vector(new_items)
        }
        FormKind::Map(items) => {
            let new_items = items
                .iter()
                .map(|i| macroexpand_all(i, env))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Map(new_items)
        }
        FormKind::Set(items) => {
            let new_items = items
                .iter()
                .map(|i| macroexpand_all(i, env))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Set(new_items)
        }
        // Atoms, keywords, strings, etc. — no sub-forms.
        _ => return Ok(expanded),
    };
    Ok(Form::new(kind, span))
}

/// If `sym` resolves to a macro in the current env, return its CljxFn.
fn resolve_macro(sym: &str, env: &Env) -> Option<cljrs_value::CljxFn> {
    let parsed = Symbol::parse(sym);
    let ns = parsed.namespace.as_deref().unwrap_or(&env.current_ns);
    let name = parsed.name.as_ref();

    let v = env.globals.lookup_in_ns(ns, name)?;
    if let Value::Macro(f) = v {
        Some(f.get().clone())
    } else {
        None
    }
}

/// Convert a `Value` to a `Form` (inverse of `form_to_value`).
///
/// Used to convert a macro's output back to a Form for further evaluation.
pub fn value_to_form(val: &Value, span: Span) -> EvalResult<Form> {
    let kind = match val {
        Value::Nil => FormKind::Nil,
        Value::Bool(b) => FormKind::Bool(*b),
        Value::Long(n) => FormKind::Int(*n),
        Value::Double(f) => FormKind::Float(*f),
        Value::Str(s) => FormKind::Str(s.get().clone()),
        Value::Char(c) => FormKind::Char(*c),
        Value::BigInt(b) => FormKind::BigInt(b.get().to_string()),
        Value::BigDecimal(d) => FormKind::BigDecimal(d.get().to_string()),
        Value::Ratio(r) => FormKind::Ratio(format!("{}/{}", r.get().numer(), r.get().denom())),

        Value::Symbol(s) => FormKind::Symbol(s.get().full_name()),
        Value::Keyword(k) => FormKind::Keyword(k.get().full_name()),

        Value::List(l) => {
            let items = l.get();
            // Reconstruct reader special forms that were encoded as lists by form_to_value.
            let head_sym = items.iter().next().and_then(|v| {
                if let Value::Symbol(s) = v {
                    Some(s.get().name.clone())
                } else {
                    None
                }
            });
            match (head_sym.as_deref(), items.count()) {
                (Some("syntax-quote"), 2) => {
                    let inner = value_to_form(items.iter().nth(1).unwrap(), span.clone())?;
                    FormKind::SyntaxQuote(Box::new(inner))
                }
                (Some("unquote"), 2) => {
                    let inner = value_to_form(items.iter().nth(1).unwrap(), span.clone())?;
                    FormKind::Unquote(Box::new(inner))
                }
                (Some("unquote-splicing"), 2) => {
                    let inner = value_to_form(items.iter().nth(1).unwrap(), span.clone())?;
                    FormKind::UnquoteSplice(Box::new(inner))
                }
                _ => {
                    let forms: Vec<Form> = items
                        .iter()
                        .map(|v| value_to_form(v, span.clone()))
                        .collect::<EvalResult<_>>()?;
                    FormKind::List(forms)
                }
            }
        }
        Value::Vector(v) => {
            let forms: Vec<Form> = v
                .get()
                .iter()
                .map(|v| value_to_form(v, span.clone()))
                .collect::<EvalResult<_>>()?;
            FormKind::Vector(forms)
        }
        Value::Map(m) => {
            let mut forms = Vec::new();
            let mut err: Option<EvalError> = None;
            let sc = span.clone();
            m.for_each(|k, v| {
                if err.is_none() {
                    match (value_to_form(k, sc.clone()), value_to_form(v, sc.clone())) {
                        (Ok(kf), Ok(vf)) => {
                            forms.push(kf);
                            forms.push(vf);
                        }
                        (Err(e), _) | (_, Err(e)) => err = Some(e),
                    }
                }
            });
            if let Some(e) = err {
                return Err(e);
            }
            FormKind::Map(forms)
        }
        Value::Set(s) => {
            let forms: Vec<Form> = s
                .iter()
                .map(|v| value_to_form(v, span.clone()))
                .collect::<EvalResult<_>>()?;
            FormKind::Set(forms)
        }

        // Lazy sequences and cons cells: materialize into a list form.
        // This handles macro output like (cons 'do (map ...)).
        Value::LazySeq(ls) => {
            return value_to_form(&ls.get().realize(), span);
        }
        Value::Cons(c) => {
            let mut items: Vec<Form> = Vec::new();
            let mut cur = Value::Cons(c.clone());
            loop {
                match cur {
                    Value::Cons(cell) => {
                        items.push(value_to_form(&cell.get().head, span.clone())?);
                        cur = cell.get().tail.clone();
                    }
                    Value::LazySeq(ls) => cur = ls.get().realize(),
                    Value::List(l) => {
                        for v in l.get().iter() {
                            items.push(value_to_form(v, span.clone())?);
                        }
                        break;
                    }
                    Value::Nil => break,
                    _ => break,
                }
            }
            FormKind::List(items)
        }

        Value::Uuid(u) => {
            let uuid_str = uuid::Uuid::from_u128(*u).to_string();
            FormKind::TaggedLiteral(
                "uuid".to_string(),
                Box::new(Form::new(FormKind::Str(uuid_str), span.clone())),
            )
        }

        // WithMeta: strip metadata and convert the inner value.
        Value::WithMeta(inner, _) => {
            return value_to_form(inner, span);
        }

        Value::Pattern(p) => FormKind::Regex(p.get().as_str().to_string()),

        // Non-data types: wrap in a symbol placeholder (best effort).
        other => FormKind::Symbol(format!("#<{}>", other.type_name())),
    };
    Ok(Form::new(kind, span))
}
