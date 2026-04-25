//! Special form evaluators.

use std::collections::HashMap;
use std::sync::Arc;

use crate::destructure::bind_pattern;
use crate::eval::{eval, eval_body, is_special_form};
use cljrs_builtins::form::{expand_reader_conds, form_to_value, select_reader_cond};
use cljrs_env::env::{Env, RequireRefer, RequireSpec};
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_env::loader::load_ns;
use cljrs_gc::GcPtr;
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_value::error::ExceptionInfo;
use cljrs_value::{
    CljxFn, CljxFnArity, Keyword, MapValue, MultiFn, Protocol, ProtocolFn, ProtocolMethod,
    TypeInstance, Value, ValueError,
};

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
        "defn" | "defn-" => eval_defn(args, env),
        "defmacro" => eval_defmacro(args, env),
        "defonce" => eval_defonce(args, env),
        "and" => eval_and(args, env),
        "or" => eval_or(args, env),
        "." => Err(EvalError::Runtime("interop not yet implemented".into())),
        "ns" => eval_ns(args, env),
        "require" => eval_require(args, env),
        "letfn" => eval_letfn(args, env),
        "in-ns" => eval_in_ns(args, env),
        "alias" => eval_alias(args, env),
        "defprotocol" => eval_defprotocol(args, env),
        "extend-type" => eval_extend_type(args, env),
        "extend-protocol" => eval_extend_protocol(args, env),
        "defmulti" => eval_defmulti(args, env),
        "defmethod" => eval_defmethod(args, env),
        "defrecord" => eval_defrecord(args, env),
        "reify" => eval_reify(args, env),
        "load-file" => eval_load_file(args, env),
        "binding" => eval_binding(args, env),
        "with-out-str" => eval_with_out_str(args, env),
        _ => unreachable!("unknown special form: {head}"),
    }
}

// ── def ───────────────────────────────────────────────────────────────────────

fn eval_def(args: &[Form], env: &mut Env) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Runtime("def requires a name".into()));
    }
    let (name, meta_opt) = extract_def_name(&args[0], env)?;
    // Skip optional docstring: (def name "docstring" value)
    let val_idx = if args.len() > 2 && matches!(args[1].kind, FormKind::Str(_)) {
        2
    } else {
        1
    };
    let val = if args.len() > val_idx {
        // Under no-gc: def value expressions go to the StaticArena since the
        // Var must outlive all scratch regions.
        #[cfg(feature = "no-gc")]
        let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
        eval(&args[val_idx], env)?
    } else {
        Value::Nil
    };
    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name.as_str()), val.clone());
    if let Some(meta_val) = meta_opt {
        var.get().set_meta(meta_val);
    }
    Ok(Value::Var(var))
}

/// Extract the def name and optional metadata from the name form.
fn extract_def_name(form: &Form, env: &mut Env) -> EvalResult<(String, Option<Value>)> {
    match &form.kind {
        FormKind::Symbol(s) => Ok((s.clone(), None)),
        FormKind::Meta(meta_form, inner) => {
            let meta_val = compile_meta_form(meta_form, env)?;
            match &inner.kind {
                FormKind::Symbol(s) => Ok((s.clone(), Some(meta_val))),
                _ => Err(EvalError::Runtime("def name must be a symbol".into())),
            }
        }
        _ => Err(EvalError::Runtime("def name must be a symbol".into())),
    }
}

