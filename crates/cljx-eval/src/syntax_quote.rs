//! Syntax-quote (backtick) expansion.
//!
//! Syntax-quote is evaluated directly to a `Value`, rather than being
//! expanded to intermediate AST and then evaluated.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cljx_gc::GcPtr;
use cljx_reader::Form;
use cljx_reader::form::FormKind;
use cljx_value::{Keyword, PersistentList, PersistentVector, Symbol, Value};

use crate::env::Env;
use crate::error::{EvalError, EvalResult};

static GENSYM_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Expand a syntax-quoted form to a Value.
pub fn syntax_quote(form: &Form, env: &mut Env) -> EvalResult {
    let mut gensyms = std::collections::HashMap::new();
    sq_form(form, env, &mut gensyms)
}

fn sq_form(
    form: &Form,
    env: &mut Env,
    gensyms: &mut std::collections::HashMap<String, Arc<str>>,
) -> EvalResult {
    match &form.kind {
        // ~expr — evaluate normally.
        FormKind::Unquote(inner) => crate::eval::eval(inner, env),

        // ~@expr at top level is an error.
        FormKind::UnquoteSplice(_) => Err(EvalError::Runtime(
            "splice-unquote outside list/vector context".into(),
        )),

        // Symbols: auto-qualify with current namespace; auto-gensym `foo#`.
        FormKind::Symbol(s) => {
            let qualified = qualify_symbol(s, env, gensyms);
            Ok(Value::symbol(Symbol::parse(&qualified)))
        }

        // Atoms: return as quoted values.
        FormKind::Nil => Ok(Value::Nil),
        FormKind::Bool(b) => Ok(Value::Bool(*b)),
        FormKind::Int(n) => Ok(Value::Long(*n)),
        FormKind::Float(f) => Ok(Value::Double(*f)),
        FormKind::Str(s) => Ok(Value::string(s.clone())),
        FormKind::Char(c) => Ok(Value::Char(*c)),
        FormKind::Keyword(s) => Ok(Value::keyword(Keyword::parse(s))),
        FormKind::AutoKeyword(s) => {
            let full = format!("{}/{}", env.current_ns, s);
            Ok(Value::keyword(Keyword::parse(&full)))
        }

        // Lists: process each element, splicing ~@ items.
        FormKind::List(forms) => {
            let parts = sq_seq(forms, env, gensyms)?;
            // Concatenate segments.
            let mut out: Vec<Value> = Vec::new();
            for part in parts {
                match part {
                    Segment::One(v) => out.push(v),
                    Segment::Many(vs) => out.extend(vs),
                }
            }
            Ok(Value::List(GcPtr::new(PersistentList::from_iter(out))))
        }

        // Vectors: process each element, splicing ~@ items.
        FormKind::Vector(forms) => {
            let parts = sq_seq(forms, env, gensyms)?;
            let mut out: Vec<Value> = Vec::new();
            for part in parts {
                match part {
                    Segment::One(v) => out.push(v),
                    Segment::Many(vs) => out.extend(vs),
                }
            }
            Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(out))))
        }

        // Maps: treat flat k/v pairs like a sequence.
        FormKind::Map(forms) => {
            let parts = sq_seq(forms, env, gensyms)?;
            let mut out: Vec<Value> = Vec::new();
            for part in parts {
                match part {
                    Segment::One(v) => out.push(v),
                    Segment::Many(vs) => out.extend(vs),
                }
            }
            if !out.len().is_multiple_of(2) {
                return Err(EvalError::Runtime(
                    "syntax-quote map requires even number of forms".into(),
                ));
            }
            let mut m = cljx_value::MapValue::empty();
            for pair in out.chunks(2) {
                m = m.assoc(pair[0].clone(), pair[1].clone());
            }
            Ok(Value::Map(m))
        }

        // Sets.
        FormKind::Set(forms) => {
            let parts = sq_seq(forms, env, gensyms)?;
            let mut out: Vec<Value> = Vec::new();
            for part in parts {
                match part {
                    Segment::One(v) => out.push(v),
                    Segment::Many(vs) => out.extend(vs),
                }
            }
            let set = out
                .into_iter()
                .fold(cljx_value::PersistentHashSet::empty(), |s, v| s.conj(v));
            Ok(Value::Set(GcPtr::new(set)))
        }

        // Everything else: wrap in quote.
        _other => Ok(crate::eval::form_to_value(form)),
    }
}

enum Segment {
    One(Value),
    Many(Vec<Value>),
}

fn sq_seq(
    forms: &[Form],
    env: &mut Env,
    gensyms: &mut std::collections::HashMap<String, Arc<str>>,
) -> EvalResult<Vec<Segment>> {
    let mut out = Vec::with_capacity(forms.len());
    for f in forms {
        match &f.kind {
            FormKind::UnquoteSplice(inner) => {
                // ~@expr: evaluate and spread.
                let v = crate::eval::eval(inner, env)?;
                let items = crate::destructure::value_to_seq_vec(&v);
                out.push(Segment::Many(items));
            }
            _ => {
                let v = sq_form(f, env, gensyms)?;
                out.push(Segment::One(v));
            }
        }
    }
    Ok(out)
}

/// Qualify a symbol name for use inside syntax-quote.
///
/// - `foo#` → unique gensym `foo__N__auto__` (same N within one backtick).
/// - `ns/foo` → kept as-is (already qualified).
/// - Special symbols (`nil`, `true`, `false`) → kept as-is.
/// - Everything else → `current-ns/name`.
fn qualify_symbol(
    s: &str,
    env: &Env,
    gensyms: &mut std::collections::HashMap<String, Arc<str>>,
) -> String {
    // Already qualified.
    if s.contains('/') {
        return s.to_string();
    }
    // Special literals.
    if matches!(s, "nil" | "true" | "false") {
        return s.to_string();
    }
    // Auto-gensym: `foo#`.
    if let Some(base) = s.strip_suffix('#') {
        let generated = gensyms.entry(s.to_string()).or_insert_with(|| {
            let n = GENSYM_COUNTER.fetch_add(1, Ordering::Relaxed);
            Arc::from(format!("{base}__{n}__auto__"))
        });
        return generated.as_ref().to_string();
    }
    // Default: qualify with current namespace.
    format!("{}/{}", env.current_ns, s)
}
