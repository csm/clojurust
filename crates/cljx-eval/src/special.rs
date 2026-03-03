//! Special form evaluators.

use std::sync::Arc;

use crate::destructure::bind_pattern;
use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::eval::{eval, eval_body, form_to_value, is_special_form};
use cljx_gc::GcPtr;
use cljx_reader::Form;
use cljx_reader::form::FormKind;
use cljx_value::{CljxFn, CljxFnArity, Value};

/// The set of names that trigger special-form dispatch.
pub const SPECIAL_FORMS: &[&str] = &[
    "def", "fn*", "fn", "if", "do", "let*", "let", "loop*", "loop", "recur", "quote", "var",
    "set!", "throw", "try", "defn", "defmacro", "defonce", "and", "or", ".", "ns", "require",
];

/// Dispatch to the right special-form handler.
pub fn eval_special(head: &str, args: &[Form], env: &mut Env) -> EvalResult {
    match head {
        "def" => eval_def(args, env),
        "fn*" | "fn" => eval_fn(args, env),
        "if" => eval_if(args, env),
        "do" => eval_body(args, env),
        "let*" | "let" => eval_let(args, env),
        "loop*" | "loop" => eval_loop(args, env),
        "recur" => eval_recur(args, env),
        "quote" => eval_quote(args),
        "var" => eval_var(args, env),
        "set!" => eval_set_bang(args, env),
        "throw" => eval_throw(args, env),
        "try" => eval_try(args, env),
        "defn" => eval_defn(args, env),
        "defmacro" => eval_defmacro(args, env),
        "defonce" => eval_defonce(args, env),
        "and" => eval_and(args, env),
        "or" => eval_or(args, env),
        "." => Err(EvalError::Runtime("interop not yet implemented".into())),
        "ns" => eval_ns(args, env),
        "require" => Ok(Value::Nil), // stub
        _ => unreachable!("unknown special form: {head}"),
    }
}

// ── def ───────────────────────────────────────────────────────────────────────

fn eval_def(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "def")?;
    let val = if args.len() > 1 {
        eval(&args[1], env)?
    } else {
        Value::Nil
    };
    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name), val.clone());
    Ok(Value::Var(var))
}

// ── fn* ───────────────────────────────────────────────────────────────────────

fn eval_fn(args: &[Form], env: &mut Env) -> EvalResult {
    let mut idx = 0;
    let mut name: Option<Arc<str>> = None;

    // Optional name.
    if let Some(FormKind::Symbol(s)) = args.first().map(|f| &f.kind)
        && !is_special_form(s)
    {
        name = Some(Arc::from(s.as_str()));
        idx = 1;
    }

    let rest = &args[idx..];
    if rest.is_empty() {
        return Err(EvalError::Runtime("fn* requires params and body".into()));
    }

    let arities = match &rest[0].kind {
        FormKind::Vector(_) => {
            // Single arity: (fn* [params] body...)
            vec![parse_arity(&rest[0], &rest[1..])?]
        }
        FormKind::List(_) => {
            // Multi-arity: (fn* ([params] body...) ...)
            rest.iter()
                .map(|arity_form| {
                    if let FormKind::List(forms) = &arity_form.kind {
                        if forms.is_empty() {
                            return Err(EvalError::Runtime("arity clause requires params".into()));
                        }
                        parse_arity(&forms[0], &forms[1..])
                    } else {
                        Err(EvalError::Runtime("expected arity clause (list)".into()))
                    }
                })
                .collect::<EvalResult<Vec<_>>>()?
        }
        _ => {
            return Err(EvalError::Runtime(
                "fn* expects vector or arity clauses".into(),
            ));
        }
    };

    // Capture closed-over locals.
    let (closed_over_names, closed_over_vals) = env.all_local_bindings();

    let cljx_fn = CljxFn::new(name, arities, closed_over_names, closed_over_vals, false);
    Ok(Value::Fn(GcPtr::new(cljx_fn)))
}

