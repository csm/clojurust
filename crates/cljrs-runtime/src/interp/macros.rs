//! Macro expansion pipeline.

use crate::builtins::form::{form_to_value, resolve_auto_forms};
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_types::span::Span;
use cljrs_value::{Symbol, Value};
use std::sync::Arc;

use crate::env::env::Env;
use crate::env::error::{EvalError, EvalResult};

/// Expand a form one step.  Returns the same form if it is not a macro call.
pub fn macroexpand_1(form: &Form, env: &mut Env) -> EvalResult<Form> {
    macroexpand_1_in(form, env, Locals::Frames)
}

fn macroexpand_1_in(form: &Form, env: &mut Env, locals: Locals) -> EvalResult<Form> {
    // Only expand list forms whose head is a macro symbol.
    if let FormKind::List(parts) = &form.kind
        && let Some(FormKind::Symbol(s)) = parts.first().map(|f| &f.kind)
        && let Some(macro_fn) = resolve_macro(s, env, locals)
    {
        // Resolve ::keywords using the caller's namespace before the macro sees
        // them.  In Clojure, ::kw is resolved at READ time; since cljrs keeps
        // AutoKeyword forms in the AST until eval, we resolve them here — the
        // last point at which env.current_ns reflects the call site.
        let resolved = resolve_auto_forms(form, env)?;
        let parts = if let FormKind::List(p) = &resolved.kind {
            p
        } else {
            unreachable!()
        };

        // Build &env value (local bindings as a map — empty at top level).
        let env_val = {
            let (names, vals) = env.all_local_bindings();
            let mut m = cljrs_value::MapValue::empty();
            for (name, val) in names.iter().zip(vals.iter()) {
                m = m.assoc(Value::symbol(Symbol::simple(name.as_ref())), val.clone());
            }
            Value::Map(m)
        };

        // `&form` is the whole call as a list and the macro's arguments are
        // that same list's tail, so convert each part once and share the
        // values between the two rather than converting every argument
        // subtree a second time. A macro body is often the largest thing in
        // the call, and `macroexpand` runs this to a fixed point.
        //
        // A `#?@` splice among the parts is the one case where the two
        // sequences genuinely differ: spliced into the list it contributes
        // its elements to the enclosing sequence, while in an argument
        // position it has no siblings and `form_to_value` rejects it. That
        // case keeps the original two-pass conversion.
        let spliced = parts
            .iter()
            .any(|f| matches!(f.kind, FormKind::ReaderCond { .. }));
        let (form_val, arg_vals) = if spliced {
            let mut arg_vals = Vec::with_capacity(parts.len() - 1);
            for p in &parts[1..] {
                arg_vals.push(form_to_value(p)?);
            }
            (form_to_value(&resolved)?, arg_vals)
        } else {
            let mut vals = Vec::with_capacity(parts.len());
            for p in parts.iter() {
                vals.push(form_to_value(p)?);
            }
            let form_val = crate::builtins::form::values_to_list(vals.iter().cloned());
            let arg_vals = vals.split_off(1);
            (form_val, arg_vals)
        };

        let mut args = Vec::with_capacity(arg_vals.len() + 2);
        args.push(form_val);
        args.push(env_val);
        args.extend(arg_vals);
        let expanded = crate::interp::apply::call_cljrs_fn(&macro_fn, &args, env)?;
        let dummy = Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
        return value_to_form(&expanded, dummy);
    }
    Ok(form.clone())
}

/// Fully expand a form until the head is no longer a macro.
pub fn macroexpand(form: &Form, env: &mut Env) -> EvalResult<Form> {
    macroexpand_in(form, env, Locals::Frames)
}