/// Expand a metadata shorthand form into a map value.
fn compile_meta_form(meta: &Form, env: &mut Env) -> EvalResult<Value> {
    match &meta.kind {
        FormKind::Keyword(k) => {
            // ^:dynamic  →  {:dynamic true}
            let m = MapValue::empty().assoc(Value::keyword(Keyword::parse(k)), Value::Bool(true));
            Ok(Value::Map(m))
        }
        FormKind::Symbol(s) => {
            // ^TypeHint  →  {:tag "TypeHint"}
            let m = MapValue::empty().assoc(
                Value::keyword(Keyword::parse("tag")),
                Value::string(s.clone()),
            );
            Ok(Value::Map(m))
        }
        _ => eval(meta, env), // literal map or general expr
    }
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

    let cljrs_fn = CljxFn::new(
        name.clone(),
        arities,
        closed_over_names,
        closed_over_vals,
        false,
        Arc::clone(&env.current_ns),
    );

    env.on_fn_defined(&cljrs_fn);

    // Eagerly lower each arity to IR if the compiler is ready.
    //eager_lower_fn(&cljrs_fn, env);

    Ok(Value::Fn(GcPtr::new(cljrs_fn)))
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
    let mut destructure_params: Vec<(usize, Form)> = Vec::new();
    let mut destructure_rest: Option<Form> = None;
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
            // Destructuring patterns: vectors and maps
            FormKind::Vector(_) | FormKind::Map(_) => {
                if saw_amp {
                    let gensym = format!("__destructure_rest_{}", params.len());
                    rest_param = Some(Arc::from(gensym.as_str()));
                    destructure_rest = Some(p.clone());
                    break;
                } else {
                    let idx = params.len();
                    let gensym = format!("__destructure_{idx}");
                    params.push(Arc::from(gensym.as_str()));
                    destructure_params.push((idx, p.clone()));
                }
            }
            _ => {
                return Err(EvalError::Runtime(
                    "fn params must be symbols, vectors, or maps".into(),
                ));
            }
        }
    }

    Ok(CljxFnArity {
        params,
        rest_param,
        body: body.to_vec(),
        destructure_params,
        destructure_rest,
        ir_arity_id: crate::arity::fresh_arity_id(),
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

    let body = &args[1..];

    // Detect assoc/conj chains that can be virtualized.
    let chains = crate::virtualize::detect_let_chains(&bindings);
    let virtualizable_chains = find_virtualizable_chains(&chains, &bindings, body);

    env.push_frame();

    let pairs: Vec<_> = bindings.chunks(2).collect();
    let mut i = 0;
    while i < pairs.len() {
        // Check if this binding starts a virtualizable chain.
        if let Some(chain) = virtualizable_chains.iter().find(|c| c.start == i) {
            match eval_virtualized_chain(chain, &pairs, env) {
                Ok(()) => {
                    i += chain.len;
                    continue;
                }
                Err(e) => {
                    env.pop_frame();
                    return Err(e);
                }
            }
        }

        // Normal evaluation.
        let pair = pairs[i];
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
        i += 1;
    }

    let result = eval_body(body, env);
    env.pop_frame();
    result
}

/// Filter chains to only those that are safe to virtualize.
///
/// A chain is safe if no intermediate binding is used outside the chain
/// (i.e., it's only used as the collection argument of the next step).
fn find_virtualizable_chains<'a>(
    chains: &'a [crate::virtualize::LetChain],
    bindings: &[Form],
    body: &[Form],
) -> Vec<&'a crate::virtualize::LetChain> {
    chains
        .iter()
        .filter(|chain| {
            // Check that no intermediate (all except the last) is used in body
            // or in other bindings outside the chain.
            for j in chain.start..(chain.start + chain.len - 1) {
                let name = match &bindings[j * 2].kind {
                    FormKind::Symbol(s) => s.as_str(),
                    _ => return false,
                };
                if crate::virtualize::binding_used_in_body(name, body) {
                    return false;
                }
                if crate::virtualize::binding_used_in_other_bindings(
                    name,
                    bindings,
                    chain.start,
                    chain.len,
                ) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// Evaluate an assoc/conj chain using transient operations.
///
/// Instead of creating N intermediate persistent collections, we:
/// 1. Evaluate the root collection (first arg of the first assoc/conj)
/// 2. Convert to transient
/// 3. Apply each assoc!/conj! mutably
/// 4. Convert back to persistent
/// 5. Bind only the final name (and intermediate names point to intermediate
///    transient values for correctness, though they shouldn't be used).
fn eval_virtualized_chain(
    chain: &crate::virtualize::LetChain,
    pairs: &[&[Form]],
    env: &mut Env,
) -> Result<(), EvalError> {
    use cljrs_builtins::transients::{
        builtin_assoc_bang, builtin_conj_bang, builtin_persistent_bang, builtin_transient,
    };

    // Step 1: Evaluate the root collection (first arg of the first call).
    let first_pair = pairs[chain.start];
    let first_expr_forms = match &first_pair[1].kind {
        FormKind::List(forms) => forms,
        _ => unreachable!("chain detection ensures this is a list"),
    };
    let root_collection = eval(&first_expr_forms[1], env)?;

    // Step 2: Convert to transient.
    let mut transient = match builtin_transient(std::slice::from_ref(&root_collection)) {
        Ok(t) => t,
        Err(_) => {
            // Collection doesn't support transients (e.g., sorted map).
            // Fall back to normal evaluation for the whole chain.
            return eval_chain_normally(chain, pairs, env);
        }
    };

    // Step 3: Apply each chain operation using transient mutation.
    for j in 0..chain.len {
        let pair_idx = chain.start + j;
        let pair = pairs[pair_idx];
        let expr_forms = match &pair[1].kind {
            FormKind::List(forms) => forms,
            _ => unreachable!(),
        };

        // Evaluate the non-collection arguments.
        let mut args = vec![transient.clone()];
        for arg_form in expr_forms.iter().skip(2) {
            args.push(eval(arg_form, env)?);
        }

        // Apply the transient operation.
        transient = match chain.ops[j] {
            crate::virtualize::ChainOpKind::Assoc => {
                builtin_assoc_bang(&args).map_err(|e| EvalError::Runtime(e.to_string()))?
            }
            crate::virtualize::ChainOpKind::Conj => {
                builtin_conj_bang(&args).map_err(|e| EvalError::Runtime(e.to_string()))?
            }
        };

        // Bind intermediate names to a placeholder (they shouldn't be used,
        // but we need them in the env for structural correctness).
        // Only the last binding gets the persistent result.
        if j < chain.len - 1 {
            let name = match &pair[0].kind {
                FormKind::Symbol(s) => s.clone(),
                _ => unreachable!(),
            };
            // Bind to nil as placeholder — intermediates are verified as unused.
            env.bind(Arc::from(name.as_str()), Value::Nil);
        }
    }

    // Step 4: Convert back to persistent.
    let persistent =
        builtin_persistent_bang(&[transient]).map_err(|e| EvalError::Runtime(e.to_string()))?;

    // Step 5: Bind the final name.
    let last_pair = pairs[chain.start + chain.len - 1];
    bind_pattern(&last_pair[0], persistent, env)?;

    Ok(())
}

/// Fallback: evaluate a chain using normal persistent operations.
fn eval_chain_normally(
    chain: &crate::virtualize::LetChain,
    pairs: &[&[Form]],
    env: &mut Env,
) -> Result<(), EvalError> {
    for j in 0..chain.len {
        let pair_idx = chain.start + j;
        let pair = pairs[pair_idx];
        let val = eval(&pair[1], env)?;
        bind_pattern(&pair[0], val, env)?;
    }
    Ok(())
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
        // Root current_vals so they survive GC — they're not yet bound in env.
        let _vals_root = cljrs_env::gc_roots::root_values(&current_vals);

        // GC safepoint on every loop iteration so tight recur loops
        // don't starve the collector.
        cljrs_env::gc_roots::gc_safepoint(env);

        // Under no-gc: push a fresh scratch region for this iteration.
        // Intermediates allocated in the body land here and are freed
        // after the iteration ends.
        #[cfg(feature = "no-gc")]
        let mut scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();

        env.push_frame();
        for (pat, val) in patterns.iter().zip(current_vals.iter()) {
            if let Err(e) = bind_pattern(pat, val.clone(), env) {
                env.pop_frame();
                return Err(e);
            }
        }

        // Under no-gc: pop scratch before tail expression so the return value
        // or recur args are allocated in the enclosing scope's context.
        #[cfg(not(feature = "no-gc"))]
        let result = eval_body_recur(body, env);
        #[cfg(feature = "no-gc")]
        let result = eval_body_with_scratch_loop(body, &mut scratch, env);

        env.pop_frame();
        // scratch drops here, resetting the region (freeing intermediates).

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
                // new_vals were evaluated in the caller's context (scratch was
                // popped before the tail form), so they survive the reset.
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

/// Under `no-gc`: eval loop body with scratch-region semantics.
///
/// Evaluates all non-tail forms inside the scratch region, then pops the
/// scratch before the tail (return/recur) expression so it lands in the
/// caller's context.
#[cfg(feature = "no-gc")]
fn eval_body_with_scratch_loop(
    body: &[Form],
    scratch: &mut cljrs_gc::alloc_ctx::ScratchGuard,
    env: &mut Env,
) -> EvalResult {
    if body.is_empty() {
        scratch.pop_for_return();
        return Ok(Value::Nil);
    }
    for form in &body[..body.len() - 1] {
        eval(form, env)?;
    }
    scratch.pop_for_return();
    eval(&body[body.len() - 1], env)
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
    let parsed = cljrs_value::Symbol::parse(&sym);
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
    let parsed = cljrs_value::Symbol::parse(&sym);
    let ns = parsed.namespace.as_deref().unwrap_or(&env.current_ns);
    let var = env
        .globals
        .lookup_var_in_ns(ns, &parsed.name)
        .ok_or_else(|| EvalError::UnboundSymbol(sym))?;
    // Prefer updating the thread-local binding if one exists.
    if !cljrs_env::dynamics::set_thread_local(&var, val.clone()) {
        var.get().bind(val.clone());
    }
    Ok(val)
}

// ── throw ─────────────────────────────────────────────────────────────────────

fn eval_throw(args: &[Form], env: &mut Env) -> EvalResult {
    let val = match args.first() {
        Some(f) => eval(f, env)?,
        None => Value::Nil,
    };
    // Wrap non-error values in an ExceptionInfo so try/catch always sees a
    // Value::Error and ex-message / ex-data work uniformly inside the handler.
    let val = match val {
        Value::Error(_) => val,
        other => {
            let msg = format!("{}", other);
            Value::Error(GcPtr::new(ExceptionInfo::new(
                ValueError::Other(msg.clone()),
                msg,
                None,
                None,
            )))
        }
    };
    Err(EvalError::Thrown(val))
}

// ── try ───────────────────────────────────────────────────────────────────────

struct CatchClause<'a> {
    type_sym: &'a str,
    binding: &'a str,
    body: &'a [Form],
}

/// Convert a non-Thrown EvalError into a `Value::Error` so it can be bound
/// inside a catch clause and inspected with `ex-message` / `ex-data`.
fn eval_error_to_value(err: &EvalError) -> Value {
    let msg = err.to_string();
    Value::Error(GcPtr::new(ExceptionInfo::new(
        ValueError::Other(msg.clone()),
        msg,
        None,
        None,
    )))
}

/// Test whether the type symbol on a `(catch <Type> e ...)` clause matches a
/// thrown value. Type names are matched by their last `.`-separated segment so
/// fully-qualified names like `java.lang.Exception` work as well as bare ones.
fn catch_type_matches(type_name: &str, val: &Value) -> bool {
    let short = type_name.rsplit('.').next().unwrap_or(type_name);
    match short {
        // Catch-all (matches any value, error or not — back-compat).
        "Object" | "Exception" | "Throwable" | "Error" => true,
        // ExceptionInfo only matches actual ex-info / Exception values.
        "ExceptionInfo" => matches!(val, Value::Error(_)),
        _ => false,
    }
}

fn eval_try(args: &[Form], env: &mut Env) -> EvalResult {
    let (body, catches, fin_body) = parse_try_args(args);

    let mut result = eval_body(body, env);

    // Handle catch: never intercept Recur (loop trampoline signal).
    let err_opt = match std::mem::replace(&mut result, Ok(Value::Nil)) {
        Ok(v) => {
            result = Ok(v);
            None
        }
        Err(EvalError::Recur(args)) => {
            result = Err(EvalError::Recur(args));
            None
        }
        Err(other) => Some(other),
    };

    if let Some(err) = err_opt {
        let thrown_val = match err {
            EvalError::Thrown(v) => v,
            ref other => eval_error_to_value(other),
        };
        let mut handled = false;
        for c in &catches {
            if catch_type_matches(c.type_sym, &thrown_val) {
                env.push_frame();
                env.bind(Arc::from(c.binding), thrown_val.clone());
                result = eval_body(c.body, env);
                env.pop_frame();
                handled = true;
                break;
            }
        }
        if !handled {
            // No matching catch — re-throw.
            result = Err(EvalError::Thrown(thrown_val));
        }
    }

    // Always run finally.
    if !fin_body.is_empty() {
        let _ = eval_body(fin_body, env);
    }

    result
}

/// Split try args into (body, catch clauses, finally body).
fn parse_try_args(args: &[Form]) -> (&[Form], Vec<CatchClause<'_>>, &[Form]) {
    let mut body_end = args.len();
    let mut catches: Vec<CatchClause<'_>> = Vec::new();
    let mut fin_body: &[Form] = &[];

    for (i, form) in args.iter().enumerate() {
        if let FormKind::List(parts) = &form.kind
            && let Some(FormKind::Symbol(s)) = parts.first().map(|f| &f.kind)
        {
            if s == "catch" {
                if i < body_end {
                    body_end = i;
                }
                let type_sym = match parts.get(1).map(|f| &f.kind) {
                    Some(FormKind::Symbol(s)) => s.as_str(),
                    _ => continue,
                };
                let binding = match parts.get(2).map(|f| &f.kind) {
                    Some(FormKind::Symbol(s)) => s.as_str(),
                    _ => continue,
                };
                catches.push(CatchClause {
                    type_sym,
                    binding,
                    body: &parts[3..],
                });
                continue;
            }
            if s == "finally" {
                if i < body_end {
                    body_end = i;
                }
                fin_body = &parts[1..];
                continue;
            }
        }
    }

    (&args[..body_end], catches, fin_body)
}

// ── defn ──────────────────────────────────────────────────────────────────────

pub fn eval_defn(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "defn")?;
    // Skip optional docstring and/or metadata map after the name.
    // Valid orderings: (defn name body...), (defn name "doc" body...),
    // (defn name {:meta ...} body...), (defn name "doc" {:meta ...} body...).
    let mut rest_start = 1;
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Str(_)) {
        rest_start += 1;
    }
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Map(_)) {
        rest_start += 1;
    }
    // Build (fn* name ...)
    let mut fn_args = vec![Form::new(
        FormKind::Symbol(name.to_string()),
        args[0].span.clone(),
    )];
    fn_args.extend_from_slice(&args[rest_start..]);
    // Under no-gc: the Fn object must live in the StaticArena since the Var
    // intern outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let fn_val = eval_fn(&fn_args, env)?;
    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name), fn_val.clone());
    Ok(Value::Var(var))
}

