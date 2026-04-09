//! Top-level `eval` dispatcher and form-to-value conversion.

use std::sync::Arc;

use crate::apply::eval_call;
use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::special::{SPECIAL_FORMS, eval_special, select_reader_cond};
use crate::syntax_quote::syntax_quote;
use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_value::value::SetValue;
use cljrs_value::{
    FutureState, Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, Symbol,
    Value,
};
use regex::Regex;

/// Evaluate a `Form` in the given `Env`.
pub fn eval(form: &Form, env: &mut Env) -> EvalResult {
    match &form.kind {
        // ── Atoms ─────────────────────────────────────────────────────────
        FormKind::Nil => Ok(Value::Nil),
        FormKind::Bool(b) => Ok(Value::Bool(*b)),
        FormKind::Int(n) => Ok(Value::Long(*n)),
        FormKind::Float(f) => Ok(Value::Double(*f)),
        FormKind::Symbolic(f) => Ok(Value::Double(*f)), // ##Inf etc.
        FormKind::Str(s) => Ok(Value::string(s.clone())),
        FormKind::Char(c) => Ok(Value::Char(*c)),
        FormKind::BigInt(s) => parse_bigint(s),
        FormKind::BigDecimal(s) => parse_bigdecimal(s),
        FormKind::Ratio(s) => parse_ratio(s),
        FormKind::Regex(s) => {
            let r = Regex::new(s);
            match r {
                Ok(r) => Ok(Value::Pattern(GcPtr::new(r))),
                Err(e) => Err(EvalError::Runtime(e.to_string())),
            }
        }

        // ── Identifiers ───────────────────────────────────────────────────
        FormKind::Symbol(s) => eval_symbol(s, env),
        FormKind::Keyword(s) => Ok(Value::keyword(Keyword::parse(s))),
        FormKind::AutoKeyword(s) => {
            let full = format!("{}/{}", env.current_ns, s);
            Ok(Value::keyword(Keyword::parse(&full)))
        }

        // ── Collections ───────────────────────────────────────────────────
        FormKind::List(forms) => eval_list(forms, env),
        FormKind::Vector(forms) => {
            let vals: Vec<Value> = forms
                .iter()
                .map(|f| eval(f, env))
                .collect::<EvalResult<Vec<_>>>()?;
            Ok(Value::Vector(GcPtr::new(PersistentVector::from_iter(vals))))
        }
        FormKind::Map(forms) => {
            if forms.len() % 2 != 0 {
                return Err(EvalError::Runtime(
                    "map literal must have an even number of forms".into(),
                ));
            }
            // Evaluate all key-value pairs, then build the map in one shot.
            // This avoids N intermediate GcPtr allocations that the old
            // empty()+assoc+assoc chain would create.
            let mut pairs = Vec::with_capacity(forms.len() / 2);
            for pair in forms.chunks(2) {
                let k = eval(&pair[0], env)?;
                let v = eval(&pair[1], env)?;
                pairs.push((k, v));
            }
            Ok(Value::Map(MapValue::from_pairs(pairs)))
        }
        FormKind::Set(forms) => {
            // Evaluate all elements, then build the set in one shot using
            // FromIterator (uses insert_mut internally, single allocation).
            let vals: Vec<Value> = forms
                .iter()
                .map(|f| eval(f, env))
                .collect::<EvalResult<Vec<_>>>()?;
            Ok(Value::Set(SetValue::Hash(GcPtr::new(
                PersistentHashSet::from_iter(vals),
            ))))
        }

        // ── Reader macros ─────────────────────────────────────────────────
        FormKind::Quote(inner) => Ok(form_to_value(inner)),
        FormKind::SyntaxQuote(inner) => syntax_quote(inner, env),
        FormKind::Unquote(_) => Err(EvalError::Runtime("unquote outside syntax-quote".into())),
        FormKind::UnquoteSplice(_) => Err(EvalError::Runtime(
            "unquote-splice outside syntax-quote".into(),
        )),
        FormKind::Deref(inner) => {
            let v = eval(inner, env)?;
            deref_value(v)
        }
        FormKind::Var(inner) => {
            if let FormKind::Symbol(s) = &inner.kind {
                let parsed = Symbol::parse(s);
                let ns = parsed.namespace.as_deref().unwrap_or(&env.current_ns);
                env.globals
                    .lookup_var_in_ns(ns, &parsed.name)
                    .map(Value::Var)
                    .ok_or_else(|| EvalError::UnboundSymbol(s.clone()))
            } else {
                Err(EvalError::Runtime("var requires a symbol".into()))
            }
        }
        FormKind::Meta(_, form) => {
            // Ignore metadata in Phase 4; just eval the annotated form.
            eval(form, env)
        }

        // ── Dispatch ──────────────────────────────────────────────────────
        FormKind::AnonFn(body) => {
            let expanded = expand_anon_fn(body, form.span.clone());
            eval(&expanded, env)
        }
        FormKind::ReaderCond {
            splicing: _,
            clauses,
        } => eval_reader_cond(clauses, env),
        FormKind::TaggedLiteral(tag, inner) => eval_tagged_literal(tag, inner, env),
    }
}

// ── List / call dispatch ──────────────────────────────────────────────────────

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

fn eval_list(forms: &[Form], env: &mut Env) -> EvalResult {
    if forms.is_empty() {
        return Ok(Value::List(GcPtr::new(PersistentList::empty())));
    }

    // Expand reader conditionals (both splicing and non-splicing) before dispatch.
    let expanded: Vec<Form>;
    let forms: &[Form] = if forms
        .iter()
        .any(|f| matches!(f.kind, FormKind::ReaderCond { .. }))
    {
        expanded = expand_reader_conds(forms);
        if expanded.is_empty() {
            return Ok(Value::List(GcPtr::new(PersistentList::empty())));
        }
        &expanded
    } else {
        forms
    };

    // Check for special form.
    if let FormKind::Symbol(s) = &forms[0].kind
        && is_special_form(s)
    {
        return eval_special(s, &forms[1..], env);
    }

    eval_call(&forms[0], &forms[1..], env)
}

// ── Symbol resolution ─────────────────────────────────────────────────────────

fn eval_symbol(s: &str, env: &mut Env) -> EvalResult {
    // Local frames (including closed-over).
    if let Some(v) = env.lookup(s) {
        return Ok(v);
    }

    // Namespace-qualified: `ns/name`
    if s.contains('/') && !s.starts_with('/') {
        let sym = Symbol::parse(s);
        if let Some(ns_part) = &sym.namespace {
            // Resolve alias first, fall back to literal namespace name.
            let resolved: Arc<str> = env
                .globals
                .resolve_alias(&env.current_ns, ns_part)
                .unwrap_or_else(|| Arc::from(ns_part.as_ref()));
            return env
                .globals
                .lookup_in_ns(&resolved, &sym.name)
                .ok_or_else(|| EvalError::UnboundSymbol(s.to_string()));
        }
    }

    // JVM class names resolve to themselves as symbols (for instance?, catch, etc.)
    if is_jvm_class_name(s) {
        return Ok(Value::symbol(Symbol::simple(s)));
    }

    Err(EvalError::UnboundSymbol(s.to_string()))
}