/// Parse one arity: params-form and body forms.
pub fn parse_arity(params_form: &Form, body: &[Form]) -> EvalResult<CljxFnArity> {
    let param_forms = match &params_form.kind {
        FormKind::Vector(v) => v,
        _ => {
            return Err(EvalError::Runtime(
                "fn arity params must be a vector".into(),
            ));
        }
    };

    let mut params: Vec<Arc<str>> = Vec::new();
    let mut rest_param: Option<Arc<str>> = None;
    let mut saw_amp = false;

    for p in param_forms {
        match &p.kind {
            FormKind::Symbol(s) if s == "&" => {
                saw_amp = true;
            }
            FormKind::Symbol(s) => {
                if saw_amp {
                    rest_param = Some(Arc::from(s.as_str()));
                    break;
                } else {
                    params.push(Arc::from(s.as_str()));
                }
            }
            _ => return Err(EvalError::Runtime("fn params must be symbols".into())),
        }
    }

    Ok(CljxFnArity {
        params,
        rest_param,
        body: body.to_vec(),
    })
}

// ── if ────────────────────────────────────────────────────────────────────────

fn eval_if(args: &[Form], env: &mut Env) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Runtime("if requires a test".into()));
    }
    let test = eval(&args[0], env)?;
    let truthy = !matches!(test, Value::Nil | Value::Bool(false));
    if truthy {
        if args.len() > 1 {
            eval(&args[1], env)
        } else {
            Ok(Value::Nil)
        }
    } else if args.len() > 2 {
        eval(&args[2], env)
    } else {
        Ok(Value::Nil)
    }
}

// ── let* ──────────────────────────────────────────────────────────────────────

fn eval_let(args: &[Form], env: &mut Env) -> EvalResult {
    let bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => v.clone(),
        _ => return Err(EvalError::Runtime("let* requires a binding vector".into())),
    };

    if bindings.len() % 2 != 0 {
        return Err(EvalError::Runtime(
            "let* binding vector must have even length".into(),
        ));
    }

    env.push_frame();

    for pair in bindings.chunks(2) {
        let val = match eval(&pair[1], env) {
            Ok(v) => v,
            Err(e) => {
                env.pop_frame();
                return Err(e);
            }
        };
        if let Err(e) = bind_pattern(&pair[0], val, env) {
            env.pop_frame();
            return Err(e);
        }
    }

    let body = &args[1..];
    let result = eval_body(body, env);
    env.pop_frame();
    result
}

// ── loop* / recur ─────────────────────────────────────────────────────────────

pub fn eval_loop(args: &[Form], env: &mut Env) -> EvalResult {
    let bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => v.clone(),
        _ => return Err(EvalError::Runtime("loop* requires a binding vector".into())),
    };

    if bindings.len() % 2 != 0 {
        return Err(EvalError::Runtime(
            "loop* binding vector must have even length".into(),
        ));
    }

    let body = &args[1..];

    // Separate pattern forms and initial values.
    let patterns: Vec<Form> = bindings.iter().step_by(2).cloned().collect();
    let mut current_vals: Vec<Value> = Vec::new();

    // Evaluate initial values.
    for i in (1..bindings.len()).step_by(2) {
        current_vals.push(eval(&bindings[i], env)?);
    }

    loop {
        env.push_frame();
        for (pat, val) in patterns.iter().zip(current_vals.iter()) {
            if let Err(e) = bind_pattern(pat, val.clone(), env) {
                env.pop_frame();
                return Err(e);
            }
        }

        let result = eval_body_recur(body, env);
        env.pop_frame();

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_vals)) => {
                if new_vals.len() != patterns.len() {
                    return Err(EvalError::Arity {
                        name: "recur".into(),
                        expected: patterns.len().to_string(),
                        got: new_vals.len(),
                    });
                }
                current_vals = new_vals;
            }
            Err(e) => return Err(e),
        }
    }
}

fn eval_recur(args: &[Form], env: &mut Env) -> EvalResult {
    let vals: Vec<Value> = args
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;
    Err(EvalError::Recur(vals))
}

/// Eval body forms, propagating Recur without catching it.
pub fn eval_body_recur(body: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in body {
        result = eval(form, env)?;
    }
    Ok(result)
}

// ── quote ─────────────────────────────────────────────────────────────────────

fn eval_quote(args: &[Form]) -> EvalResult {
    match args.first() {
        Some(f) => Ok(form_to_value(f)),
        None => Err(EvalError::Runtime("quote requires an argument".into())),
    }
}

// ── var ───────────────────────────────────────────────────────────────────────

