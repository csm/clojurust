//! Macro expansion pipeline.

use std::sync::Arc;

use cljx_reader::Form;
use cljx_reader::form::FormKind;
use cljx_types::span::Span;
use cljx_value::{Symbol, Value};

use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::eval::form_to_value;

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
            let mut m = cljx_value::MapValue::empty();
            for (name, val) in names.iter().zip(vals.iter()) {
                m = m.assoc(
                    Value::symbol(Symbol::simple(name.as_ref())),
                    val.clone(),
                );
            }
            Value::Map(m)
        };
        let mut args = vec![form_val, env_val];
        args.extend(parts[1..].iter().map(form_to_value));
        let expanded = crate::apply::call_cljx_fn(&macro_fn, args, env)?;
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

/// If `sym` resolves to a macro in the current env, return its CljxFn.
fn resolve_macro(sym: &str, env: &Env) -> Option<cljx_value::CljxFn> {
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

        Value::Symbol(s) => FormKind::Symbol(s.get().full_name()),
        Value::Keyword(k) => FormKind::Keyword(k.get().full_name()),

        Value::List(l) => {
            let forms: Vec<Form> = l
                .get()
                .iter()
                .map(|v| value_to_form(v, span.clone()))
                .collect::<EvalResult<_>>()?;
            FormKind::List(forms)
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
                .get()
                .iter()
                .map(|v| value_to_form(&v, span.clone()))
                .collect::<EvalResult<_>>()?;
            FormKind::Set(forms)
        }

        // Non-data types: wrap in a symbol placeholder (best effort).
        other => FormKind::Symbol(format!("#<{}>", other.type_name())),
    };
    Ok(Form::new(kind, span))
}