/// Recognise JVM-style class names used in Clojure for `instance?`, `catch`, etc.
fn is_jvm_class_name(s: &str) -> bool {
    matches!(
        s,
        "clojure.lang.BigInt"
            | "java.math.BigDecimal"
            | "java.math.BigInteger"
            | "clojure.lang.Ratio"
            | "java.lang.Long"
            | "java.lang.Double"
            | "java.lang.String"
            | "java.lang.Boolean"
            | "java.lang.Character"
            | "java.lang.Number"
            | "clojure.lang.Symbol"
            | "clojure.lang.Keyword"
            | "clojure.lang.PersistentList"
            | "clojure.lang.PersistentVector"
            | "clojure.lang.PersistentHashMap"
            | "clojure.lang.PersistentHashSet"
            | "clojure.lang.PersistentArrayMap"
            | "clojure.lang.IFn"
            | "clojure.lang.ISeq"
            | "clojure.lang.IPending"
            | "clojure.lang.Atom"
            | "clojure.lang.Var"
            | "clojure.lang.Namespace"
            | "java.util.UUID"
            | "java.lang.Exception"
            | "java.lang.Throwable"
            | "java.lang.Error"
            | "Exception"
            | "Throwable"
            | "Error"
            | "clojure.lang.ExceptionInfo"
            | "clojure.lang.IEditableCollection"
            | "Boolean"
            | "clojure.lang.PersistentQueue"
            | "java.util.regex.Pattern"
    )
}

// ── is_special_form ───────────────────────────────────────────────────────────

pub fn is_special_form(s: &str) -> bool {
    SPECIAL_FORMS.contains(&s)
}

// ── eval_body ─────────────────────────────────────────────────────────────────

/// Dereference a value: used by `@x` reader macro and the `deref` builtin.
pub fn deref_value(v: Value) -> EvalResult {
    match v {
        Value::Atom(a) => Ok(a.get().deref()),
        Value::Var(var) => {
            crate::dynamics::deref_var(&var).ok_or_else(|| EvalError::Runtime("unbound var".into()))
        }
        Value::Volatile(vol) => Ok(vol.get().deref()),
        Value::Delay(d) => d.get().force().map_err(EvalError::Runtime),
        Value::Agent(a) => Ok(a.get().get_state()),
        Value::Promise(p) => Ok(p.get().deref_blocking()),
        Value::Future(f) => {
            let mut guard = f.get().state.lock().unwrap();
            loop {
                match &*guard {
                    FutureState::Done(v) => return Ok(v.clone()),
                    FutureState::Failed(e) => return Err(EvalError::Runtime(e.clone())),
                    FutureState::Cancelled => {
                        return Err(EvalError::Runtime("future was cancelled".into()));
                    }
                    FutureState::Running => {
                        guard = f.get().cond.wait(guard).unwrap();
                    }
                }
            }
        }
        other => Err(EvalError::Runtime(format!(
            "cannot deref {}",
            other.type_name()
        ))),
    }
}

/// Evaluate a sequence of forms and return the value of the last one.
pub fn eval_body(forms: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in forms {
        result = eval(form, env)?;
    }
    Ok(result)
}

// ── form_to_value ─────────────────────────────────────────────────────────────

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
        FormKind::BigInt(s) => parse_bigint(s).unwrap_or(Value::Nil),
        FormKind::BigDecimal(s) => parse_bigdecimal(s).unwrap_or(Value::Nil),
        FormKind::Ratio(s) => parse_ratio(s).unwrap_or(Value::Nil),

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

// ── Numeric parsing ───────────────────────────────────────────────────────────

fn parse_bigint(s: &str) -> EvalResult {
    let s = s.trim_end_matches('N');
    s.parse::<num_bigint::BigInt>()
        .map(|n| Value::BigInt(GcPtr::new(n)))
        .map_err(|e| EvalError::Runtime(format!("bad bigint: {e}")))
}

fn parse_bigdecimal(s: &str) -> EvalResult {
    let s = s.trim_end_matches('M');
    s.parse::<bigdecimal::BigDecimal>()
        .map(|d| Value::BigDecimal(GcPtr::new(d)))
        .map_err(|e| EvalError::Runtime(format!("bad bigdecimal: {e}")))
}

fn parse_ratio(s: &str) -> EvalResult {
    use num_traits::{ToPrimitive, Zero};
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return Err(EvalError::Runtime(format!("bad ratio: {s}")));
    }
    let numer: num_bigint::BigInt = parts[0]
        .parse()
        .map_err(|e| EvalError::Runtime(format!("bad ratio numer: {e}")))?;
    let denom: num_bigint::BigInt = parts[1]
        .parse()
        .map_err(|e| EvalError::Runtime(format!("bad ratio denom: {e}")))?;
    if denom.is_zero() {
        return Err(EvalError::Runtime("ratio denominator is zero".into()));
    }
    let r = num_rational::Ratio::new(numer, denom);
    if r.is_integer() {
        let n = r.to_integer();
        match n.to_i64() {
            Some(i) => Ok(Value::Long(i)),
            None => Ok(Value::BigInt(GcPtr::new(n))),
        }
    } else {
        Ok(Value::Ratio(GcPtr::new(r)))
    }
}

// ── reader cond ───────────────────────────────────────────────────────────────

fn eval_reader_cond(clauses: &[Form], env: &mut Env) -> EvalResult {
    // clauses = [kw form kw form ...]
    let mut i = 0;
    let mut default: Option<&Form> = None;
    while i + 1 < clauses.len() {
        match &clauses[i].kind {
            FormKind::Keyword(k) if k == "rust" => {
                return eval(&clauses[i + 1], env);
            }
            FormKind::Keyword(k) if k == "default" => {
                default = Some(&clauses[i + 1]);
            }
            _ => {}
        }
        i += 2;
    }
    match default {
        Some(f) => eval(f, env),
        None => Ok(Value::Nil),
    }
}

// ── tagged literals ──────────────────────────────────────────────────────────