// ── defmacro ──────────────────────────────────────────────────────────────────

/// Prepend `&form` and `&env` Form symbols to an arity form's parameter vector.
///
/// Handles:
/// - Single-arity vector `[params...]` → `[&form &env params...]`
/// - Multi-arity clause list `([params...] body...)` → `([&form &env params...] body...)`
fn prepend_macro_params(form: &Form) -> Form {
    let span = form.span.clone();
    match &form.kind {
        FormKind::Vector(params) => {
            let mut new_params = vec![
                Form::new(FormKind::Symbol("&form".to_string()), span.clone()),
                Form::new(FormKind::Symbol("&env".to_string()), span.clone()),
            ];
            new_params.extend_from_slice(params);
            Form::new(FormKind::Vector(new_params), span)
        }
        FormKind::List(forms) => {
            // Arity clause: ([params...] body...) — prepend to first element (params vector).
            if let Some(first) = forms.first() {
                let new_params_form = prepend_macro_params(first);
                let mut new_forms = vec![new_params_form];
                new_forms.extend_from_slice(&forms[1..]);
                Form::new(FormKind::List(new_forms), span)
            } else {
                form.clone()
            }
        }
        _ => form.clone(),
    }
}

fn eval_defmacro(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "defmacro")?;
    let mut rest_start = 1;
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Str(_)) {
        rest_start += 1;
    }
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Map(_)) {
        rest_start += 1;
    }
    // Prepend implicit &form and &env params to each arity.
    let mut fn_args = vec![Form::new(
        FormKind::Symbol(name.to_string()),
        args[0].span.clone(),
    )];
    for form in &args[rest_start..] {
        fn_args.push(prepend_macro_params(form));
    }
    // Under no-gc: the Macro object must live in the StaticArena since the Var
    // intern outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
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
    let mut last = Value::Nil;
    for form in args {
        last = eval(form, env)?;
        if !matches!(last, Value::Nil | Value::Bool(false)) {
            return Ok(last);
        }
    }
    Ok(last)
}

