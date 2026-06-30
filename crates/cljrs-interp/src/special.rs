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
    CljxFn, CljxFnArity, FutureState, Keyword, MapValue, MultiFn, Protocol, ProtocolFn,
    ProtocolMethod, TypeHint, TypeInstance, Value, ValueError,
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
        "await" => eval_await(args, env),
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

/// Does a `^meta` form (or metadata map literal) request `:async`?
///
/// Handles the keyword shorthand `^:async` (a bare `:async` keyword form) and
/// an explicit map such as `^{:async true}` or a `defn` attr-map `{:async true}`.
pub fn meta_form_is_async(meta: &Form) -> bool {
    match &meta.kind {
        FormKind::Keyword(k) => k == "async",
        FormKind::Map(entries) => entries.chunks(2).any(|kv| {
            matches!(&kv[0].kind, FormKind::Keyword(k) if k == "async")
                && !matches!(
                    kv.get(1).map(|f| &f.kind),
                    None | Some(FormKind::Bool(false)) | Some(FormKind::Nil)
                )
        }),
        _ => false,
    }
}

fn eval_fn(args: &[Form], env: &mut Env) -> EvalResult {
    // Peel any leading `^meta` wrappers, e.g. `(fn ^:async [..] ..)` or
    // `(fn ^:async name [..] ..)`, recording whether `:async` was requested.
    let mut is_async = false;
    let peeled: Vec<Form>;
    let args: &[Form] = if matches!(args.first().map(|f| &f.kind), Some(FormKind::Meta(..))) {
        let mut head = args[0].clone();
        while let FormKind::Meta(meta, inner) = head.kind {
            is_async |= meta_form_is_async(&meta);
            head = *inner;
        }
        peeled = std::iter::once(head)
            .chain(args[1..].iter().cloned())
            .collect();
        &peeled
    } else {
        args
    };

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

    let mut cljrs_fn = CljxFn::new(
        name.clone(),
        arities,
        closed_over_names,
        closed_over_vals,
        false,
        Arc::clone(&env.current_ns),
    );
    cljrs_fn.is_async = is_async;

    env.on_fn_defined(&cljrs_fn);

    // Eagerly lower each arity to IR if the compiler is ready.
    //eager_lower_fn(&cljrs_fn, env);

    let mut ptr = GcPtr::new(cljrs_fn);
    // For named anonymous functions (fn g ...), store a back-pointer so that
    // the self-reference returned from the body is pointer-equal to the outer
    // binding — required for `(= f (f))` to be `true`.
    if ptr.get().name.is_some() {
        let self_clone = ptr.clone();
        ptr.get_mut().self_ptr = Some(self_clone);
    }
    Ok(Value::Fn(ptr))
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
    let mut param_hints: Vec<Option<TypeHint>> = Vec::new();
    let mut rest_param: Option<Arc<str>> = None;
    let mut rest_hint: Option<TypeHint> = None;
    let mut destructure_params: Vec<(usize, Form)> = Vec::new();
    let mut destructure_rest: Option<Form> = None;
    let mut saw_amp = false;

    for p in param_forms {
        // Peel a leading `^hint` (e.g. `^long x`, `^doubles a`) from the param,
        // resolving the `:tag` to a primitive `TypeHint`.  Unknown / non-primitive
        // tags resolve to `None` and are simply ignored (Clojure treats them as
        // advisory).  The unwrapped form keeps the existing symbol/destructure
        // handling unchanged.
        let (hint, p) = peel_param_hint(p);
        match &p.kind {
            FormKind::Symbol(s) if s == "&" => {
                saw_amp = true;
            }
            FormKind::Symbol(s) => {
                if saw_amp {
                    rest_param = Some(Arc::from(s.as_str()));
                    rest_hint = hint;
                    break;
                } else {
                    params.push(Arc::from(s.as_str()));
                    param_hints.push(hint);
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
                    // Destructured params can't be primitive-tagged.
                    param_hints.push(None);
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

    let body = desugar_pre_post_conditions(body);

    Ok(CljxFnArity {
        params,
        rest_param,
        body,
        destructure_params,
        destructure_rest,
        ir_arity_id: crate::arity::fresh_arity_id(),
        param_hints,
        rest_hint,
    })
}

/// Desugar a `:pre`/`:post` conditions map at the head of a function body.
///
/// Clojure binds `%` to the return value inside `:post` conditions. This
/// transforms the raw body forms into equivalent assertion code so the
/// interpreter does not need to handle conditions separately at call time.
///
/// Input body (simplified):
/// ```text
/// [{:pre [(pos? x)] :post [(pos? %)]} (inc x)]
/// ```
/// Output body:
/// ```text
/// [(assert (pos? x))
///  (let* [% (inc x)]
///    (assert (pos? %))
///    %)]
/// ```
fn desugar_pre_post_conditions(body: &[Form]) -> Vec<Form> {
    let first = match body.first() {
        Some(f) => f,
        None => return body.to_vec(),
    };

    let entries = match &first.kind {
        FormKind::Map(entries) => entries,
        _ => return body.to_vec(),
    };

    let mut pre_conds: Vec<Form> = Vec::new();
    let mut post_conds: Vec<Form> = Vec::new();
    let mut has_conditions = false;

    for chunk in entries.chunks(2) {
        if chunk.len() < 2 {
            break;
        }
        let (key, val) = (&chunk[0], &chunk[1]);
        match &key.kind {
            FormKind::Keyword(k) if k == "pre" => {
                if let FormKind::Vector(conds) = &val.kind {
                    pre_conds.extend_from_slice(conds);
                    has_conditions = true;
                }
            }
            FormKind::Keyword(k) if k == "post" => {
                if let FormKind::Vector(conds) = &val.kind {
                    post_conds.extend_from_slice(conds);
                    has_conditions = true;
                }
            }
            _ => {}
        }
    }

    if !has_conditions {
        return body.to_vec();
    }

    let real_body = &body[1..]; // strip the conditions map
    let span = first.span.clone();
    let mut new_body: Vec<Form> = Vec::new();

    // Emit (assert cond) for each :pre condition.
    for cond in &pre_conds {
        new_body.push(make_assert_call(cond, &span));
    }

    if post_conds.is_empty() {
        new_body.extend_from_slice(real_body);
    } else {
        // Wrap real body in (let* [% <body>] (assert post-cond)... %)
        // so that % refers to the return value inside :post conditions.
        let percent = Form::new(FormKind::Symbol("%".to_string()), span.clone());

        let body_expr = if real_body.len() == 1 {
            real_body[0].clone()
        } else {
            let mut do_forms = vec![Form::new(FormKind::Symbol("do".to_string()), span.clone())];
            do_forms.extend_from_slice(real_body);
            Form::new(FormKind::List(do_forms), span.clone())
        };

        let binding_vec = Form::new(
            FormKind::Vector(vec![percent.clone(), body_expr]),
            span.clone(),
        );

        let mut let_forms = vec![
            Form::new(FormKind::Symbol("let*".to_string()), span.clone()),
            binding_vec,
        ];
        for cond in &post_conds {
            let_forms.push(make_assert_call(cond, &span));
        }
        let_forms.push(percent);

        new_body.push(Form::new(FormKind::List(let_forms), span));
    }

    new_body
}

/// Build `(assert <cond>)` as a Form node.
fn make_assert_call(cond: &Form, span: &cljrs_types::span::Span) -> Form {
    Form::new(
        FormKind::List(vec![
            Form::new(FormKind::Symbol("assert".to_string()), span.clone()),
            cond.clone(),
        ]),
        span.clone(),
    )
}

/// Peel a `^hint` wrapper off a parameter form, returning the resolved
/// primitive [`TypeHint`] (if the tag is a recognized primitive) together with
/// the unwrapped inner form.  A param with no metadata returns `(None, form)`.
fn peel_param_hint(p: &Form) -> (Option<TypeHint>, &Form) {
    if let FormKind::Meta(meta, inner) = &p.kind {
        let hint = tag_name_of_meta(meta).and_then(|n| TypeHint::from_tag(&n));
        // Recurse in case of stacked metadata; the innermost form is the param.
        let (inner_hint, inner_form) = peel_param_hint(inner);
        (hint.or(inner_hint), inner_form)
    } else {
        (None, p)
    }
}

/// Extract the bare tag name from a `^meta` form: `^long` (symbol shorthand) or
/// `^{:tag long}` (explicit map).  Strips any namespace from the tag symbol.
fn tag_name_of_meta(meta: &Form) -> Option<Arc<str>> {
    match &meta.kind {
        // `^long x` — the metadata is a bare symbol naming the tag.
        FormKind::Symbol(s) => Some(strip_tag_ns(s)),
        // `^{:tag long} x` — pull the `:tag` entry out of the map literal.
        FormKind::Map(entries) => entries.chunks(2).find_map(|kv| {
            let is_tag = matches!(&kv[0].kind, FormKind::Keyword(k) if k == "tag");
            if !is_tag {
                return None;
            }
            match kv.get(1).map(|f| &f.kind) {
                Some(FormKind::Symbol(s)) => Some(strip_tag_ns(s)),
                Some(FormKind::Str(s)) => Some(strip_tag_ns(s)),
                _ => None,
            }
        }),
        _ => None,
    }
}

/// Strip a namespace prefix from a tag name (`clojure.core/long` → `long`).
fn strip_tag_ns(s: &str) -> Arc<str> {
    match s.rfind('/') {
        Some(pos) if pos + 1 < s.len() => Arc::from(&s[pos + 1..]),
        _ => Arc::from(s),
    }
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
    let raw_bindings = match args.first().map(|f| &f.kind) {
        Some(FormKind::Vector(v)) => v.clone(),
        _ => return Err(EvalError::Runtime("let* requires a binding vector".into())),
    };
    let bindings = if raw_bindings
        .iter()
        .any(|f| matches!(f.kind, FormKind::ReaderCond { .. }))
    {
        expand_reader_conds(&raw_bindings)
    } else {
        raw_bindings
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

        // Under GC: scope this iteration's heap allocations in a fresh alloc
        // frame.  Every value the body allocates is rooted (via ALLOC_ROOTS)
        // only until this frame drops at the end of the iteration; the
        // intermediates — and the previous iteration's now-dead recur values —
        // then become collectable, instead of being pinned for the lifetime of
        // the enclosing top-level form.  `result` (the return value or recur
        // values) is moved out before the frame drops and is re-rooted at the
        // top of the next iteration (`root_values`) or by the caller during
        // return unwinding; no GC safepoint runs in the interval, exactly as
        // the IR/JIT dispatch seam relies on (see cljrs-eval `apply.rs`).
        #[cfg(not(feature = "no-gc"))]
        let _iter_frame = cljrs_gc::push_alloc_frame();

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
        // scratch / _iter_frame drop at the end of the iteration (after the
        // match below), freeing this iteration's intermediates.

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
    let ns: Arc<str> = match parsed.namespace.as_deref() {
        Some(ns_part) => env
            .globals
            .resolve_alias(&env.current_ns, ns_part)
            .unwrap_or_else(|| Arc::from(ns_part)),
        None => env.current_ns.clone(),
    };
    let name = parsed.name.as_ref();
    env.globals
        .lookup_var_in_ns(&ns, name)
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

/// A catch clause: `(catch Type binding body...)`.
///
/// Public so the async evaluator (`cljrs-async`) can reuse `parse_try_args` to
/// build a yielding `try`/`catch`.
pub struct CatchClause<'a> {
    pub type_sym: &'a str,
    pub binding: &'a str,
    pub body: &'a [Form],
}

/// Convert a non-Thrown EvalError into a `Value::Error` so it can be bound
/// inside a catch clause and inspected with `ex-message` / `ex-data`.
///
/// Public so the async evaluator can convert errors the same way `try` does.
pub fn eval_error_to_value(err: &EvalError) -> Value {
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
pub fn catch_type_matches(type_name: &str, val: &Value) -> bool {
    let short = type_name.rsplit('.').next().unwrap_or(type_name);
    match short {
        // Catch-all (matches any value, error or not — back-compat).
        // `:default` is the ClojureScript universal catch keyword; it arrives
        // here as the keyword's name ("default") via `parse_try_args`.
        "Object" | "Exception" | "Throwable" | "Error" | "default" => true,
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

    // Always run finally.  Its normal (Ok) value is discarded — `try` returns
    // the body/catch result — but an exception thrown from `finally` supersedes
    // the pending result or exception (matching JVM/Clojure semantics).
    if !fin_body.is_empty() {
        eval_body(fin_body, env)?;
    }

    result
}

/// Split try args into (body, catch clauses, finally body).
///
/// Public so the async evaluator can parse `try` forms identically.
pub fn parse_try_args(args: &[Form]) -> (&[Form], Vec<CatchClause<'_>>, &[Form]) {
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
                // The catch type may be a symbol (`Throwable`, `java.lang.Exception`),
                // the ClojureScript `:default` catch-all keyword, or a reader
                // conditional `#?(:rust Exception :clj ...)` whose selected branch
                // resolves to one of those.
                let type_sym = match parts.get(1).map(|f| &f.kind) {
                    Some(FormKind::Symbol(s)) => s.as_str(),
                    Some(FormKind::Keyword(s)) => s.as_str(),
                    Some(FormKind::ReaderCond {
                        splicing: false,
                        clauses,
                    }) => match select_reader_cond(clauses).map(|f| &f.kind) {
                        Some(FormKind::Symbol(s)) => s.as_str(),
                        Some(FormKind::Keyword(s)) => s.as_str(),
                        _ => continue,
                    },
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
    // The name may carry reader metadata, e.g. `(defn ^:async fetch ...)`.
    let (name, mut is_async) = match args.first().map(|f| &f.kind) {
        Some(FormKind::Symbol(s)) => (s.clone(), false),
        Some(FormKind::Meta(meta, inner)) => match &inner.kind {
            FormKind::Symbol(s) => (s.clone(), meta_form_is_async(meta)),
            _ => return Err(EvalError::Runtime("defn name must be a symbol".into())),
        },
        _ => return Err(EvalError::Runtime("defn requires a symbol name".into())),
    };
    // Skip optional docstring and/or metadata map after the name.
    // Valid orderings: (defn name body...), (defn name "doc" body...),
    // (defn name {:meta ...} body...), (defn name "doc" {:meta ...} body...).
    let mut rest_start = 1;
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Str(_)) {
        rest_start += 1;
    }
    if rest_start < args.len() && matches!(args[rest_start].kind, FormKind::Map(_)) {
        // An attr-map such as `{:async true}` can also request async dispatch.
        is_async |= meta_form_is_async(&args[rest_start]);
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
    let mut fn_val = eval_fn(&fn_args, env)?;
    if is_async && let Value::Fn(ref mut f) = fn_val {
        f.get_mut().is_async = true;
    }
    let var = env
        .globals
        .intern(&env.current_ns, Arc::from(name.as_str()), fn_val.clone());
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
/// `['some.ns :refer :all]`, and versioned forms like `'some.ns@abc1234` or
/// `['some.ns@abc1234 :as alias]`.
fn parse_require_spec_val(val: Value) -> Result<RequireSpec, String> {
    match val {
        Value::Symbol(s) => {
            let sym = s.get();
            Ok(RequireSpec {
                ns: sym.name.clone(),
                version: sym.version.clone(),
                alias: None,
                refer: RequireRefer::None,
            })
        }
        Value::Vector(v) => {
            let items: Vec<Value> = v.get().iter().cloned().collect();
            if items.is_empty() {
                return Err("require spec vector must not be empty".into());
            }
            let (ns, version) = match &items[0] {
                Value::Symbol(s) => {
                    let sym = s.get();
                    (sym.name.clone(), sym.version.clone())
                }
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
            Ok(RequireSpec {
                ns,
                version,
                alias,
                refer,
            })
        }
        other => Err(format!(
            "require expects a symbol or vector, got {}",
            other.type_name()
        )),
    }
}

/// Parse a `RequireSpec` from a raw `Form` (unevaluated, used in `ns` macro).
/// Also handles versioned namespace symbols such as `my.ns@abc1234`.
fn parse_require_spec_form(form: &Form) -> Result<RequireSpec, String> {
    match &form.kind {
        FormKind::Symbol(s) => {
            let sym = cljrs_value::Symbol::parse(s);
            Ok(RequireSpec {
                ns: sym.name.clone(),
                version: sym.version.clone(),
                alias: None,
                refer: RequireRefer::None,
            })
        }
        FormKind::Vector(items) => {
            if items.is_empty() {
                return Err("require spec vector must not be empty".into());
            }
            let (ns, version) = match &items[0].kind {
                FormKind::Symbol(s) => {
                    let sym = cljrs_value::Symbol::parse(s);
                    (sym.name.clone(), sym.version.clone())
                }
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
            Ok(RequireSpec {
                ns,
                version,
                alias,
                refer,
            })
        }
        _ => Err("require spec must be a symbol or vector".into()),
    }
}

// ── ns ────────────────────────────────────────────────────────────────────────

/// Extract the ns name and optional metadata from the `ns` macro's name form.
/// Handles plain symbols and `^meta` shorthand / map forms, e.g.
/// `(ns ^{:doc "..."} my.ns ...)` or `(ns ^:no-doc my.ns ...)`.
fn extract_ns_name_form(form: Option<&Form>, env: &mut Env) -> EvalResult<(String, Option<Value>)> {
    let mut current = form
        .ok_or_else(|| EvalError::Runtime("ns requires a symbol at position 0".into()))?
        .clone();
    let mut meta_acc: Option<Value> = None;
    while let FormKind::Meta(meta_form, inner) = current.kind {
        let meta_val = compile_meta_form(&meta_form, env)?;
        meta_acc = merge_meta(meta_acc, Some(meta_val));
        current = *inner;
    }
    match &current.kind {
        FormKind::Symbol(s) => Ok((s.clone(), meta_acc)),
        _ => Err(EvalError::Runtime("ns requires a symbol at position 0".into())),
    }
}

/// Merge two optional metadata maps, with `overlay` entries taking
/// precedence over `base` entries (matching Clojure's `(merge base overlay)`).
fn merge_meta(base: Option<Value>, overlay: Option<Value>) -> Option<Value> {
    match (base, overlay) {
        (None, None) => None,
        (Some(b), None) => Some(b),
        (None, Some(o)) => Some(o),
        (Some(Value::Map(b)), Some(Value::Map(o))) => {
            let mut merged = b;
            for (k, v) in o.iter() {
                merged = merged.assoc(k.clone(), v.clone());
            }
            Some(Value::Map(merged))
        }
        (_, Some(o)) => Some(o),
    }
}

fn eval_ns(args: &[Form], env: &mut Env) -> EvalResult {
    let (name, name_meta) = extract_ns_name_form(args.first(), env)?;
    env.globals.get_or_create_ns(&name);
    env.current_ns = Arc::from(name.as_str());
    // Auto-refer clojure.core (Clojure default behaviour).
    if name != "clojure.core" {
        env.globals.refer_all(&name, "clojure.core");
    }
    sync_star_ns(env);

    // Optional docstring, then optional attr-map, before reference clauses.
    let mut rest = &args[1..];
    if matches!(rest.first().map(|f| &f.kind), Some(FormKind::Str(_))) {
        rest = &rest[1..];
    }
    let mut attr_meta = None;
    if matches!(rest.first().map(|f| &f.kind), Some(FormKind::Map(_))) {
        attr_meta = Some(eval(&rest[0], env)?);
        rest = &rest[1..];
    }

    if let Some(m) = merge_meta(name_meta, attr_meta) {
        let ns_ptr = env.globals.get_or_create_ns(&env.current_ns);
        ns_ptr.get().set_meta(m);
    }

    for clause in rest {
        if let FormKind::List(items) = &clause.kind {
            match items.first().map(|f| &f.kind) {
                Some(FormKind::Keyword(k)) if k == "require" => {
                    // Expand reader conditionals among require specs
                    let expanded = expand_reader_conds(&items[1..]);
                    for spec_form in &expanded {
                        let spec =
                            parse_require_spec_form(spec_form).map_err(EvalError::Runtime)?;
                        load_ns(env.globals.clone(), &spec, &name)?;
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
                drop(impls);
                cljrs_value::bump_protocol_generation();
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
                drop(impls);
                cljrs_value::bump_protocol_generation();
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
            param_hints: vec![],
            rest_hint: None,
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
            param_hints: vec![],
            rest_hint: None,
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
                drop(impls);
                cljrs_value::bump_protocol_generation();
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

// ── await ─────────────────────────────────────────────────────────────────────

/// Blocking deref in sync context; yielding deref in async context.
///
/// When `cljrs-async` is loaded, `eval_async` intercepts `await` forms before
/// the sync evaluator reaches this handler, so this path is only taken in
/// non-async (sync) code. It blocks the OS thread until the future/promise
/// resolves — equivalent to `(deref val)`.
fn eval_await(args: &[Form], env: &mut Env) -> EvalResult {
    if args.is_empty() {
        return Err(EvalError::Runtime("await requires one argument".into()));
    }
    let val = eval(&args[0], env)?;
    match val {
        Value::Future(f) => {
            let mut guard = f.get().state.lock().unwrap();
            loop {
                match &*guard {
                    FutureState::Done(v) => {
                        f.get().mark_observed();
                        return Ok(v.clone());
                    }
                    FutureState::Failed(v) => {
                        f.get().mark_observed();
                        return Err(EvalError::Thrown(v.clone()));
                    }
                    FutureState::Cancelled => {
                        return Err(EvalError::Runtime("future was cancelled".into()));
                    }
                    FutureState::Running => {
                        guard = f.get().cond.wait(guard).unwrap();
                    }
                }
            }
        }
        Value::Promise(p) => Ok(p.get().deref_blocking()),
        other => Ok(other),
    }
}