fn macroexpand_in(form: &Form, env: &mut Env, locals: Locals) -> EvalResult<Form> {
    let mut current = form.clone();
    loop {
        // `macroexpand_1` returns an unchanged clone for non-macro forms.  Do
        // not use structural equality to discover that case: IEEE NaN is not
        // equal to itself, so a form containing `##NaN` would otherwise make
        // this fixed-point loop run forever.
        let is_macro_call = matches!(
            &current.kind,
            FormKind::List(parts)
                if matches!(parts.first().map(|f| &f.kind), Some(FormKind::Symbol(s)) if resolve_macro(s, env, locals).is_some())
        );
        if !is_macro_call {
            return Ok(current);
        }

        let expanded = macroexpand_1_in(&current, env, locals)?;
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
///
/// The form is taken to stand at the top level: the only locals that shadow
/// a macro in it are the ones its own binding forms introduce. `env`'s frames
/// are not consulted, since they are not the form's lexical scope.
pub fn macroexpand_all(form: &Form, env: &mut Env) -> EvalResult<Form> {
    macroexpand_all_in(form, env, &[])
}

/// [`macroexpand_all`] for a form that stands inside the bindings `locals`,
/// such as the body of a function with those parameters.
pub fn macroexpand_all_in(form: &Form, env: &mut Env, locals: &[Arc<str>]) -> EvalResult<Form> {
    expand_all(form, env, &mut locals.to_vec())
}

/// Expand `form` and its sub-forms; `scope` holds the locals in scope at
/// `form` and is left as it was found.
fn expand_all(form: &Form, env: &mut Env, scope: &mut Vec<Arc<str>>) -> EvalResult<Form> {
    // First, expand the top level.
    let expanded = macroexpand_in(form, env, Locals::Lexical(scope))?;

    let span = expanded.span.clone();
    let kind = match &expanded.kind {
        FormKind::List(parts) if !parts.is_empty() => {
            // Check if the head is a special form that shouldn't be walked.
            let head_name = match &parts[0].kind {
                FormKind::Symbol(s) => Some(s.as_str()),
                _ => None,
            };
            // Each binding form below puts its names in scope for the forms
            // they are visible to, and takes them out again when it ends.
            let outer = scope.len();
            let mut new_parts = vec![parts[0].clone()];
            match head_name {
                // quote: don't expand inside quoted forms
                Some("quote") => return Ok(expanded),
                // (fn name? [params] body...) or (fn name? ([params] body...) ...),
                // and the `def` forms that take the same tail.
                Some("fn*" | "fn" | "defn" | "defn-" | "defmacro") => {
                    expand_fn_tail(&parts[1..], env, scope, &mut new_parts)?;
                }
                // Each binding's value sees the names bound before it; the
                // body sees them all.
                Some("let*" | "let" | "loop*" | "loop") => {
                    if let Some(bindings) = parts.get(1) {
                        if let FormKind::Vector(pairs) = &bindings.kind {
                            let mut new_pairs = Vec::with_capacity(pairs.len());
                            for pair in pairs.chunks(2) {
                                let value = match pair.get(1) {
                                    Some(v) => Some(expand_all(v, env, scope)?),
                                    None => None,
                                };
                                new_pairs.push(expand_all(&pair[0], env, scope)?);
                                new_pairs.extend(value);
                                binding_names(&pair[0], scope);
                            }
                            new_parts.push(Form::new(
                                FormKind::Vector(new_pairs),
                                bindings.span.clone(),
                            ));
                        } else {
                            new_parts.push(bindings.clone());
                        }
                        for p in &parts[2..] {
                            new_parts.push(expand_all(p, env, scope)?);
                        }
                    }
                }
                // (letfn [(name [params] body...) ...] body...): every name is
                // in scope in every function, and in the body.
                Some("letfn") => {
                    if let Some(specs) = parts.get(1) {
                        if let FormKind::Vector(fns) = &specs.kind {
                            scope.extend(fns.iter().filter_map(|f| match &f.kind {
                                FormKind::List(spec) => {
                                    spec.first().and_then(Form::as_symbol).map(Arc::from)
                                }
                                _ => None,
                            }));
                            let mut new_fns = Vec::with_capacity(fns.len());
                            for f in fns {
                                match &f.kind {
                                    FormKind::List(spec) if !spec.is_empty() => {
                                        let mut new_spec = vec![spec[0].clone()];
                                        expand_fn_tail(&spec[1..], env, scope, &mut new_spec)?;
                                        new_fns.push(Form::new(
                                            FormKind::List(new_spec),
                                            f.span.clone(),
                                        ));
                                    }
                                    _ => new_fns.push(f.clone()),
                                }
                            }
                            new_parts
                                .push(Form::new(FormKind::Vector(new_fns), specs.span.clone()));
                        } else {
                            new_parts.push(specs.clone());
                        }
                        for p in &parts[2..] {
                            new_parts.push(expand_all(p, env, scope)?);
                        }
                    }
                }
                // (catch Type name body...)
                Some("catch") if parts.len() > 2 => {
                    new_parts.extend([parts[1].clone(), parts[2].clone()]);
                    binding_names(&parts[2], scope);
                    for p in &parts[3..] {
                        new_parts.push(expand_all(p, env, scope)?);
                    }
                }
                _ => {
                    // Generic: expand all sub-forms
                    new_parts.clear();
                    for p in parts {
                        new_parts.push(expand_all(p, env, scope)?);
                    }
                }
            }
            scope.truncate(outer);
            FormKind::List(new_parts)
        }
        FormKind::Vector(items) => {
            let new_items = items
                .iter()
                .map(|i| expand_all(i, env, scope))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Vector(new_items)
        }
        FormKind::Map(items) => {
            let new_items = items
                .iter()
                .map(|i| expand_all(i, env, scope))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Map(new_items)
        }
        FormKind::Set(items) => {
            let new_items = items
                .iter()
                .map(|i| expand_all(i, env, scope))
                .collect::<EvalResult<Vec<_>>>()?;
            FormKind::Set(new_items)
        }
        // Atoms, keywords, strings, etc. — no sub-forms.
        _ => return Ok(expanded),
    };
    Ok(Form::new(kind, span))
}

/// Expand what follows the head of a `fn`: an optional name, then one arity
/// written `[params] body...` or several written `([params] body...)`. A
/// docstring or attribute map among them (`defn`) is expanded as data.
fn expand_fn_tail(
    tail: &[Form],
    env: &mut Env,
    scope: &mut Vec<Arc<str>>,
    out: &mut Vec<Form>,
) -> EvalResult<()> {
    for (i, part) in tail.iter().enumerate() {
        match &part.unmeta().kind {
            // The function's own name, which its bodies can call.
            FormKind::Symbol(name) => {
                scope.push(Arc::from(name.as_str()));
                out.push(part.clone());
            }
            FormKind::Vector(_) => return expand_arity(&tail[i..], env, scope, out),
            FormKind::List(arity)
                if matches!(
                    arity.first().map(|f| &f.unmeta().kind),
                    Some(FormKind::Vector(_))
                ) =>
            {
                let mut new_arity = Vec::with_capacity(arity.len());
                expand_arity(arity, env, scope, &mut new_arity)?;
                out.push(Form::new(FormKind::List(new_arity), part.span.clone()));
            }
            _ => out.push(expand_all(part, env, scope)?),
        }
    }
    Ok(())
}

/// Expand one arity, `arity[0]` being its parameter vector: the parameters
/// are in scope in the body and nowhere else.
fn expand_arity(
    arity: &[Form],
    env: &mut Env,
    scope: &mut Vec<Arc<str>>,
    out: &mut Vec<Form>,
) -> EvalResult<()> {
    let outer = scope.len();
    out.push(expand_all(&arity[0], env, scope)?);
    binding_names(&arity[0], scope);
    for p in &arity[1..] {
        out.push(expand_all(p, env, scope)?);
    }
    scope.truncate(outer);
    Ok(())
}

/// Append the names a binding form introduces: a symbol, or every symbol a
/// destructuring pattern binds (`[a & more :as all]`, `{a :a :keys [b]
/// :as m}`). The default expressions under `:or` bind nothing.
pub fn binding_names(pattern: &Form, out: &mut Vec<Arc<str>>) {
    // `:keys [ns/a]` and `:keys [:ns/a]` both bind `a`.
    let local = |s: &str| Arc::from(s.rsplit('/').next().unwrap_or(s));
    match &pattern.unmeta().kind {
        FormKind::Symbol(s) if s != "&" => out.push(local(s)),
        FormKind::Vector(items) => items.iter().for_each(|i| binding_names(i, out)),
        FormKind::Map(items) => {
            for pair in items.chunks(2) {
                let [key, value] = pair else { continue };
                match key.as_keyword().map(|k| k.rsplit('/').next().unwrap_or(k)) {
                    Some("keys" | "strs" | "syms") => {
                        if let FormKind::Vector(names) = &value.unmeta().kind {
                            for name in names {
                                if let FormKind::Symbol(s) | FormKind::Keyword(s) =
                                    &name.unmeta().kind
                                {
                                    out.push(local(s));
                                }
                            }
                        }
                    }
                    Some("as") => binding_names(value, out),
                    Some(_) => {}
                    None => binding_names(key, out),
                }
            }
        }
        _ => {}
    }
}

/// The local bindings a call's head symbol is checked against. A local
/// shadows a macro: `(let [doc (fn [x] x)] (doc 1))` calls the local, it does
/// not expand `clojure.core/doc`.
#[derive(Clone, Copy)]
enum Locals<'a> {
    /// The frames of the environment evaluating the form.
    Frames,
    /// The names bound lexically around a form that is expanded ahead of its
    /// evaluation. The environment's frames say nothing about such a form:
    /// they belong to whoever asked for the expansion.
    Lexical(&'a [Arc<str>]),
}

impl Locals<'_> {
    fn binds(self, name: &str, env: &Env) -> bool {
        match self {
            Locals::Frames => env.lookup_local_frames(name).is_some(),
            Locals::Lexical(names) => names.iter().any(|n| n.as_ref() == name),
        }
    }
}

/// If `sym` resolves to a macro in the current env, return its CljxFn.
fn resolve_macro(sym: &str, env: &Env, locals: Locals) -> Option<cljrs_value::CljxFn> {
    let parsed = Symbol::parse(sym);
    let name = parsed.name.as_ref();
    // Only unqualified symbols can be locals.
    if parsed.namespace.is_none() && locals.binds(name, env) {
        return None;
    }
    let ns: Arc<str> = env.resolve_ns_or_current(parsed.namespace.as_deref());

    let v = env.globals.lookup_in_ns(&ns, name)?;
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

        // WithMeta: keep the annotation as a `^meta` wrapper so metadata a
        // macro received (or attached) survives back into the AST.
        //
        // The annotation is *quoted*: it is already a value, and re-analysing
        // it would resolve its contents as code — `{:tag 'String}` would look
        // up `String`, and `{:arglists '([x])}` would look up `x`.
        Value::WithMeta(inner, meta) => {
            if matches!(**meta, Value::Nil) {
                return value_to_form(inner, span);
            }
            let inner_form = value_to_form(inner, span.clone())?;
            let meta_form = value_to_form(meta, span.clone())?;
            let quoted_meta = Form::new(FormKind::Quote(Box::new(meta_form)), span.clone());
            FormKind::Meta(Box::new(quoted_meta), Box::new(inner_form))
        }

        Value::Pattern(p) => FormKind::Regex(p.get().as_str().to_string()),

        // Non-data types: wrap in a symbol placeholder (best effort).
        other => FormKind::Symbol(format!("#<{}>", other.type_name())),
    };
    Ok(Form::new(kind, span))
}