// ── require ───────────────────────────────────────────────────────────────────

fn eval_require(args: &[Form], env: &mut Env) -> EvalResult {
    for arg in args {
        let val = eval(arg, env)?;
        let spec = parse_require_spec_val(val).map_err(EvalError::Runtime)?;
        load_ns(env.globals.clone(), &spec, &env.current_ns)?;
    }
    Ok(Value::Nil)
}

/// Parse a `RequireSpec` from an already-evaluated `Value`.
/// Accepts: `'some.ns`, `['some.ns :as alias]`, `['some.ns :refer [syms]]`,
/// `['some.ns :refer :all]`.
fn parse_require_spec_val(val: Value) -> Result<RequireSpec, String> {
    match val {
        Value::Symbol(s) => Ok(RequireSpec {
            ns: s.get().name.clone(),
            alias: None,
            refer: RequireRefer::None,
        }),
        Value::Vector(v) => {
            let items: Vec<Value> = v.get().iter().cloned().collect();
            if items.is_empty() {
                return Err("require spec vector must not be empty".into());
            }
            let ns = match &items[0] {
                Value::Symbol(s) => s.get().name.clone(),
                other => {
                    return Err(format!(
                        "require spec: first element must be a symbol, got {}",
                        other.type_name()
                    ));
                }
            };
            let mut alias = None;
            let mut refer = RequireRefer::None;
            let mut i = 1;
            while i < items.len() {
                match &items[i] {
                    Value::Keyword(k) if k.get().name.as_ref() == "as" => {
                        i += 1;
                        alias = Some(match items.get(i) {
                            Some(Value::Symbol(s)) => s.get().name.clone(),
                            _ => return Err("require :as expects a symbol".into()),
                        });
                    }
                    Value::Keyword(k) if k.get().name.as_ref() == "refer" => {
                        i += 1;
                        refer = match items.get(i) {
                            Some(Value::Keyword(k2)) if k2.get().name.as_ref() == "all" => {
                                RequireRefer::All
                            }
                            Some(Value::Vector(rv)) => {
                                let names: Vec<Arc<str>> = rv
                                    .get()
                                    .iter()
                                    .map(|v| match v {
                                        Value::Symbol(s) => Ok(s.get().name.clone()),
                                        other => Err(format!(
                                            "require :refer expects symbols, got {}",
                                            other.type_name()
                                        )),
                                    })
                                    .collect::<Result<_, _>>()?;
                                RequireRefer::Named(names)
                            }
                            _ => {
                                return Err(
                                    "require :refer expects :all or a vector of symbols".into()
                                );
                            }
                        };
                    }
                    other => {
                        return Err(format!(
                            "require spec: unexpected option {}",
                            other.type_name()
                        ));
                    }
                }
                i += 1;
            }
            Ok(RequireSpec { ns, alias, refer })
        }
        other => Err(format!(
            "require expects a symbol or vector, got {}",
            other.type_name()
        )),
    }
}

/// Parse a `RequireSpec` from a raw `Form` (unevaluated, used in `ns` macro).
fn parse_require_spec_form(form: &Form) -> Result<RequireSpec, String> {
    match &form.kind {
        FormKind::Symbol(s) => Ok(RequireSpec {
            ns: Arc::from(s.as_str()),
            alias: None,
            refer: RequireRefer::None,
        }),
        FormKind::Vector(items) => {
            if items.is_empty() {
                return Err("require spec vector must not be empty".into());
            }
            let ns = match &items[0].kind {
                FormKind::Symbol(s) => Arc::from(s.as_str()),
                _ => return Err("require spec: first element must be a symbol".into()),
            };
            let mut alias = None;
            let mut refer = RequireRefer::None;
            let mut i = 1;
            while i < items.len() {
                // Resolve reader conditionals inline (e.g. #?(:cljs :refer-macros :default :refer)).
                let item = match &items[i].kind {
                    FormKind::ReaderCond { clauses, .. } => {
                        match select_reader_cond(clauses) {
                            Some(f) => f,
                            None => {
                                i += 1;
                                continue;
                            } // no matching branch — skip
                        }
                    }
                    _ => &items[i],
                };
                match &item.kind {
                    FormKind::Keyword(k) if k == "as" => {
                        i += 1;
                        alias = Some(match items.get(i).map(|f| &f.kind) {
                            Some(FormKind::Symbol(s)) => Arc::from(s.as_str()),
                            _ => return Err("require :as expects a symbol".into()),
                        });
                    }
                    FormKind::Keyword(k) if k == "refer" => {
                        i += 1;
                        refer = match items.get(i).map(|f| &f.kind) {
                            Some(FormKind::Keyword(k2)) if k2 == "all" => RequireRefer::All,
                            Some(FormKind::Vector(rv)) => {
                                let names: Vec<Arc<str>> = rv
                                    .iter()
                                    .map(|f| match &f.kind {
                                        FormKind::Symbol(s) => Ok(Arc::from(s.as_str())),
                                        _ => Err("require :refer expects symbols".to_string()),
                                    })
                                    .collect::<Result<_, _>>()?;
                                RequireRefer::Named(names)
                            }
                            _ => return Err("require :refer expects :all or a vector".into()),
                        };
                    }
                    _ => return Err(format!("require spec: unexpected form at position {i}")),
                }
                i += 1;
            }
            Ok(RequireSpec { ns, alias, refer })
        }
        _ => Err("require spec must be a symbol or vector".into()),
    }
}