fn eval_var(args: &[Form], env: &mut Env) -> EvalResult {
    let sym = match args.first().map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => s.clone(),
        _ => return Err(EvalError::Runtime("var requires a symbol".into())),
    };
    let parsed = cljx_value::Symbol::parse(&sym);
    let ns = parsed.namespace.as_deref().unwrap_or(&env.current_ns);
    let name = parsed.name.as_ref();
    env.globals
        .lookup_var_in_ns(ns, name)
        .map(Value::Var)
        .ok_or_else(|| EvalError::UnboundSymbol(sym))
}

// ── set! ──────────────────────────────────────────────────────────────────────

fn eval_set_bang(args: &[Form], env: &mut Env) -> EvalResult {
    let sym = match args.first().map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => s.clone(),
        _ => return Err(EvalError::Runtime("set! requires a symbol".into())),
    };
    let val = if args.len() > 1 {
        eval(&args[1], env)?
    } else {
        Value::Nil
    };
    let parsed = cljx_value::Symbol::parse(&sym);
    let ns = parsed.namespace.as_deref().unwrap_or(&env.current_ns);
    let var = env
        .globals
        .lookup_var_in_ns(ns, &parsed.name)
        .ok_or_else(|| EvalError::UnboundSymbol(sym))?;
    var.get().bind(val.clone());
    Ok(val)
}

// ── throw ─────────────────────────────────────────────────────────────────────

fn eval_throw(args: &[Form], env: &mut Env) -> EvalResult {
    let val = match args.first() {
        Some(f) => eval(f, env)?,
        None => Value::Nil,
    };
    Err(EvalError::Thrown(val))
}

// ── try ───────────────────────────────────────────────────────────────────────

