//! Top-level `eval` dispatcher and form-to-value conversion.

use cljx_gc::GcPtr;
use cljx_reader::Form;
use cljx_reader::form::FormKind;
use cljx_value::{
    Keyword, MapValue, PersistentHashSet, PersistentList, PersistentVector, Symbol, Value,
};

use crate::apply::eval_call;
use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::special::{SPECIAL_FORMS, eval_special};
use crate::syntax_quote::syntax_quote;

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
        FormKind::Regex(_) => Err(EvalError::Runtime(
            "regex literals not yet supported".into(),
        )),

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
            let mut m = MapValue::empty();
            for pair in forms.chunks(2) {
                let k = eval(&pair[0], env)?;
                let v = eval(&pair[1], env)?;
                m = m.assoc(k, v);
            }
            Ok(Value::Map(m))
        }
        FormKind::Set(forms) => {
            let mut s = PersistentHashSet::empty();
            for f in forms {
                let v = eval(f, env)?;
                s = s.conj(v);
            }
            Ok(Value::Set(GcPtr::new(s)))
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
            match v {
                Value::Atom(a) => Ok(a.get().deref()),
                Value::Var(var) => var
                    .get()
                    .deref()
                    .ok_or_else(|| EvalError::Runtime("unbound var".into())),
                _ => Err(EvalError::Runtime(format!(
                    "cannot deref {}",
                    v.type_name()
                ))),
            }
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
        FormKind::TaggedLiteral(_, _) => Err(EvalError::Runtime(
            "tagged literals not yet supported".into(),
        )),
    }
}

// ── List / call dispatch ──────────────────────────────────────────────────────

fn eval_list(forms: &[Form], env: &mut Env) -> EvalResult {
    if forms.is_empty() {
        return Ok(Value::List(GcPtr::new(PersistentList::empty())));
    }

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
        if let Some(ns) = &sym.namespace {
            return env
                .globals
                .lookup_in_ns(ns, &sym.name)
                .ok_or_else(|| EvalError::UnboundSymbol(s.to_string()));
        }
    }

    Err(EvalError::UnboundSymbol(s.to_string()))
}

// ── is_special_form ───────────────────────────────────────────────────────────

pub fn is_special_form(s: &str) -> bool {
    SPECIAL_FORMS.contains(&s)
}

// ── eval_body ─────────────────────────────────────────────────────────────────

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
        FormKind::Regex(s) => Value::string(s.clone()),

        FormKind::List(forms) => {
            let items: Vec<Value> = forms.iter().map(form_to_value).collect();
            Value::List(GcPtr::new(PersistentList::from_iter(items)))
        }
        FormKind::Vector(forms) => {
            let items: Vec<Value> = forms.iter().map(form_to_value).collect();
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
            Value::Set(GcPtr::new(s))
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
            let items: Vec<Value> = body.iter().map(form_to_value).collect();
            Value::List(GcPtr::new(PersistentList::from_iter(items)))
        }
        FormKind::TaggedLiteral(_, inner) => form_to_value(inner),
        FormKind::ReaderCond { .. } => Value::Nil,
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
    use num_traits::Zero;
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
    Ok(Value::Ratio(GcPtr::new(num_rational::Ratio::new(
        numer, denom,
    ))))
}

// ── reader cond ───────────────────────────────────────────────────────────────

fn eval_reader_cond(clauses: &[Form], env: &mut Env) -> EvalResult {
    // clauses = [kw form kw form ...]
    let mut i = 0;
    let mut default: Option<&Form> = None;
    while i + 1 < clauses.len() {
        match &clauses[i].kind {
            FormKind::Keyword(k) if k == "cljx" => {
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

// ── anon fn expansion ─────────────────────────────────────────────────────────

/// Expand `#(...)` to `(fn* [p__1 p__2 ... & rest__] ...)`.
fn expand_anon_fn(body: &[Form], span: cljx_types::span::Span) -> Form {
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

    let mut fn_forms = vec![
        Form::new(FormKind::Symbol("fn*".into()), s.clone()),
        Form::new(FormKind::Vector(params), s.clone()),
    ];
    fn_forms.extend(new_body);
    Form::new(FormKind::List(fn_forms), span)
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

fn rewrite_pct_refs(forms: &[Form], span: cljx_types::span::Span) -> Vec<Form> {
    forms
        .iter()
        .map(|f| rewrite_pct_form(f, span.clone()))
        .collect()
}

fn rewrite_pct_form(form: &Form, span: cljx_types::span::Span) -> Form {
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
        let mut parser = cljx_reader::Parser::new(src.to_string(), "<test>".to_string());
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
    fn test_reader_cond_cljx() {
        // :cljx branch selected.
        assert_eq!(eval_str("#?(:cljx 1 :clj 2)").unwrap(), long(1));
    }

    #[test]
    fn test_reader_cond_default() {
        // No :cljx; fall through to :default.
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
        let v = eval_str("(map inc [1 2 3])").unwrap();
        assert!(matches!(v, Value::List(_)));
        if let Value::List(l) = &v {
            let items: Vec<_> = l.get().iter().cloned().collect();
            assert_eq!(items, vec![long(2), long(3), long(4)]);
        }
    }

    #[test]
    fn test_filter_fn() {
        let v = eval_str("(filter odd? [1 2 3 4 5])").unwrap();
        assert!(matches!(v, Value::List(_)));
        if let Value::List(l) = &v {
            assert_eq!(l.get().count(), 3);
        }
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
        let path = std::env::temp_dir().join("cljx_test_spit_slurp.txt");
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
}