// ── ns ────────────────────────────────────────────────────────────────────────

fn eval_ns(args: &[Form], env: &mut Env) -> EvalResult {
    let name = require_sym(args, 0, "ns")?;
    env.globals.get_or_create_ns(name);
    env.current_ns = Arc::from(name);
    // Auto-refer clojure.core (Clojure default behaviour).
    if name != "clojure.core" {
        env.globals.refer_all(name, "clojure.core");
    }
    sync_star_ns(env);

    for clause in &args[1..] {
        if let FormKind::List(items) = &clause.kind {
            match items.first().map(|f| &f.kind) {
                Some(FormKind::Keyword(k)) if k == "require" => {
                    // Expand reader conditionals among require specs
                    let expanded = expand_reader_conds(&items[1..]);
                    for spec_form in &expanded {
                        let spec =
                            parse_require_spec_form(spec_form).map_err(EvalError::Runtime)?;
                        load_ns(env.globals.clone(), &spec, name)?;
                    }
                }
                // Other clauses (:refer-clojure, :use, :import) — skip for now.
                _ => {}
            }
        }
    }

    let ns_ptr = env.globals.get_or_create_ns(&env.current_ns);
    Ok(Value::Namespace(ns_ptr))
}

// ── load-file ─────────────────────────────────────────────────────────────────

fn eval_load_file(args: &[Form], env: &mut Env) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Runtime(
            "load-file requires a path argument".into(),
        ));
    }
    let path_val = eval(&args[0], env)?;
    let path = match &path_val {
        Value::Str(s) => s.get().clone(),
        v => {
            return Err(EvalError::Runtime(format!(
                "load-file: expected string, got {}",
                v.type_name()
            )));
        }
    };
    let src = std::fs::read_to_string(&path)
        .map_err(|e| EvalError::Runtime(format!("load-file: {e}")))?;
    let mut parser = cljrs_reader::Parser::new(src, path.clone());
    let forms = parser
        .parse_all()
        .map_err(|e| EvalError::Runtime(format!("load-file parse error: {e}")))?;
    let mut result = Value::Nil;
    for form in forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        result = eval(&form, env)?;
    }
    Ok(result)
}

// ── letfn ─────────────────────────────────────────────────────────────────────

fn eval_letfn(args: &[Form], env: &mut Env) -> EvalResult {
    // (letfn [(f [params] body...) ...] body...)
    let bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => v.clone(),
        _ => return Err(EvalError::Runtime("letfn requires a binding vector".into())),
    };

    env.push_frame();

    for binding in &bindings {
        if let FormKind::List(parts) = &binding.kind {
            if parts.is_empty() {
                continue;
            }
            // parts[0] = name, parts[1] = params, parts[2..] = body
            // Reuse eval_fn: it expects (optional-name params body...)
            // We pass parts directly since parts[0] is the function name symbol.
            let fn_val = match eval_fn(parts, env) {
                Ok(v) => v,
                Err(e) => {
                    env.pop_frame();
                    return Err(e);
                }
            };
            let name = match &parts[0].kind {
                FormKind::Symbol(s) => s.clone(),
                _ => {
                    env.pop_frame();
                    return Err(EvalError::Runtime(
                        "letfn binding name must be a symbol".into(),
                    ));
                }
            };
            env.bind(Arc::from(name.as_str()), fn_val);
        }
    }

    let body = &args[1..];
    let result = eval_body(body, env);
    env.pop_frame();
    result
}

// ── in-ns ─────────────────────────────────────────────────────────────────────

fn eval_in_ns(args: &[Form], env: &mut Env) -> EvalResult {
    // (in-ns 'foo.bar)
    if args.is_empty() {
        return Err(EvalError::Runtime("in-ns requires a namespace name".into()));
    }
    let ns_val = eval(&args[0], env)?;
    let ns_name = extract_ns_name(&ns_val)?;
    env.globals.get_or_create_ns(&ns_name);
    env.globals.refer_all(&ns_name, "clojure.core");
    env.current_ns = Arc::from(ns_name.as_str());
    sync_star_ns(env);
    let ns_ptr = env.globals.get_or_create_ns(&env.current_ns);
    Ok(Value::Namespace(ns_ptr))
}

// ── alias ─────────────────────────────────────────────────────────────────────

fn eval_alias(args: &[Form], env: &mut Env) -> EvalResult {
    // (alias 'short 'some.long.ns)
    if args.len() < 2 {
        return Err(EvalError::Runtime(
            "alias requires alias-sym and namespace-sym".into(),
        ));
    }
    let alias_val = eval(&args[0], env)?;
    let ns_val = eval(&args[1], env)?;

    let alias_name = extract_ns_name(&alias_val)?;
    let full_ns = extract_ns_name(&ns_val)?;

    let ns_ptr = env.globals.get_or_create_ns(&env.current_ns);
    let mut aliases = ns_ptr.get().aliases.lock().unwrap();
    aliases.insert(Arc::from(alias_name.as_str()), Arc::from(full_ns.as_str()));
    Ok(Value::Nil)
}

/// Extract a namespace-name string from a Value::Symbol, Value::Str, or Value::Keyword.
fn extract_ns_name(v: &Value) -> EvalResult<String> {
    match v {
        Value::Symbol(s) => {
            // Use the full name (e.g. "clojure.core").
            Ok(s.get().name.as_ref().to_string())
        }
        Value::Str(s) => Ok(s.get().clone()),
        Value::Keyword(k) => Ok(k.get().name.as_ref().to_string()),
        other => Err(EvalError::Runtime(format!(
            "expected a symbol or string for namespace name, got {}",
            other.type_name()
        ))),
    }
}

// ── defprotocol ───────────────────────────────────────────────────────────────

