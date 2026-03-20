//! Sequential and associative destructuring for `let*`, `fn*`, and `loop*`.

use std::sync::Arc;

use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_value::{Keyword, PersistentList, Symbol, Value};

use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::eval::form_to_value;

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
        FormKind::Map(forms) => bind_associative(forms, &val, env),
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
            let rest_list = if idx < items.len() {
                let rest_vals: Vec<Value> = items[idx..].to_vec();
                Value::List(GcPtr::new(PersistentList::from_iter(rest_vals)))
            } else {
                Value::Nil
            };
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
        Value::WithMeta(inner, _) => value_to_seq_vec(inner),
        Value::Nil => vec![],
        Value::LazySeq(ls) => value_to_seq_vec(&ls.get().realize()),
        Value::Cons(c) => {
            let mut result = vec![c.get().head.clone()];
            let mut tail = c.get().tail.clone();
            loop {
                match tail {
                    Value::Nil => break,
                    Value::List(l) => {
                        result.extend(l.get().iter().cloned());
                        break;
                    }
                    Value::Cons(next_c) => {
                        result.push(next_c.get().head.clone());
                        tail = next_c.get().tail.clone();
                    }
                    Value::LazySeq(ls) => {
                        tail = ls.get().realize();
                    }
                    _ => break,
                }
            }
            result
        }
        Value::List(l) => l.get().iter().cloned().collect(),
        Value::Vector(v) => v.get().iter().cloned().collect(),
        Value::Set(s) => s.iter().cloned().collect(),
        _ => vec![],
    }
}

// ── Associative destructuring ─────────────────────────────────────────────────

/// Bind a map destructuring pattern against `val` in `env`.
///
/// `pattern` is a flat `[key val key val ...]` slice from `FormKind::Map`.
///
/// Supports:
/// - `:keys [a b c]`   — bind symbols from keyword keys `:a`, `:b`, `:c`
/// - `:strs [a b]`     — bind symbols from string keys `"a"`, `"b"`
/// - `:syms [a b]`     — bind symbols from symbol keys `'a`, `'b`
/// - `:as name`        — bind the whole value to `name`
/// - `:or {a default}` — default value for missing keys
/// - Regular `{sym :key}` direct bindings
pub fn bind_associative(pattern: &[Form], val: &Value, env: &mut Env) -> EvalResult<()> {
    // First pass: collect :or defaults.
    let mut defaults: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    let mut i = 0;
    while i + 1 < pattern.len() {
        let k = &pattern[i];
        let v = &pattern[i + 1];
        if let FormKind::Keyword(kw) = &k.kind
            && kw == "or"
        {
            // v is a map literal {sym default ...}
            if let FormKind::Map(or_forms) = &v.kind {
                let mut j = 0;
                while j + 1 < or_forms.len() {
                    if let FormKind::Symbol(sym) = &or_forms[j].kind {
                        defaults.insert(sym.clone(), form_to_value(&or_forms[j + 1]));
                    }
                    j += 2;
                }
            }
        }
        i += 2;
    }

    let get_val = |key: &Value| -> Value {
        match val.unwrap_meta() {
            Value::Map(m) => m.get(key).unwrap_or(Value::Nil),
            _ => Value::Nil,
        }
    };

    let mut i = 0;
    while i + 1 < pattern.len() {
        let k = &pattern[i];
        let v = &pattern[i + 1];
        i += 2;

        match &k.kind {
            FormKind::Keyword(kw) if kw == "keys" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let key = Value::keyword(Keyword::simple(sym.as_str()));
                            let mut bound_val = get_val(&key);
                            if matches!(bound_val, Value::Nil)
                                && let Some(d) = defaults.get(sym.as_str())
                            {
                                bound_val = d.clone();
                            }
                            env.bind(Arc::from(sym.as_str()), bound_val);
                        }
                    }
                }
            }
            FormKind::Keyword(kw) if kw == "strs" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let key = Value::string(sym.clone());
                            let mut bound_val = get_val(&key);
                            if matches!(bound_val, Value::Nil)
                                && let Some(d) = defaults.get(sym.as_str())
                            {
                                bound_val = d.clone();
                            }
                            env.bind(Arc::from(sym.as_str()), bound_val);
                        }
                    }
                }
            }
            FormKind::Keyword(kw) if kw == "syms" => {
                if let FormKind::Vector(syms) = &v.kind {
                    for sym_form in syms {
                        if let FormKind::Symbol(sym) = &sym_form.kind {
                            let key = Value::symbol(Symbol::simple(sym.as_str()));
                            let mut bound_val = get_val(&key);
                            if matches!(bound_val, Value::Nil)
                                && let Some(d) = defaults.get(sym.as_str())
                            {
                                bound_val = d.clone();
                            }
                            env.bind(Arc::from(sym.as_str()), bound_val);
                        }
                    }
                }
            }
            FormKind::Keyword(kw) if kw == "as" => {
                if let FormKind::Symbol(sym) = &v.kind {
                    env.bind(Arc::from(sym.as_str()), val.clone());
                }
            }
            FormKind::Keyword(kw) if kw == "or" => {
                // Already processed in the first pass.
            }
            _ => {
                // Regular {binding-form lookup-key} pair.
                // In Clojure map destructuring {a :x}, the key position is the
                // binding target and the value position is the lookup key.
                let lookup_key = form_to_value(v);
                let mut bound_val = get_val(&lookup_key);
                // Apply defaults for simple symbol bindings.
                if matches!(bound_val, Value::Nil) {
                    if let FormKind::Symbol(sym) = &k.kind {
                        if let Some(d) = defaults.get(sym.as_str()) {
                            bound_val = d.clone();
                        }
                    }
                }
                // Bind via pattern to support nested destructuring.
                bind_pattern(k, bound_val, env)?;
            }
        }
    }
    Ok(())
}
