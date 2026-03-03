//! Sequential destructuring for `let*`, `fn*`, and `loop*`.

use std::sync::Arc;

use cljx_gc::GcPtr;
use cljx_reader::Form;
use cljx_reader::form::FormKind;
use cljx_value::{PersistentList, Value};

use crate::env::Env;
use crate::error::{EvalError, EvalResult};

/// Bind a destructuring pattern `pattern` against `val` in `env`.
///
/// Supports:
/// - Plain symbol binding
/// - `[a b]` sequential destructuring (recursive)
pub fn bind_pattern(pattern: &Form, val: Value, env: &mut Env) -> EvalResult<()> {
    match &pattern.kind {
        FormKind::Symbol(s) => {
            // Plain binding, including `_`.
            env.bind(Arc::from(s.as_str()), val);
            Ok(())
        }
        FormKind::Vector(forms) => bind_sequential(forms, &val, env),
        _ => Err(EvalError::Runtime(format!(
            "unsupported binding pattern: {:?}",
            pattern.kind
        ))),
    }
}

/// Bind a sequential destructuring pattern against `val`.
///
/// Grammar of `pattern`:
/// ```text
/// [sym* (& rest)? (:as alias)?]
/// ```
pub fn bind_sequential(pattern: &[Form], val: &Value, env: &mut Env) -> EvalResult<()> {
    let items = value_to_seq_vec(val);
    let mut idx = 0usize;
    let mut i = 0usize;

    while i < pattern.len() {
        let p = &pattern[i];

        // `&` introduces a rest binding.
        if matches!(&p.kind, FormKind::Symbol(s) if s == "&") {
            i += 1;
            let rest_pat = pattern
                .get(i)
                .ok_or_else(|| EvalError::Runtime("& in destructuring requires a name".into()))?;
            let rest_vals: Vec<Value> = items[idx..].to_vec();
            let rest_list = Value::List(GcPtr::new(PersistentList::from_iter(rest_vals)));
            bind_pattern(rest_pat, rest_list, env)?;
            i += 1;
            // Skip optional `:as` after rest.
            if i < pattern.len()
                && let FormKind::Keyword(k) = &pattern[i].kind
                && k == "as"
            {
                i += 1;
                let alias = pattern
                    .get(i)
                    .ok_or_else(|| EvalError::Runtime(":as requires a name".into()))?;
                bind_pattern(alias, val.clone(), env)?;
            }
            break;
        }

        // `:as` alias — must be last.
        if let FormKind::Keyword(k) = &p.kind
            && k == "as"
        {
            i += 1;
            let alias = pattern
                .get(i)
                .ok_or_else(|| EvalError::Runtime(":as requires a name".into()))?;
            bind_pattern(alias, val.clone(), env)?;
            break;
        }

        // Normal positional binding.
        let item = items.get(idx).cloned().unwrap_or(Value::Nil);
        bind_pattern(p, item, env)?;
        idx += 1;
        i += 1;
    }

    Ok(())
}

/// Convert any sequential Value to a Vec of its elements.
pub fn value_to_seq_vec(val: &Value) -> Vec<Value> {
    match val {
        Value::Nil => vec![],
        Value::List(l) => l.get().iter().cloned().collect(),
        Value::Vector(v) => v.get().iter().cloned().collect(),
        Value::Set(s) => s.get().iter().collect(),
        _ => vec![],
    }
}