fn eval_defprotocol(args: &[Form], env: &mut Env) -> EvalResult {
    // (defprotocol Name "doc?" (method [this & args] "doc?") ...)
    let name = require_sym(args, 0, "defprotocol")?;
    let proto_name: Arc<str> = Arc::from(name);

    // Skip optional docstring.
    let methods_start = if args.len() > 1 && matches!(args[1].kind, FormKind::Str(_)) {
        2
    } else {
        1
    };

    let mut methods: Vec<ProtocolMethod> = Vec::new();

    for form in &args[methods_start..] {
        // Each method spec is (method-name [params...] "doc"?)
        let parts = match &form.kind {
            FormKind::List(parts) => parts,
            _ => continue, // skip unknown forms
        };
        if parts.is_empty() {
            continue;
        }
        let method_name = match &parts[0].kind {
            FormKind::Symbol(s) => Arc::from(s.as_str()),
            _ => continue,
        };
        // Find the parameter vector (first vector in parts).
        let (min_arity, variadic) = if let Some(params_form) =
            parts.iter().find(|f| matches!(f.kind, FormKind::Vector(_)))
        {
            if let FormKind::Vector(param_forms) = &params_form.kind {
                let variadic = param_forms
                    .iter()
                    .any(|p| matches!(&p.kind, FormKind::Symbol(s) if s == "&"));
                let fixed: usize = param_forms
                    .iter()
                    .filter(|p| !matches!(&p.kind, FormKind::Symbol(s) if s == "&"))
                    .count();
                (fixed, variadic)
            } else {
                (1, false)
            }
        } else {
            (1, false)
        };
        methods.push(ProtocolMethod {
            name: method_name,
            min_arity,
            variadic,
        });
    }

    let ns: Arc<str> = env.current_ns.clone();
    let proto = Protocol::new(proto_name.clone(), ns, methods.clone());
    let proto_ptr = GcPtr::new(proto);

    // Intern the protocol itself.
    let proto_var = env.globals.intern(
        &env.current_ns,
        proto_name.clone(),
        Value::Protocol(proto_ptr.clone()),
    );

    // Create and intern a ProtocolFn for each method.
    for method in &methods {
        let pf = ProtocolFn {
            protocol: proto_ptr.clone(),
            method_name: method.name.clone(),
            min_arity: method.min_arity,
            variadic: method.variadic,
        };
        env.globals.intern(
            &env.current_ns,
            method.name.clone(),
            Value::ProtocolFn(GcPtr::new(pf)),
        );
    }

    Ok(Value::Var(proto_var))
}

// ── extend-type ───────────────────────────────────────────────────────────────

fn eval_extend_type(args: &[Form], env: &mut Env) -> EvalResult {
    // (extend-type TypeSym Proto1 (m [this] body) ... Proto2 ...)
    if args.is_empty() {
        return Err(EvalError::Runtime(
            "extend-type requires a type symbol".into(),
        ));
    }
    let type_sym = match &args[0].kind {
        FormKind::Symbol(s) => s.clone(),
        _ => {
            return Err(EvalError::Runtime(
                "extend-type: first arg must be a type symbol".into(),
            ));
        }
    };
    let type_tag = crate::apply::resolve_type_tag(&type_sym);

    let mut current_proto: Option<GcPtr<Protocol>> = None;

    for form in &args[1..] {
        match &form.kind {
            FormKind::Symbol(s) => {
                // Look up protocol in env.
                let val = env.globals.lookup_in_ns(&env.current_ns, s);
                match val {
                    Some(Value::Protocol(p)) => {
                        current_proto = Some(p);
                    }
                    _ => {
                        return Err(EvalError::Runtime(format!(
                            "extend-type: {} is not a protocol",
                            s
                        )));
                    }
                }
            }
            FormKind::List(parts) => {
                // (method-name [params] body...)
                let proto = current_proto.as_ref().ok_or_else(|| {
                    EvalError::Runtime("extend-type: method before protocol name".into())
                })?;
                if parts.is_empty() {
                    continue;
                }
                let method_name = match &parts[0].kind {
                    FormKind::Symbol(s) => Arc::from(s.as_str()),
                    _ => continue,
                };
                let fn_val = build_impl_fn(parts, env)?;
                let mut impls = proto.get().impls.lock().unwrap();
                impls
                    .entry(type_tag.clone())
                    .or_default()
                    .insert(method_name, fn_val);
            }
            _ => {}
        }
    }

    Ok(Value::Nil)
}

// ── extend-protocol ───────────────────────────────────────────────────────────

fn eval_extend_protocol(args: &[Form], env: &mut Env) -> EvalResult {
    // (extend-protocol Proto Type1 (m [this] body) ... Type2 ...)
    if args.is_empty() {
        return Err(EvalError::Runtime(
            "extend-protocol requires a protocol".into(),
        ));
    }
    let proto_sym = match &args[0].kind {
        FormKind::Symbol(s) => s.clone(),
        _ => {
            return Err(EvalError::Runtime(
                "extend-protocol: first arg must be a protocol symbol".into(),
            ));
        }
    };
    let proto_val = env.globals.lookup_in_ns(&env.current_ns, &proto_sym);
    let proto_ptr = match proto_val {
        Some(Value::Protocol(p)) => p,
        _ => {
            return Err(EvalError::Runtime(format!(
                "extend-protocol: {} is not a protocol",
                proto_sym
            )));
        }
    };

    let mut current_type: Option<Arc<str>> = None;

    for form in &args[1..] {
        match &form.kind {
            FormKind::Symbol(s) => {
                current_type = Some(crate::apply::resolve_type_tag(s));
            }
            FormKind::List(parts) => {
                let type_tag = current_type.as_ref().ok_or_else(|| {
                    EvalError::Runtime("extend-protocol: method before type name".into())
                })?;
                if parts.is_empty() {
                    continue;
                }
                let method_name = match &parts[0].kind {
                    FormKind::Symbol(s) => Arc::from(s.as_str()),
                    _ => continue,
                };
                let fn_val = build_impl_fn(parts, env)?;
                let mut impls = proto_ptr.get().impls.lock().unwrap();
                impls
                    .entry(type_tag.clone())
                    .or_default()
                    .insert(method_name, fn_val);
            }
            _ => {}
        }
    }

    Ok(Value::Nil)
}

/// Build a `CljxFn` from the tail of a method-impl list: `(name [params] body...)`.
/// `parts[0]` is the method name symbol (ignored here — caller handles it).
/// `parts[1]` is the params vector.
/// `parts[2..]` is the body.
fn build_impl_fn(parts: &[Form], env: &mut Env) -> EvalResult<Value> {
    if parts.len() < 2 {
        return Err(EvalError::Runtime(
            "protocol method impl requires params and body".into(),
        ));
    }
    // parts[1] should be the params vector.
    let params_form = &parts[1];
    let body = &parts[2..];
    let arity = parse_arity(params_form, body)?;
    let (closed_over_names, closed_over_vals) = env.all_local_bindings();
    let fn_name = match &parts[0].kind {
        FormKind::Symbol(s) => Some(Arc::from(s.as_str())),
        _ => None,
    };
    let cljrs_fn = CljxFn::new(
        fn_name,
        vec![arity],
        closed_over_names,
        closed_over_vals,
        false,
        Arc::clone(&env.current_ns),
    );
    Ok(Value::Fn(GcPtr::new(cljrs_fn)))
}