fn eval_try(args: &[Form], env: &mut Env) -> EvalResult {
    // Split args into body, catch clauses, finally.
    let mut body_forms: Vec<&Form> = Vec::new();
    let mut _catch_clauses: Vec<(&str, &[Form])> = Vec::new(); // (binding_sym, handler_body)
    let mut _finally_forms: Vec<&Form> = Vec::new();
    let mut in_catch = false;
    let mut in_finally = false;

    for form in args {
        match &form.kind {
            FormKind::List(parts) if !parts.is_empty() => {
                match &parts[0].kind {
                    FormKind::Symbol(s) if s == "catch" => {
                        // (catch ExType sym handler...)
                        // Phase 4: just one catch clause matching any thrown value.
                        let _sym = match parts.get(2).map(|f| &f.kind) {
                            Some(FormKind::Symbol(s)) => s.as_str(),
                            _ => {
                                return Err(EvalError::Runtime(
                                    "catch requires a binding symbol".into(),
                                ));
                            }
                        };
                        // Store raw reference to form for later processing.
                        // We'll process them as a slice after splitting.
                        in_catch = true;
                        in_finally = false;
                        // Defer; collect whole forms.
                        body_forms.push(form); // sentinel — handled below
                        continue;
                    }
                    FormKind::Symbol(s) if s == "finally" => {
                        in_catch = false;
                        in_finally = true;
                        body_forms.push(form); // sentinel
                        continue;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        if !in_catch && !in_finally {
            body_forms.push(form);
        }
    }

    // Re-parse cleanly.
    let (true_body, catch_sym, catch_body, fin_body) = parse_try_args(args);

    // Eval body.
    let mut result = eval_body(true_body, env);

    // Handle catch.
    result = match result {
        Err(EvalError::Thrown(thrown_val)) => {
            if let Some(sym) = catch_sym {
                env.push_frame();
                env.bind(Arc::from(sym), thrown_val);
                let r = eval_body(catch_body, env);
                env.pop_frame();
                r
            } else {
                Err(EvalError::Thrown(thrown_val))
            }
        }
        other => other,
    };

    // Always run finally.
    if !fin_body.is_empty() {
        let _ = eval_body(fin_body, env);
    }

    result
}

/// Split try args into (body, catch_sym, catch_body, finally_body).
fn parse_try_args(args: &[Form]) -> (&[Form], Option<&str>, &[Form], &[Form]) {
    let mut body_end = args.len();
    let mut catch_sym: Option<&str> = None;
    let mut catch_start = args.len();
    let mut _catch_end = args.len();
    let mut fin_start = args.len();

    for (i, form) in args.iter().enumerate() {
        if let FormKind::List(parts) = &form.kind
            && let Some(FormKind::Symbol(s)) = parts.first().map(|f| &f.kind)
        {
            if s == "catch" {
                if i < body_end {
                    body_end = i;
                }
                catch_start = i;
                _catch_end = i + 1;
                // Extract sym — it's the third element (index 2).
                if let Some(FormKind::Symbol(sym)) = parts.get(2).map(|f| &f.kind) {
                    catch_sym = Some(sym.as_str());
                }
                continue;
            }
            if s == "finally" {
                if i < body_end {
                    body_end = i;
                }
                fin_start = i;
                continue;
            }
        }
    }

    let body = &args[..body_end];
    let catch_body = if catch_sym.is_some() {
        // Extract body from the catch form.
        if let Some(FormKind::List(parts)) = args.get(catch_start).map(|f| &f.kind) {
            // skip (catch ExType sym ...) — body starts at index 3
            &parts[3..]
        } else {
            &[]
        }
    } else {
        &[]
    };
    let fin_body = if fin_start < args.len() {
        if let FormKind::List(parts) = &args[fin_start].kind {
            &parts[1..] // skip "finally"
        } else {
            &[]
        }
    } else {
        &[]
    };

    (body, catch_sym, catch_body, fin_body)
}

// ── defn ──────────────────────────────────────────────────────────────────────

pub fn eval_defn(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "defn")?;
    // Skip optional docstring.
    let rest_start = if args.len() > 2 && matches!(args[1].kind, FormKind::Str(_)) {
        2
    } else {
        1
    };
    // Build (fn* name ...)
    let mut fn_args = vec![Form::new(
        FormKind::Symbol(name.to_string()),
        args[0].span.clone(),
    )];
    fn_args.extend_from_slice(&args[rest_start..]);
    let fn_val = eval_fn(&fn_args, env)?;
    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name), fn_val.clone());
    Ok(Value::Var(var))
}

// ── defmacro ──────────────────────────────────────────────────────────────────

fn eval_defmacro(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "defmacro")?;
    let rest_start = if args.len() > 2 && matches!(args[1].kind, FormKind::Str(_)) {
        2
    } else {
        1
    };
    let mut fn_args = vec![Form::new(
        FormKind::Symbol(name.to_string()),
        args[0].span.clone(),
    )];
    fn_args.extend_from_slice(&args[rest_start..]);
    let fn_val = eval_fn(&fn_args, env)?;

    // Convert Fn → Macro by setting is_macro = true.
    let macro_val = match fn_val {
        Value::Fn(f) => {
            let mut mfn = f.get().clone();
            mfn.is_macro = true;
            Value::Macro(GcPtr::new(mfn))
        }
        other => other,
    };

    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name), macro_val.clone());
    Ok(Value::Var(var))
}

// ── defonce ───────────────────────────────────────────────────────────────────

fn eval_defonce(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "defonce")?;
    // If already bound, return immediately.
    if let Some(var) = env.globals.lookup_var(&env.current_ns, name)
        && var.get().is_bound()
    {
        return Ok(Value::Var(var));
    }
    eval_def(args, env)
}

// ── and / or ──────────────────────────────────────────────────────────────────

fn eval_and(args: &[Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Bool(true);
    for form in args {
        result = eval(form, env)?;
        if matches!(result, Value::Nil | Value::Bool(false)) {
            return Ok(result);
        }
    }
    Ok(result)
}

fn eval_or(args: &[Form], env: &mut Env) -> EvalResult {
    for form in args {
        let v = eval(form, env)?;
        if !matches!(v, Value::Nil | Value::Bool(false)) {
            return Ok(v);
        }
    }
    Ok(Value::Nil)
}

// ── ns ────────────────────────────────────────────────────────────────────────

fn eval_ns(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "ns")?;
    env.globals.get_or_create_ns(name);
    env.current_ns = Arc::from(name);
    Ok(Value::Nil)
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn require_sym<'a>(args: &'a [Form], idx: usize, form_name: &str) -> EvalResult<&'a str> {
    match args.get(idx).map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => Ok(s.as_str()),
        _ => Err(EvalError::Runtime(format!(
            "{form_name} requires a symbol at position {idx}"
        ))),
    }
}