fn eval_tagged_literal(tag: &str, inner: &Form, env: &mut Env) -> EvalResult {
    match tag {
        "uuid" => {
            let val = eval(inner, env)?;
            match &val {
                Value::Str(s) => {
                    let uuid = uuid::Uuid::parse_str(s.get())
                        .map_err(|e| EvalError::Runtime(format!("invalid UUID: {e}")))?;
                    Ok(Value::Uuid(uuid.as_u128()))
                }
                _ => Err(EvalError::Runtime(format!(
                    "#uuid expects a string, got {}",
                    val.type_name()
                ))),
            }
        }
        "inst" => {
            // TODO: implement #inst for date/time literals
            let val = eval(inner, env)?;
            Ok(val)
        }
        _ => Err(EvalError::Runtime(format!(
            "unknown tagged literal: #{tag}"
        ))),
    }
}

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
            FormKind::Symbol(s) if s == "%" || s == "%1" => {
                if *max_pos < 1 {
                    *max_pos = 1;
                }
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

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GlobalEnv, standard_env};
    use std::sync::Arc;

    fn make_env() -> (Arc<GlobalEnv>, Env) {
        let globals = standard_env();
        let env = Env::new(globals.clone(), "user");
        (globals, env)
    }

    fn eval_str(src: &str) -> EvalResult {
        let (_, mut env) = make_env();
        eval_src(src, &mut env)
    }

    fn eval_src(src: &str, env: &mut Env) -> EvalResult {
        let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
        let forms = parser.parse_all().map_err(EvalError::Read)?;
        let mut result = Value::Nil;
        for form in forms {
            result = eval(&form, env)?;
        }
        Ok(result)
    }

    fn long(n: i64) -> Value {
        Value::Long(n)
    }
    fn bool_v(b: bool) -> Value {
        Value::Bool(b)
    }

    // ── Atoms ─────────────────────────────────────────────────────────────

    #[test]
    fn test_literal_int() {
        assert_eq!(eval_str("42").unwrap(), long(42));
    }

    #[test]
    fn test_literal_string() {
        assert!(matches!(eval_str("\"hello\"").unwrap(), Value::Str(_)));
    }

    #[test]
    fn test_literal_nil() {
        assert_eq!(eval_str("nil").unwrap(), Value::Nil);
    }

    #[test]
    fn test_literal_true() {
        assert_eq!(eval_str("true").unwrap(), bool_v(true));
    }

    #[test]
    fn test_literal_false() {
        assert_eq!(eval_str("false").unwrap(), bool_v(false));
    }

    // ── Arithmetic ────────────────────────────────────────────────────────

    #[test]
    fn test_add() {
        assert_eq!(eval_str("(+ 1 2)").unwrap(), long(3));
    }

    #[test]
    fn test_mul() {
        assert_eq!(eval_str("(* 2 3)").unwrap(), long(6));
    }

    #[test]
    fn test_div_exact() {
        assert_eq!(eval_str("(/ 10 2)").unwrap(), long(5));
    }

    #[test]
    fn test_sub() {
        assert_eq!(eval_str("(- 10 3)").unwrap(), long(7));
    }

    // ── let ───────────────────────────────────────────────────────────────

    #[test]
    fn test_let_simple() {
        assert_eq!(eval_str("(let* [x 1 y 2] (+ x y))").unwrap(), long(3));
    }

    #[test]
    fn test_let_shadowing() {
        assert_eq!(eval_str("(let* [x 1] (let* [x 10] x))").unwrap(), long(10));
    }

    // ── fn + call ─────────────────────────────────────────────────────────

    #[test]
    fn test_fn_call() {
        assert_eq!(eval_str("((fn* [x] (* x x)) 5)").unwrap(), long(25));
    }

    #[test]
    fn test_closure_capture() {
        assert_eq!(
            eval_str("(let* [n 3] ((fn* [x] (+ x n)) 4))").unwrap(),
            long(7)
        );
    }

    #[test]
    fn test_multi_arity_fn() {
        assert_eq!(
            eval_str("((fn* ([x] x) ([x y] (+ x y))) 1 2)").unwrap(),
            long(3)
        );
    }

    // ── recur / loop ──────────────────────────────────────────────────────

    #[test]
    fn test_loop_recur() {
        let result =
            eval_str("(loop* [i 0 acc 0] (if (= i 5) acc (recur (inc i) (+ acc i))))").unwrap();
        assert_eq!(result, long(10));
    }

    // ── def / defn ────────────────────────────────────────────────────────

    #[test]
    fn test_def() {
        let (_, mut env) = make_env();
        eval_src("(def x 42)", &mut env).unwrap();
        assert_eq!(eval_src("x", &mut env).unwrap(), long(42));
    }

    #[test]
    fn test_defn() {
        let (_, mut env) = make_env();
        eval_src("(defn square [x] (* x x))", &mut env).unwrap();
        assert_eq!(eval_src("(square 7)", &mut env).unwrap(), long(49));
    }

    // ── if ────────────────────────────────────────────────────────────────

    #[test]
    fn test_if_truthy() {
        assert_eq!(eval_str("(if true 1 2)").unwrap(), long(1));
    }

    #[test]
    fn test_if_falsy() {
        assert_eq!(eval_str("(if false 1 2)").unwrap(), long(2));
    }

    #[test]
    fn test_if_nil_branch() {
        assert_eq!(eval_str("(if false 1)").unwrap(), Value::Nil);
    }

    // ── do ────────────────────────────────────────────────────────────────

    #[test]
    fn test_do() {
        assert_eq!(eval_str("(do 1 2 3)").unwrap(), long(3));
    }

    // ── quote ─────────────────────────────────────────────────────────────

    #[test]
    fn test_quote_list() {
        let v = eval_str("'(1 2 3)").unwrap();
        assert!(matches!(v, Value::List(_)));
    }

    // ── keyword lookup ────────────────────────────────────────────────────

    #[test]
    fn test_keyword_lookup() {
        assert_eq!(eval_str("(:a {:a 1})").unwrap(), long(1));
    }

    #[test]
    fn test_keyword_lookup_missing() {
        assert_eq!(eval_str("(:b {:a 1})").unwrap(), Value::Nil);
    }

    // ── Map / Vector / Set literals ───────────────────────────────────────

    #[test]
    fn test_map_literal() {
        let v = eval_str("{:a 1 :b 2}").unwrap();
        assert!(matches!(v, Value::Map(_)));
        if let Value::Map(m) = &v {
            assert_eq!(m.count(), 2);
        }
    }

    #[test]
    fn test_vector_literal() {
        let v = eval_str("[1 2 3]").unwrap();
        assert!(matches!(v, Value::Vector(_)));
    }

    #[test]
    fn test_set_literal() {
        let v = eval_str("#{1 2 3}").unwrap();
        assert!(matches!(v, Value::Set(_)));
    }

    // ── set! ──────────────────────────────────────────────────────────────

    #[test]
    fn test_set_bang() {
        let (_, mut env) = make_env();
        eval_src("(def x 1)", &mut env).unwrap();
        eval_src("(set! x 99)", &mut env).unwrap();
        assert_eq!(eval_src("x", &mut env).unwrap(), long(99));
    }

    // ── throw / try / catch ───────────────────────────────────────────────

    #[test]
    fn test_throw_catch() {
        let v = eval_str("(try (throw (ex-info \"oops\" {})) (catch Exception e (ex-message e)))")
            .unwrap();
        assert!(matches!(v, Value::Str(_)));
    }

    #[test]
    fn test_try_no_throw() {
        assert_eq!(eval_str("(try 42)").unwrap(), long(42));
    }

    // ── destructuring ─────────────────────────────────────────────────────

    #[test]
    fn test_sequential_destructure() {
        assert_eq!(eval_str("(let* [[a b] [1 2]] (+ a b))").unwrap(), long(3));
    }

    #[test]
    fn test_rest_destructure() {
        let v = eval_str("(let* [[h & t] [1 2 3]] t)").unwrap();
        assert!(matches!(v, Value::List(_)));
        if let Value::List(l) = &v {
            assert_eq!(l.get().count(), 2);
        }
    }

    // ── defmacro ──────────────────────────────────────────────────────────

    #[test]
    fn test_defmacro() {
        let (_, mut env) = make_env();
        eval_src("(defmacro my-if [t a b] (list 'if t a b))", &mut env).unwrap();
        assert_eq!(eval_src("(my-if true 1 2)", &mut env).unwrap(), long(1));
        assert_eq!(eval_src("(my-if false 1 2)", &mut env).unwrap(), long(2));
    }

    // ── syntax-quote ──────────────────────────────────────────────────────

    #[test]
    fn test_syntax_quote_basic() {
        let (_, mut env) = make_env();
        eval_src("(def b 2)", &mut env).unwrap();
        let v = eval_src("`(a ~b)", &mut env).unwrap();
        assert!(matches!(v, Value::List(_)));
        if let Value::List(l) = &v {
            // Should be (user/a 2)
            let items: Vec<_> = l.get().iter().cloned().collect();
            assert_eq!(items.len(), 2);
            assert_eq!(items[1], long(2));
        }
    }

    // ── reader conditionals ───────────────────────────────────────────────

    #[test]
    fn test_reader_cond_rust() {
        // :rust branch selected.
        assert_eq!(eval_str("#?(:rust 1 :clj 2)").unwrap(), long(1));
    }

    #[test]
    fn test_reader_cond_default() {
        // No :rust; fall through to :default.
        assert_eq!(eval_str("#?(:clj 2 :default 99)").unwrap(), long(99));
    }

    // ── Error cases ───────────────────────────────────────────────────────

    #[test]
    fn test_unbound_symbol() {
        let r = eval_str("undefined-var-xyz");
        assert!(matches!(r, Err(EvalError::UnboundSymbol(_))));
    }

    #[test]
    fn test_wrong_arity() {
        let (_, mut env) = make_env();
        eval_src("(defn one-arg [x] x)", &mut env).unwrap();
        let r = eval_src("(one-arg 1 2)", &mut env);
        assert!(matches!(r, Err(EvalError::Arity { .. })));
    }

    #[test]
    fn test_not_callable() {
        let r = eval_str("(42 1 2)");
        assert!(matches!(r, Err(EvalError::NotCallable(_))));
    }

    // ── Higher-order functions (bootstrap) ────────────────────────────────

    #[test]
    fn test_map_fn() {
        assert_eq!(
            eval_str("(vec (map inc [1 2 3]))").unwrap(),
            eval_str("[2 3 4]").unwrap()
        );
    }

    #[test]
    fn test_filter_fn() {
        assert_eq!(
            eval_str("(vec (filter odd? [1 2 3 4 5]))").unwrap(),
            eval_str("[1 3 5]").unwrap()
        );
    }

    #[test]
    fn test_reduce_fn() {
        assert_eq!(eval_str("(reduce + [1 2 3 4 5])").unwrap(), long(15));
    }

    #[test]
    fn test_apply_fn() {
        assert_eq!(eval_str("(apply + [1 2 3])").unwrap(), long(6));
    }

    #[test]
    fn test_atom_ops() {
        let (_, mut env) = make_env();
        eval_src("(def a (atom 0))", &mut env).unwrap();
        eval_src("(swap! a inc)", &mut env).unwrap();
        assert_eq!(eval_src("(deref a)", &mut env).unwrap(), long(1));
    }

    #[test]
    fn test_when_macro() {
        assert_eq!(eval_str("(when true 42)").unwrap(), long(42));
        assert_eq!(eval_str("(when false 42)").unwrap(), Value::Nil);
    }

    #[test]
    fn test_cond_macro() {
        assert_eq!(eval_str("(cond false 1 true 2)").unwrap(), long(2));
    }

    #[test]
    fn test_and_or() {
        assert_eq!(eval_str("(and 1 2 3)").unwrap(), long(3));
        assert_eq!(eval_str("(and 1 false 3)").unwrap(), bool_v(false));
        assert_eq!(eval_str("(or false nil 42)").unwrap(), long(42));
        assert_eq!(eval_str("(or false nil)").unwrap(), Value::Nil);
    }

    // ── Phase 5: Lazy sequences ───────────────────────────────────────────

    #[test]
    fn test_lazy_range() {
        assert_eq!(
            eval_str("(= (into [] (take 5 (range))) [0 1 2 3 4])").unwrap(),
            bool_v(true)
        );
    }

    #[test]
    fn test_lazy_range_bounded() {
        assert_eq!(
            eval_str("(= (into [] (range 3)) [0 1 2])").unwrap(),
            bool_v(true)
        );
    }

    #[test]
    fn test_lazy_iterate() {
        assert_eq!(
            eval_str("(= (into [] (take 3 (iterate inc 0))) [0 1 2])").unwrap(),
            bool_v(true)
        );
    }

    #[test]
    fn test_lazy_repeat() {
        assert_eq!(
            eval_str("(= (into [] (take 3 (repeat :x))) [:x :x :x])").unwrap(),
            bool_v(true)
        );
    }

    #[test]
    fn test_lazy_cycle() {
        assert_eq!(
            eval_str("(= (into [] (take 5 (cycle [1 2]))) [1 2 1 2 1])").unwrap(),
            bool_v(true)
        );
    }

    // ── Phase 5: Associative destructuring ───────────────────────────────

    #[test]
    fn test_assoc_destructure() {
        assert_eq!(
            eval_str("(let [{:keys [a b]} {:a 1 :b 2}] (+ a b))").unwrap(),
            long(3)
        );
    }

    #[test]
    fn test_assoc_destructure_or() {
        assert_eq!(
            eval_str("(let [{:keys [a b] :or {b 99}} {:a 1}] b)").unwrap(),
            long(99)
        );
    }

    // ── Phase 5: letfn ───────────────────────────────────────────────────

    #[test]
    fn test_letfn() {
        assert_eq!(
            eval_str("(letfn [(fact [n] (if (= n 0) 1 (* n (fact (dec n)))))] (fact 5))").unwrap(),
            long(120)
        );
    }

    // ── Phase 5: namespace ops ────────────────────────────────────────────

    #[test]
    fn test_in_ns() {
        let (_, mut env) = make_env();
        eval_src("(in-ns 'mytest)", &mut env).unwrap();
        assert_eq!(env.current_ns.as_ref(), "mytest");
        eval_src("(in-ns 'user)", &mut env).unwrap();
        assert_eq!(env.current_ns.as_ref(), "user");
    }

    // ── Phase 5: spit / slurp ─────────────────────────────────────────────

    #[test]
    fn test_spit_slurp() {
        let path = std::env::temp_dir().join("cljrs_test_spit_slurp.txt");
        let path_str = path.to_str().unwrap();
        let src = format!(
            r#"(do (spit "{}" "hello clojurust") (slurp "{}"))"#,
            path_str, path_str
        );
        let result = eval_str(&src).unwrap();
        if let Value::Str(s) = result {
            assert_eq!(s.get().as_str(), "hello clojurust");
        } else {
            panic!("expected string result from slurp");
        }
        let _ = std::fs::remove_file(path);
    }

    // ── Phase 5: update-in ───────────────────────────────────────────────

    #[test]
    fn test_update_in() {
        assert_eq!(
            eval_str("(= (update-in {:a {:b 1}} [:a :b] inc) {:a {:b 2}})").unwrap(),
            bool_v(true)
        );
    }

    // ── Phase 5: if-let / when-let ────────────────────────────────────────

    #[test]
    fn test_if_let_truthy() {
        assert_eq!(eval_str("(if-let [x 42] x :nope)").unwrap(), long(42));
    }

    #[test]
    fn test_if_let_falsy() {
        assert_eq!(
            eval_str("(if-let [x nil] x :nope)").unwrap(),
            eval_str(":nope").unwrap()
        );
    }

    #[test]
    fn test_when_let_truthy() {
        assert_eq!(eval_str("(when-let [x 7] (* x 2))").unwrap(), long(14));
    }

    #[test]
    fn test_when_let_falsy() {
        assert_eq!(eval_str("(when-let [x nil] 99)").unwrap(), Value::Nil);
    }

    // ── Phase 5: math functions ───────────────────────────────────────────

    #[test]
    fn test_math_trig() {
        // sin(0) = 0, cos(0) = 1
        assert_eq!(eval_str("(Math/sin 0)").unwrap(), Value::Double(0.0));
        assert_eq!(eval_str("(Math/cos 0)").unwrap(), Value::Double(1.0));
    }

    #[test]
    fn test_math_constants() {
        assert!(
            matches!(eval_str("Math/PI").unwrap(), Value::Double(v) if (v - std::f64::consts::PI).abs() < 1e-10)
        );
        assert!(
            matches!(eval_str("Math/E").unwrap(), Value::Double(v) if (v - std::f64::consts::E).abs() < 1e-10)
        );
    }

    #[test]
    fn test_math_log_exp() {
        // exp(0) = 1, log(1) = 0
        assert_eq!(eval_str("(Math/exp 0)").unwrap(), Value::Double(1.0));
        assert_eq!(eval_str("(Math/log 1)").unwrap(), Value::Double(0.0));
    }

    // ── Phase 6: Protocols & Multimethods ─────────────────────────────────

    #[test]
    fn test_defprotocol() {
        // Defining a protocol creates a callable ProtocolFn that errors without impl.
        let result = eval_str(
            r#"
            (defprotocol Greet
              (greet [this]))
            (greet "hello")
            "#,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No implementation"), "got: {msg}");
    }

    #[test]
    fn test_extend_type() {
        let result = eval_str(
            r#"
            (defprotocol Greet
              (greet [this]))
            (extend-type String
              Greet
              (greet [this] (str "Hello, " this "!")))
            (greet "world")
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::string("Hello, world!"));
    }

    #[test]
    fn test_protocol_dispatch() {
        let result = eval_str(
            r#"
            (defprotocol Describable
              (describe [this]))
            (extend-type String
              Describable
              (describe [this] (str "string:" this)))
            (extend-type Long
              Describable
              (describe [this] (str "long:" this)))
            [(describe "hi") (describe 42)]
            "#,
        )
        .unwrap();
        assert!(matches!(result, Value::Vector(_)));
        let s = format!("{}", result);
        assert!(s.contains("string:hi"), "got: {s}");
        assert!(s.contains("long:42"), "got: {s}");
    }

    #[test]
    fn test_extend_protocol() {
        let result = eval_str(
            r#"
            (defprotocol Showable
              (show [this]))
            (extend-protocol Showable
              String
              (show [this] (str "S:" this))
              Long
              (show [this] (str "L:" this)))
            [(show "x") (show 7)]
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        assert!(s.contains("S:x"), "got: {s}");
        assert!(s.contains("L:7"), "got: {s}");
    }

    #[test]
    fn test_satisfies() {
        let result = eval_str(
            r#"
            (defprotocol Animal
              (speak [this]))
            (extend-type String
              Animal
              (speak [this] this))
            [(satisfies? Animal "dog") (satisfies? Animal 42)]
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        assert!(s.contains("true"), "got: {s}");
        assert!(s.contains("false"), "got: {s}");
    }

    #[test]
    fn test_defmulti_defmethod() {
        // Note: fn param destructuring not yet supported; use explicit map lookups.
        let result = eval_str(
            r#"
            (defmulti area :shape)
            (defmethod area :circle [m] (* 3 (:r m) (:r m)))
            (defmethod area :rectangle [m] (* (:w m) (:h m)))
            [(area {:shape :circle :r 2}) (area {:shape :rectangle :w 3 :h 4})]
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        // circle: 3*2*2=12, rectangle: 3*4=12
        assert!(s.contains("12"), "got: {s}");
    }

    #[test]
    fn test_default_dispatch() {
        let result = eval_str(
            r#"
            (defmulti classify :kind)
            (defmethod classify :default [x] :unknown)
            (defmethod classify :cat [x] :meow)
            [(classify {:kind :dog}) (classify {:kind :cat})]
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        assert!(s.contains(":unknown"), "got: {s}");
        assert!(s.contains(":meow"), "got: {s}");
    }

    #[test]
    fn test_prefer_method() {
        // prefer-method shouldn't error; just records preference
        let result = eval_str(
            r#"
            (defmulti foo identity)
            (defmethod foo :a [x] 1)
            (prefer-method foo :a :b)
            (foo :a)
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::Long(1));
    }

    #[test]
    fn test_remove_method() {
        let result = eval_str(
            r#"
            (defmulti bar identity)
            (defmethod bar :x [_] 99)
            (remove-method bar :x)
            (bar :x)
            "#,
        );
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("No method"), "got: {msg}");
    }

    // ── Phase 7: Concurrency primitives ──────────────────────────────────────

    #[test]
    fn test_compare_and_set() {
        let result = eval_str(
            r#"
            (let [a (atom 10)]
              [(compare-and-set! a 10 20)   ; succeeds: 10 == 10
               (compare-and-set! a 10 30)   ; fails:    20 != 10
               @a])
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        assert!(s.contains("true"), "got: {s}");
        assert!(s.contains("false"), "got: {s}");
        assert!(s.contains("20"), "got: {s}");
    }

    #[test]
    fn test_volatile() {
        let result = eval_str(
            r#"
            (let [v (volatile! 1)]
              (vreset! v 2)
              (vswap! v + 10)
              @v)
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::Long(12));
    }

    #[test]
    fn test_delay() {
        // Body should not be evaluated until forced.
        let result = eval_str(
            r#"
            (let [calls (atom 0)
                  d (delay (swap! calls inc) 42)]
              [@calls (force d) @calls (force d) @calls])
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        // calls starts at 0, force evaluates body once (returns 42), second force uses cache
        // s = [0 42 1 42 1]
        assert!(s.starts_with("[0 42 1 42 1]"), "got: {s}");
    }

    #[test]
    fn test_realized() {
        let result = eval_str(
            r#"
            (let [d (delay 99)]
              [(realized? d) (force d) (realized? d)])
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        assert!(s.starts_with("[false 99 true]"), "got: {s}");
    }

    #[test]
    fn test_promise() {
        let result = eval_str(
            r#"
            (let [p (promise)]
              (deliver p 42)
              (deliver p 99)  ; second deliver is ignored
              @p)
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::Long(42));
    }

    #[test]
    fn test_future() {
        let result = eval_str(
            r#"
            (let [f (future (+ 1 2))]
              @f)
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::Long(3));
    }

    #[test]
    fn test_agent_send() {
        let result = eval_str(
            r#"
            (let [a (agent 0)]
              (send a + 1)
              (send a + 2)
              (await a)
              @a)
            "#,
        )
        .unwrap();
        assert_eq!(result, Value::Long(3));
    }

    #[test]
    fn test_agent_error_restart() {
        let result = eval_str(
            r#"
            (let [a (agent 10)]
              (send a (fn [_] (throw (ex-info "boom" {}))))
              (await a)
              (let [err (agent-error a)]
                (restart-agent a 99)
                [err @a]))
            "#,
        )
        .unwrap();
        let s = format!("{}", result);
        // err should be a string containing "boom", @a should be 99
        assert!(s.contains("boom"), "got: {s}");
        assert!(s.contains("99"), "got: {s}");
    }

    #[test]
    fn test_defrecord_basic() {
        // Constructor and field access via keyword.
        let result = eval_str(
            r#"
            (defrecord Point [x y])
            (let [p (->Point 3 4)]
              [(:x p) (:y p)])
            "#,
        )
        .unwrap();
        assert_eq!(result.to_string(), "[3 4]");
    }

    #[test]
    fn test_defrecord_map_constructor() {
        let result = eval_str(
            r#"
            (defrecord Color [r g b])
            (let [c (map->Color {:r 255 :g 128 :b 0})]
              [(:r c) (:g c) (:b c)])
            "#,
        )
        .unwrap();
        assert_eq!(result.to_string(), "[255 128 0]");
    }

    #[test]
    fn test_defrecord_assoc() {
        // assoc on a record returns a new record of the same type.
        let result = eval_str(
            r#"
            (defrecord Pt [x y])
            (let [p (->Pt 1 2)
                  q (assoc p :x 99)]
              [(:x q) (:y q) (record? q)])
            "#,
        )
        .unwrap();
        assert_eq!(result.to_string(), "[99 2 true]");
    }

    #[test]
    fn test_defrecord_with_protocol() {
        let result = eval_str(
            r#"
            (defprotocol IShape
              (area [this]))
            (defrecord Circle [radius]
              IShape
              (area [this] (* 3 (:radius this) (:radius this))))
            (let [c (->Circle 5)]
              (area c))
            "#,
        )
        .unwrap();
        assert_eq!(result, cljrs_value::Value::Long(75));
    }

    #[test]
    fn test_instance_q() {
        let result = eval_str(
            r#"
            (defrecord Dog [name])
            (let [d (->Dog "Rex")]
              [(instance? Dog d) (instance? Dog 42)])
            "#,
        )
        .unwrap();
        assert_eq!(result.to_string(), "[true false]");
    }

    #[test]
    fn test_reify_basic() {
        let result = eval_str(
            r#"
            (defprotocol IGreet
              (greet [this name]))
            (let [greeter (reify IGreet
                            (greet [this name] (str "Hello, " name "!")))]
              (greet greeter "World"))
            "#,
        )
        .unwrap();
        assert_eq!(result.to_string(), "\"Hello, World!\"");
    }

    // ── require / load-file ───────────────────────────────────────────────

    fn temp_ns_dir(test_name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cljrs_test_{test_name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn make_env_with_paths(paths: Vec<std::path::PathBuf>) -> (Arc<GlobalEnv>, Env) {
        use crate::standard_env_with_paths;
        let globals = standard_env_with_paths(paths);
        let env = Env::new(globals.clone(), "user");
        (globals, env)
    }

    #[test]
    fn test_require_as() {
        let dir = temp_ns_dir("require_as");
        std::fs::write(
            dir.join("mylib.cljrs"),
            "(ns mylib) (defn greet [n] (str \"hello \" n))",
        )
        .unwrap();
        let (_, mut env) = make_env_with_paths(vec![dir]);
        let result = eval_src("(require '[mylib :as ml]) (ml/greet \"world\")", &mut env).unwrap();
        assert_eq!(result.to_string(), "\"hello world\"");
    }

    #[test]
    fn test_require_refer() {
        let dir = temp_ns_dir("require_refer");
        std::fs::write(
            dir.join("myutil.cljrs"),
            "(ns myutil) (defn twice [x] (* 2 x))",
        )
        .unwrap();
        let (_, mut env) = make_env_with_paths(vec![dir]);
        let result = eval_src("(require '[myutil :refer [twice]]) (twice 21)", &mut env).unwrap();
        assert_eq!(result, Value::Long(42));
    }

    #[test]
    fn test_require_refer_all() {
        let dir = temp_ns_dir("require_refer_all");
        std::fs::write(
            dir.join("mymath.cljrs"),
            "(ns mymath) (defn square [x] (* x x))",
        )
        .unwrap();
        let (_, mut env) = make_env_with_paths(vec![dir]);
        let result = eval_src("(require '[mymath :refer :all]) (square 7)", &mut env).unwrap();
        assert_eq!(result, Value::Long(49));
    }

    #[test]
    fn test_ns_require_clause() {
        let dir = temp_ns_dir("ns_require");
        std::fs::write(
            dir.join("greeter.cljrs"),
            "(ns greeter) (defn hi [n] (str \"Hi \" n))",
        )
        .unwrap();
        let (_, mut env) = make_env_with_paths(vec![dir]);
        let result = eval_src(
            "(ns myapp (:require [greeter :as g])) (g/hi \"Alice\")",
            &mut env,
        )
        .unwrap();
        assert_eq!(result.to_string(), "\"Hi Alice\"");
    }

    #[test]
    fn test_require_idempotent() {
        let dir = temp_ns_dir("require_idempotent");
        // File has a side effect tracked via an atom
        std::fs::write(
            dir.join("counter.cljrs"),
            "(ns counter) (def loaded-count (atom 0)) (swap! loaded-count inc)",
        )
        .unwrap();
        let (globals, mut env) = make_env_with_paths(vec![dir]);
        eval_src("(require 'counter)", &mut env).unwrap();
        eval_src("(require 'counter)", &mut env).unwrap();
        // The atom should have been incremented only once.
        let count = globals.lookup_in_ns("counter", "loaded-count").unwrap();
        if let Value::Atom(a) = count {
            assert_eq!(a.get().deref(), Value::Long(1));
        } else {
            panic!("expected atom");
        }
    }

    #[test]
    fn test_require_not_found() {
        let (_, mut env) = make_env_with_paths(vec![]);
        let err = eval_src("(require 'nonexistent.ns)", &mut env).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("nonexistent.ns"), "unexpected error: {msg}");
    }

    #[test]
    fn test_require_circular() {
        let dir = temp_ns_dir("require_circular");
        // a requires b, b requires a
        std::fs::write(dir.join("cira.cljrs"), "(ns cira (:require [cirb]))").unwrap();
        std::fs::write(dir.join("cirb.cljrs"), "(ns cirb (:require [cira]))").unwrap();
        let (_, mut env) = make_env_with_paths(vec![dir]);
        let err = eval_src("(require 'cira)", &mut env).unwrap_err();
        let msg = format!("{err:?}");
        assert!(
            msg.contains("circular"),
            "expected circular error, got: {msg}"
        );
    }

    #[test]
    fn test_load_file() {
        let dir = temp_ns_dir("load_file");
        let path = dir.join("script.cljrs");
        std::fs::write(&path, "(+ 1 2)").unwrap();
        let (_, mut env) = make_env_with_paths(vec![]);
        let result = eval_src(&format!("(load-file \"{}\")", path.display()), &mut env).unwrap();
        assert_eq!(result, Value::Long(3));
    }

    // ── *ns* and namespace reflection ─────────────────────────────────────────

    #[test]
    fn test_star_ns_initial() {
        // After standard_env(), *ns* should be the user namespace.
        let (_, mut env) = make_env();
        let v = eval_src("*ns*", &mut env).unwrap();
        match v {
            Value::Namespace(ns) => assert_eq!(ns.get().name.as_ref(), "user"),
            other => panic!("expected Namespace, got {other:?}"),
        }
    }

    #[test]
    fn test_star_ns_after_in_ns() {
        let (_, mut env) = make_env();
        eval_src("(in-ns 'myns)", &mut env).unwrap();
        let v = eval_src("*ns*", &mut env).unwrap();
        match v {
            Value::Namespace(ns) => assert_eq!(ns.get().name.as_ref(), "myns"),
            other => panic!("expected Namespace, got {other:?}"),
        }
    }

    #[test]
    fn test_star_ns_after_ns_form() {
        let (_, mut env) = make_env();
        eval_src("(ns mytest.ns)", &mut env).unwrap();
        let v = eval_src("*ns*", &mut env).unwrap();
        match v {
            Value::Namespace(ns) => assert_eq!(ns.get().name.as_ref(), "mytest.ns"),
            other => panic!("expected Namespace, got {other:?}"),
        }
    }

    #[test]
    fn test_ns_name() {
        let (_, mut env) = make_env();
        let v = eval_src("(ns-name *ns*)", &mut env).unwrap();
        match v {
            Value::Symbol(s) => assert_eq!(s.get().name.as_ref(), "user"),
            other => panic!("expected Symbol, got {other:?}"),
        }
    }

    #[test]
    fn test_find_ns() {
        let (_, mut env) = make_env();
        // known ns
        let v = eval_src("(find-ns 'user)", &mut env).unwrap();
        assert!(matches!(v, Value::Namespace(_)));
        // unknown ns
        let v2 = eval_src("(find-ns 'nonexistent)", &mut env).unwrap();
        assert_eq!(v2, Value::Nil);
    }

    #[test]
    fn test_all_ns() {
        let (_, mut env) = make_env();
        let v = eval_src("(all-ns)", &mut env).unwrap();
        // Should be a list containing at least user and clojure.core
        let names: Vec<String> = match &v {
            Value::List(l) => l
                .get()
                .iter()
                .filter_map(|ns| match ns {
                    Value::Namespace(n) => Some(n.get().name.as_ref().to_string()),
                    _ => None,
                })
                .collect(),
            other => panic!("expected list, got {other:?}"),
        };
        assert!(names.contains(&"user".to_string()));
        assert!(names.contains(&"clojure.core".to_string()));
    }

    #[test]
    fn test_ns_interns() {
        let (_, mut env) = make_env();
        eval_src("(def my-test-var 42)", &mut env).unwrap();
        let v = eval_src("(ns-interns *ns*)", &mut env).unwrap();
        let Value::Map(m) = v else {
            panic!("expected map")
        };
        // The map should contain 'my-test-var
        let sym = Value::Symbol(cljrs_gc::GcPtr::new(cljrs_value::Symbol {
            namespace: None,
            name: Arc::from("my-test-var"),
        }));
        assert!(m.get(&sym).is_some());
    }

    #[test]
    fn test_create_ns() {
        let (_, mut env) = make_env();
        let v = eval_src("(create-ns 'fresh.ns)", &mut env).unwrap();
        match v {
            Value::Namespace(ns) => assert_eq!(ns.get().name.as_ref(), "fresh.ns"),
            other => panic!("expected Namespace, got {other:?}"),
        }
        // find-ns should now find it
        let v2 = eval_src("(find-ns 'fresh.ns)", &mut env).unwrap();
        assert!(matches!(v2, Value::Namespace(_)));
    }

    // ── Dynamic variables (Phase 9) ───────────────────────────────────────────

    #[test]
    fn test_dynamic_var_basic() {
        let (globals, mut env) = make_env();
        let result = eval_src("(def ^:dynamic *x* 10) (binding [*x* 42] *x*)", &mut env).unwrap();
        assert_eq!(result, Value::Long(42));
        // verify root is still bound
        let root = globals.lookup_in_ns("user", "*x*");
        assert_eq!(root, Some(Value::Long(10)));
    }

    #[test]
    fn test_dynamic_var_restore() {
        let (globals, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 10)", &mut env).unwrap();
        eval_src("(binding [*x* 42] *x*)", &mut env).unwrap();
        // After binding block, value restored to root
        let val = eval_src("*x*", &mut env).unwrap();
        assert_eq!(val, Value::Long(10));
    }

    #[test]
    fn test_dynamic_var_nested() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 1)", &mut env).unwrap();
        let result = eval_src("(binding [*x* 2] (binding [*x* 3] *x*))", &mut env).unwrap();
        assert_eq!(result, Value::Long(3));
        // After both blocks
        let val = eval_src("*x*", &mut env).unwrap();
        assert_eq!(val, Value::Long(1));
    }

    #[test]
    fn test_dynamic_var_unaffected() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 10)", &mut env).unwrap();
        eval_src("(def y 99)", &mut env).unwrap();
        eval_src("(binding [*x* 42] *x*)", &mut env).unwrap();
        // non-dynamic var y is unchanged
        let val = eval_src("y", &mut env).unwrap();
        assert_eq!(val, Value::Long(99));
    }

    #[test]
    fn test_binding_conveyance() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 10)", &mut env).unwrap();
        let result = eval_src("(binding [*x* 42] @(future *x*))", &mut env).unwrap();
        assert_eq!(result, Value::Long(42));
    }

    #[test]
    fn test_var_set_in_binding() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 10)", &mut env).unwrap();
        // set! inside binding sets thread-local
        let inside = eval_src("(binding [*x* 1] (set! *x* 2) *x*)", &mut env).unwrap();
        assert_eq!(inside, Value::Long(2));
        // root still 10
        let root = eval_src("*x*", &mut env).unwrap();
        assert_eq!(root, Value::Long(10));
    }

    #[test]
    fn test_with_bindings_star() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 10)", &mut env).unwrap();
        let result = eval_src("(with-bindings* {#'*x* 99} (fn [] *x*))", &mut env).unwrap();
        assert_eq!(result, Value::Long(99));
    }

    #[test]
    fn test_meta_on_var() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 1)", &mut env).unwrap();
        let m = eval_src("(meta #'*x*)", &mut env).unwrap();
        // meta should be {:dynamic true}
        if let Value::Map(mv) = &m {
            let kw = Value::keyword(cljrs_value::Keyword::parse("dynamic"));
            assert_eq!(mv.get(&kw), Some(Value::Bool(true)));
        } else {
            panic!("expected map, got {m:?}");
        }
    }

    #[test]
    fn test_bound_pred() {
        let (_, mut env) = make_env();
        eval_src("(def ^:dynamic *x* 1)", &mut env).unwrap();
        let t = eval_src("(bound? #'*x*)", &mut env).unwrap();
        assert_eq!(t, Value::Bool(true));
    }

    #[test]
    fn test_alter_var_root() {
        let (_, mut env) = make_env();
        eval_src("(def x 1)", &mut env).unwrap();
        eval_src("(alter-var-root #'x inc)", &mut env).unwrap();
        let val = eval_src("x", &mut env).unwrap();
        assert_eq!(val, Value::Long(2));
    }

    // ── clojure.test ─────────────────────────────────────────────────────────

    #[test]
    fn test_clojure_test_is_pass() {
        // (is expr) returns true on a passing assertion.
        let (_, mut env) = make_env();
        eval_src(
            "(require '[clojure.test :refer [is deftest run-tests]])",
            &mut env,
        )
        .unwrap();
        let v = eval_src("(is (= 1 1))", &mut env).unwrap();
        assert_eq!(v, Value::Bool(true));
    }

    #[test]
    fn test_clojure_test_is_fail() {
        // (is expr) returns false on a failing assertion.
        let (_, mut env) = make_env();
        eval_src("(require '[clojure.test :refer [is]])", &mut env).unwrap();
        let v = eval_src("(is (= 1 2))", &mut env).unwrap();
        assert_eq!(v, Value::Bool(false));
    }

    #[test]
    fn test_clojure_test_is_catch_error() {
        // (is expr) catches runtime errors and returns false.
        let (_, mut env) = make_env();
        eval_src("(require '[clojure.test :refer [is]])", &mut env).unwrap();
        let v = eval_src("(is (/ 1 0))", &mut env).unwrap();
        assert_eq!(v, Value::Bool(false));
    }

    #[test]
    fn test_clojure_test_deftest_and_run() {
        // deftest + run-tests smoke test: counters reflect pass/fail.
        let (_, mut env) = make_env();
        eval_src(
            "(require '[clojure.test :refer [deftest is run-tests]])",
            &mut env,
        )
        .unwrap();
        eval_src("(deftest my-passing-test (is (= 1 1)))", &mut env).unwrap();
        eval_src("(deftest my-failing-test (is (= 1 2)))", &mut env).unwrap();
        let counters = eval_src("(run-tests)", &mut env).unwrap();
        // Should have run 2 tests, 1 pass, 1 fail.
        if let Value::Map(m) = counters {
            let get = |k: &str| {
                m.get(&Value::keyword(cljrs_value::Keyword {
                    namespace: None,
                    name: Arc::from(k),
                }))
            };
            assert_eq!(get("test"), Some(Value::Long(2)));
            assert_eq!(get("pass"), Some(Value::Long(1)));
            assert_eq!(get("fail"), Some(Value::Long(1)));
            assert_eq!(get("error"), Some(Value::Long(0)));
        } else {
            panic!("expected map from run-tests, got {counters:?}");
        }
    }

    #[test]
    fn test_alter_meta_bang() {
        // alter-meta! applies fn to var's meta and stores result.
        let (_, mut env) = make_env();
        eval_src("(def myvar 42)", &mut env).unwrap();
        eval_src("(alter-meta! #'myvar assoc :foo :bar)", &mut env).unwrap();
        let m = eval_src("(meta #'myvar)", &mut env).unwrap();
        if let Value::Map(map) = m {
            let foo_key = Value::keyword(cljrs_value::Keyword {
                namespace: None,
                name: Arc::from("foo"),
            });
            assert!(map.get(&foo_key).is_some());
        } else {
            panic!("expected map, got {m:?}");
        }
    }

    #[test]
    fn test_catch_runtime_error() {
        // (try (/ 1 0) (catch Exception e "caught")) => "caught"
        let (_, mut env) = make_env();
        let v = eval_src(r#"(try (/ 1 0) (catch Exception e "caught"))"#, &mut env).unwrap();
        assert_eq!(v, Value::string("caught".to_string()));
    }

    #[test]
    fn test_ns_resolve() {
        let (_, mut env) = make_env();
        eval_src("(def somevar 99)", &mut env).unwrap();
        // ns-resolve with current ns returns the var.
        let v = eval_src("(ns-resolve *ns* 'somevar)", &mut env).unwrap();
        assert!(matches!(v, Value::Var(_)));
        // ns-resolve for non-existent symbol returns nil.
        let v2 = eval_src("(ns-resolve *ns* 'nonexistent)", &mut env).unwrap();
        assert_eq!(v2, Value::Nil);
    }

    // ── Persistent structure virtualization ──────────────────────────────

    #[test]
    fn test_assoc_chain_virtualized() {
        // Assoc chain where intermediates aren't used — should be virtualized.
        let v = eval_str(
            "(let [m {}
                   a (assoc m :x 1)
                   b (assoc a :y 2)
                   c (assoc b :z 3)]
               c)",
        )
        .unwrap();
        // Result should be {:x 1, :y 2, :z 3}.
        assert!(matches!(&v, Value::Map(_)));
        if let Value::Map(m) = &v {
            assert_eq!(m.count(), 3);
            assert_eq!(m.get(&Value::keyword(Keyword::simple("x"))), Some(long(1)));
            assert_eq!(m.get(&Value::keyword(Keyword::simple("y"))), Some(long(2)));
            assert_eq!(m.get(&Value::keyword(Keyword::simple("z"))), Some(long(3)));
        }
    }

    #[test]
    fn test_conj_chain_virtualized() {
        // Conj chain on a vector.
        let v = eval_str(
            "(let [v [1]
                   a (conj v 2)
                   b (conj a 3)
                   c (conj b 4)]
               c)",
        )
        .unwrap();
        assert_eq!(v, eval_str("[1 2 3 4]").unwrap());
    }

    #[test]
    fn test_assoc_chain_intermediate_used_no_virtualize() {
        // If an intermediate is used in the body, virtualization should not apply,
        // but the result should still be correct.
        let v = eval_str(
            "(let [a (assoc {} :x 1)
                   b (assoc a :y 2)]
               (list (count a) (count b)))",
        )
        .unwrap();
        // a has 1 entry, b has 2.
        if let Value::List(l) = &v {
            let items: Vec<_> = l.get().iter().cloned().collect();
            assert_eq!(items, vec![long(1), long(2)]);
        } else {
            panic!("expected list, got {:?}", v);
        }
    }

    #[test]
    fn test_assoc_chain_on_existing_map() {
        // Chain on an existing non-empty map.
        let v = eval_str(
            "(let [m {:a 1}
                   a (assoc m :b 2)
                   b (assoc a :c 3)]
               b)",
        )
        .unwrap();
        if let Value::Map(m) = &v {
            assert_eq!(m.count(), 3);
        } else {
            panic!("expected map");
        }
    }
}