// ── defmulti ──────────────────────────────────────────────────────────────────

fn eval_defmulti(args: &[Form], env: &mut Env) -> EvalResult {
    // (defmulti name dispatch-fn-form) or (defmulti name "doc" dispatch-fn :default val)
    let name = require_sym(args, 0, "defmulti")?;
    let name_arc: Arc<str> = Arc::from(name);

    let rest_start = if args.len() > 2 && matches!(args[1].kind, FormKind::Str(_)) {
        2
    } else {
        1
    };

    if args.len() <= rest_start {
        return Err(EvalError::Runtime(
            "defmulti requires a dispatch function".into(),
        ));
    }

    let dispatch_fn = eval(&args[rest_start], env)?;

    // Parse optional :default val.
    let mut default_dispatch = ":default".to_string();
    let mut i = rest_start + 1;
    while i + 1 < args.len() {
        if let FormKind::Keyword(k) = &args[i].kind
            && k == "default"
        {
            let dv = eval(&args[i + 1], env)?;
            default_dispatch = format!("{}", dv);
        }
        i += 2;
    }

    let mfn = MultiFn::new(name_arc.clone(), dispatch_fn, default_dispatch);
    let var = env
        .globals
        .intern(&env.current_ns, name_arc, Value::MultiFn(GcPtr::new(mfn)));
    Ok(Value::Var(var))
}

// ── defmethod ─────────────────────────────────────────────────────────────────

fn eval_defmethod(args: &[Form], env: &mut Env) -> EvalResult {
    // (defmethod multi-name dispatch-val [params] body...)
    if args.len() < 3 {
        return Err(EvalError::Runtime(
            "defmethod requires name, dispatch-val, params, and body".into(),
        ));
    }
    let multi_name = require_sym(args, 0, "defmethod")?;

    let mf_ptr = match env.globals.lookup_in_ns(&env.current_ns, multi_name) {
        Some(Value::MultiFn(mf)) => mf,
        _ => {
            return Err(EvalError::Runtime(format!(
                "defmethod: {} is not a multimethod",
                multi_name
            )));
        }
    };

    let dispatch_val = eval(&args[1], env)?;
    let key = format!("{}", dispatch_val);

    // Build CljxFn from ([params] body...).
    let params_form = &args[2];
    let body = &args[3..];
    let arity = parse_arity(params_form, body)?;
    let (closed_over_names, closed_over_vals) = env.all_local_bindings();
    let fn_name = Some(Arc::from(multi_name));
    let cljrs_fn = CljxFn::new(
        fn_name,
        vec![arity],
        closed_over_names,
        closed_over_vals,
        false,
        Arc::clone(&env.current_ns),
    );
    let fn_val = Value::Fn(GcPtr::new(cljrs_fn));

    mf_ptr.get().methods.lock().unwrap().insert(key, fn_val);

    Ok(Value::MultiFn(mf_ptr))
}

// ── binding ───────────────────────────────────────────────────────────────────

fn eval_binding(args: &[Form], env: &mut Env) -> EvalResult {
    let pairs = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => v.clone(),
        _ => return Err(EvalError::Runtime("binding requires a vector".into())),
    };
    if pairs.len() % 2 != 0 {
        return Err(EvalError::Runtime(
            "binding vector must have even count".into(),
        ));
    }

    let mut frame: HashMap<usize, Value> = HashMap::new();
    for pair in pairs.chunks(2) {
        let sym_str = match &pair[0].kind {
            FormKind::Symbol(s) => s.clone(),
            _ => return Err(EvalError::Runtime("binding targets must be symbols".into())),
        };
        let parsed = cljrs_value::Symbol::parse(&sym_str);
        let ns_part = parsed
            .namespace
            .as_deref()
            .unwrap_or(env.current_ns.as_ref());
        let var_ptr = env
            .globals
            .lookup_var_in_ns(ns_part, &parsed.name)
            .ok_or_else(|| EvalError::UnboundSymbol(sym_str.clone()))?;
        let val = eval(&pair[1], env)?;
        frame.insert(cljrs_env::dynamics::var_key_of(&var_ptr), val);
    }

    let _guard = cljrs_env::dynamics::push_frame(frame);
    eval_body(&args[1..], env)
    // _guard drops here → pop_frame()
}

// ── defrecord ─────────────────────────────────────────────────────────────────

fn eval_defrecord(args: &[Form], env: &mut Env) -> EvalResult {
    // (defrecord TypeName [field1 field2 ...] Proto1 (method [this] body) ...)
    if args.len() < 2 {
        return Err(EvalError::Runtime(
            "defrecord requires a name and field vector".into(),
        ));
    }
    let type_name = require_sym(args, 0, "defrecord")?;
    let type_tag: Arc<str> = Arc::from(type_name);

    // Parse field names from the vector.
    let field_names: Vec<Arc<str>> = match &args[1].kind {
        FormKind::Vector(fields) => fields
            .iter()
            .map(|f| match &f.kind {
                FormKind::Symbol(s) => Ok(Arc::from(s.as_str())),
                _ => Err(EvalError::Runtime(
                    "defrecord field names must be symbols".into(),
                )),
            })
            .collect::<EvalResult<_>>()?,
        _ => {
            return Err(EvalError::Runtime(
                "defrecord requires a field vector as second arg".into(),
            ));
        }
    };

    // Register protocol implementations (same as extend-type inner logic).
    register_impls_for_tag(&type_tag, &args[2..], env)?;

    // Generate constructors in clojure.core.
    // ->TypeName: positional constructor
    // map->TypeName: map constructor
    let ns = env.current_ns.clone();
    let globals = env.globals.clone();
    let field_names_clone = field_names.clone();
    let type_tag2 = type_tag.clone();

    // Build `->TypeName` as a native-Clojure fn: (fn [f1 f2 ...] (make-type-instance "T" {:f1 f1 :f2 f2 ...}))
    {
        let params: Vec<Arc<str>> = field_names.clone();
        let rest_param = None;
        // Build body forms manually: (make-type-instance "TypeName" {:field1 field1 ...})
        use cljrs_reader::form::FormKind as FK;
        let dummy_span =
            cljrs_types::span::Span::new(std::sync::Arc::new("<defrecord>".into()), 0, 0, 1, 1);
        let make_form = |kind: FK| Form {
            kind,
            span: dummy_span.clone(),
        };
        let mut kv_forms: Vec<Form> = Vec::new();
        for f in &field_names {
            kv_forms.push(make_form(FK::Keyword(f.as_ref().to_string())));
            kv_forms.push(make_form(FK::Symbol(f.as_ref().to_string())));
        }
        let map_form = make_form(FK::Map(kv_forms));
        let body = vec![make_form(FK::List(vec![
            make_form(FK::Symbol("make-type-instance".into())),
            make_form(FK::Str(type_tag.as_ref().to_string())),
            map_form,
        ]))];
        let arity = CljxFnArity {
            params,
            rest_param,
            body,
            destructure_params: vec![],
            destructure_rest: None,
            ir_arity_id: crate::arity::fresh_arity_id(),
        };
        let fn_name: Arc<str> = Arc::from(format!("->{}", type_name));
        let ctor = CljxFn::new(
            Some(fn_name.clone()),
            vec![arity],
            vec![],
            vec![],
            false,
            Arc::clone(&ns),
        );
        globals.intern(&ns, fn_name, Value::Fn(GcPtr::new(ctor)));
    }

    // Build `map->TypeName`: (fn [m] (make-type-instance "TypeName" m))
    {
        use cljrs_reader::form::FormKind as FK;
        let dummy_span =
            cljrs_types::span::Span::new(std::sync::Arc::new("<defrecord>".into()), 0, 0, 1, 1);
        let make_form = |kind: FK| Form {
            kind,
            span: dummy_span.clone(),
        };
        let m_sym: Arc<str> = Arc::from("m__");
        let body = vec![make_form(FK::List(vec![
            make_form(FK::Symbol("make-type-instance".into())),
            make_form(FK::Str(type_tag2.as_ref().to_string())),
            make_form(FK::Symbol(m_sym.as_ref().to_string())),
        ]))];
        let arity = CljxFnArity {
            params: vec![m_sym],
            rest_param: None,
            body,
            destructure_params: vec![],
            destructure_rest: None,
            ir_arity_id: crate::arity::fresh_arity_id(),
        };
        let fn_name: Arc<str> = Arc::from(format!("map->{}", type_name));
        let ctor = CljxFn::new(
            Some(fn_name.clone()),
            vec![arity],
            vec![],
            vec![],
            false,
            Arc::clone(&ns),
        );
        globals.intern(&ns, fn_name, Value::Fn(GcPtr::new(ctor)));
    }

    // Intern the type name as a Symbol value so `(instance? TypeName x)` works.
    let _ = field_names_clone;
    let type_sym = cljrs_value::Symbol::simple(type_name);
    globals.intern(
        &ns,
        Arc::from(type_name),
        Value::Symbol(GcPtr::new(type_sym)),
    );
    Ok(Value::Nil)
}

// ── reify ─────────────────────────────────────────────────────────────────────

fn eval_reify(args: &[Form], env: &mut Env) -> EvalResult {
    // (reify Proto1 (method [this] body) ...)
    // Generate a unique type tag for this instance.
    let n =
        cljrs_builtins::builtins::GENSYM_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let type_tag: Arc<str> = Arc::from(format!("reify__{}", n));

    // Register protocol implementations.
    register_impls_for_tag(&type_tag, args, env)?;

    // Return an empty TypeInstance with the unique tag.
    Ok(Value::TypeInstance(GcPtr::new(TypeInstance {
        type_tag,
        fields: MapValue::empty(),
    })))
}

// ── register_impls_for_tag ────────────────────────────────────────────────────

/// Parse `Proto (method [params] body) ...` segments and register them under `type_tag`.
/// Shared by `defrecord` and `reify`.
fn register_impls_for_tag(type_tag: &Arc<str>, forms: &[Form], env: &mut Env) -> EvalResult<()> {
    let mut current_proto: Option<GcPtr<cljrs_value::Protocol>> = None;

    for form in forms {
        match &form.kind {
            FormKind::Symbol(s) => {
                let val = env.globals.lookup_in_ns(&env.current_ns, s);
                match val {
                    Some(Value::Protocol(p)) => {
                        current_proto = Some(p);
                    }
                    _ => {
                        return Err(EvalError::Runtime(format!(
                            "reify/defrecord: {} is not a protocol",
                            s
                        )));
                    }
                }
            }
            FormKind::List(parts) => {
                let proto = current_proto.as_ref().ok_or_else(|| {
                    EvalError::Runtime("reify/defrecord: method impl before protocol name".into())
                })?;
                if parts.is_empty() {
                    continue;
                }
                let method_name = match &parts[0].kind {
                    FormKind::Symbol(s) => Arc::from(s.as_str()),
                    _ => continue,
                };
                let fn_val = build_impl_fn(parts, env)?;
                let mut impls = proto.get().impls.lock().unwrap();
                impls
                    .entry(type_tag.clone())
                    .or_default()
                    .insert(method_name, fn_val);
            }
            _ => {}
        }
    }
    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Update the root binding of `*ns*` in `clojure.core` to the current namespace.
/// Called whenever `env.current_ns` changes (ns, in-ns, standard_env setup).
pub fn sync_star_ns(env: &mut Env) {
    if let Some(star_ns_var) = env.globals.lookup_var("clojure.core", "*ns*") {
        let ns_ptr = env.globals.get_or_create_ns(&env.current_ns);
        star_ns_var.get().bind(Value::Namespace(ns_ptr));
    }
}

fn require_sym<'a>(args: &'a [Form], idx: usize, form_name: &str) -> EvalResult<&'a str> {
    match args.get(idx).map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => Ok(s.as_str()),
        _ => Err(EvalError::Runtime(format!(
            "{form_name} requires a symbol at position {idx}"
        ))),
    }
}

// ── with-out-str ──────────────────────────────────────────────────────────────

fn eval_with_out_str(body: &[Form], env: &mut Env) -> EvalResult {
    cljrs_builtins::builtins::push_output_capture();
    let result = eval_body(body, env);
    let captured = cljrs_builtins::builtins::pop_output_capture().unwrap_or_default();
    // Propagate errors but still pop the capture buffer
    result?;
    Ok(Value::string(captured))
}
