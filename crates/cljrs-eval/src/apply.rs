//! Function application and the recur trampoline.

use std::collections::HashMap;
use std::sync::Arc;

use cljrs_gc::{GcPtr, check_cancellation};
use cljrs_reader::Form;
use cljrs_value::{
    AgentFn, AgentMsg, Arity, Atom, CljxFn, CljxFnArity, Delay, LazySeq, MapValue, PersistentList,
    Symbol, Thunk, Value,
};

use crate::destructure::value_to_seq_vec;
use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::eval::eval;

/// Convert an EvalError to a Value for storage (e.g. agent errors).
/// Preserves Thrown values (ex-info); other errors become strings.
fn eval_error_to_value(e: EvalError) -> Value {
    match e {
        EvalError::Thrown(v) => v,
        other => Value::string(format!("{other}")),
    }
}

// ── Watch notification ───────────────────────────────────────────────────────

/// Fire all watches on a watchable (atom, var, agent).
/// Each watch fn is called as `(f key ref old new)`.
/// Errors thrown by watch fns are re-thrown (matching Clojure behavior).
fn fire_watches(
    watches: &std::sync::Mutex<Vec<(Value, Value)>>,
    reference: &Value,
    old: &Value,
    new: &Value,
    env: &mut Env,
) {
    let ws: Vec<(Value, Value)> = watches.lock().unwrap().clone();
    for (key, f) in &ws {
        let args = vec![key.clone(), reference.clone(), old.clone(), new.clone()];
        if let Err(e) = apply_value(f, args, env) {
            // Re-throw watch errors (Clojure behavior: exception propagates to caller)
            // We can't return EvalResult from here, so we store and re-throw below.
            // For now, propagate by re-invoking so it surfaces.
            // Actually, in Clojure, watch exceptions propagate to the mutating call.
            // We need to handle this differently — but for simplicity, just ignore for now
            // and let the caller check. Actually let's just use a thread-local to propagate.
            WATCH_ERROR.with(|cell| {
                cell.borrow_mut().replace(e);
            });
            return;
        }
    }
}

thread_local! {
    static WATCH_ERROR: std::cell::RefCell<Option<EvalError>> = const { std::cell::RefCell::new(None) };
}

/// Check if a watch error occurred and propagate it.
fn check_watch_error() -> EvalResult<()> {
    WATCH_ERROR.with(|cell| {
        if let Some(e) = cell.borrow_mut().take() {
            Err(e)
        } else {
            Ok(())
        }
    })
}

// ── Lazy seq error propagation ───────────────────────────────────────────────

thread_local! {
    /// Stash for errors from lazy seq thunk evaluation.
    /// `Thunk::force` can only return `Value`, so errors are stored here
    /// and checked by callers of `realize()`.
    static LAZY_SEQ_ERROR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Check and take the last lazy seq error, if any.
pub fn take_lazy_seq_error() -> Option<String> {
    LAZY_SEQ_ERROR.with(|e| e.borrow_mut().take())
}

// ── ClosureThunk ──────────────────────────────────────────────────────────────

/// A Thunk that calls a zero-arg Clojure closure when forced.
#[derive(Debug)]
struct ClosureThunk {
    f: CljxFn,
    globals: std::sync::Arc<crate::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
}

impl cljrs_gc::Trace for ClosureThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.f.trace(visitor);
    }
}

impl Thunk for ClosureThunk {
    fn force(&self) -> Value {
        let mut env = Env::with_closure(self.globals.clone(), &self.ns, &self.f);
        match call_cljrs_fn(&self.f, vec![], &mut env) {
            Ok(v) => v,
            Err(e) => {
                LAZY_SEQ_ERROR.with(|slot| {
                    *slot.borrow_mut() = Some(format!("{e}"));
                });
                Value::Nil
            }
        }
    }
}

/// Evaluate a call expression `(func-form arg1 arg2 ...)`.
///
/// Handles:
/// - Macro expansion (if callee is a macro).
/// - The `apply` function (spread last arg).
/// - The `swap!` function (needs env to call the function).
/// - Regular function calls.
pub fn eval_call(func_form: &Form, arg_forms: &[Form], env: &mut Env) -> EvalResult {
    // Evaluate the callee first.
    let callee = eval(func_form, env)?;

    // Macro check: expand then re-eval.
    if let Value::Macro(mfn) = &callee {
        let expanded = macro_apply(mfn.get(), func_form, arg_forms, env)?;
        return eval(&expanded, env);
    }

    // Special case: `apply` native fn — spread last arg.
    if let Value::NativeFunction(nf) = &callee {
        match nf.get().name.as_ref() {
            "apply" => return handle_apply_call(arg_forms, env),
            "atom" => return handle_atom_call(arg_forms, env),
            "reset!" => return handle_reset_bang(arg_forms, env),
            "swap!" => return handle_swap_call(arg_forms, env),
            "make-lazy-seq" => return handle_make_lazy_seq(arg_forms, env),
            "make-delay" => return handle_make_delay(arg_forms, env),
            "vswap!" => return handle_vswap(arg_forms, env),
            "send" | "send-off" => return handle_send(arg_forms, env),
            "with-bindings*" => return handle_with_bindings(arg_forms, env),
            "alter-var-root" => return handle_alter_var_root(arg_forms, env),
            "vary-meta" => return handle_vary_meta(arg_forms, env),
            "find-ns" | "the-ns" => return handle_find_ns(arg_forms, env),
            "all-ns" => return handle_all_ns(arg_forms, env),
            "create-ns" => return handle_create_ns(arg_forms, env),
            "ns-aliases" => return handle_ns_aliases(arg_forms, env),
            "remove-ns" => return handle_remove_ns(arg_forms, env),
            "alter-meta!" => return handle_alter_meta(arg_forms, env),
            "ns-resolve" => return handle_ns_resolve(arg_forms, env),
            "resolve" => return handle_resolve(arg_forms, env),
            "intern" => return handle_intern(arg_forms, env),
            "bound-fn*" => return handle_bound_fn_star(arg_forms, env),
            _ => {}
        }
    }

    // Evaluate arguments.
    let args: Vec<Value> = arg_forms
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    apply_value(&callee, args, env)
}

/// Return the canonical type tag for a value (used by protocol dispatch).
pub fn type_tag_of(val: &Value) -> Arc<str> {
    match val {
        Value::Nil => Arc::from("nil"),
        Value::Bool(_) => Arc::from("Boolean"),
        Value::Long(_) => Arc::from("Long"),
        Value::Double(_) => Arc::from("Double"),
        Value::BigInt(_) => Arc::from("BigInt"),
        Value::BigDecimal(_) => Arc::from("BigDecimal"),
        Value::Ratio(_) => Arc::from("Ratio"),
        Value::Char(_) => Arc::from("Character"),
        Value::Str(_) => Arc::from("String"),
        Value::Keyword(_) => Arc::from("Keyword"),
        Value::Symbol(_) => Arc::from("Symbol"),
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_) => Arc::from("List"),
        Value::Vector(_) => Arc::from("Vector"),
        Value::Map(_) => Arc::from("Map"),
        Value::Set(_) => Arc::from("Set"),
        Value::Fn(_) | Value::NativeFunction(_) | Value::ProtocolFn(_) | Value::MultiFn(_) => {
            Arc::from("Fn")
        }
        Value::Atom(_) => Arc::from("Atom"),
        Value::Var(_) => Arc::from("Var"),
        Value::Protocol(_) => Arc::from("Protocol"),
        Value::Volatile(_) => Arc::from("Volatile"),
        Value::Delay(_) => Arc::from("Delay"),
        Value::Promise(_) => Arc::from("Promise"),
        Value::Future(_) => Arc::from("Future"),
        Value::Agent(_) => Arc::from("Agent"),
        Value::TypeInstance(ti) => ti.get().type_tag.clone(),
        Value::NativeObject(obj) => Arc::from(obj.get().type_tag()),
        Value::Resource(_) => Arc::from("Resource"),
        _ => Arc::from("Object"),
    }
}

/// Resolve a type symbol from `extend-type` to a canonical tag.
/// Canonical tags ARE the short names, so this just passes through.
pub fn resolve_type_tag(sym: &str) -> Arc<str> {
    Arc::from(sym)
}

/// Apply `callee` to the already-evaluated `args`.
pub fn apply_value(callee: &Value, args: Vec<Value>, env: &mut Env) -> EvalResult {
    // Check for GC cancellation at function application boundary
    check_cancellation()
        .map_err(|_| EvalError::Runtime("GC in progress, operation cancelled".to_string()))?;

    match callee {
        Value::NativeFunction(nf) => {
            check_arity(&nf.get().arity, args.len(), &nf.get().name)?;
            crate::callback::push_eval_context(env);
            let result = (nf.get().func)(&args).map_err(|e| EvalError::Runtime(e.to_string()));
            crate::callback::pop_eval_context();
            result
        }
        Value::Fn(f) => call_cljrs_fn(f.get(), args, env),
        Value::BoundFn(bf) => {
            let bf_ref = bf.get();
            // Push captured bindings as a frame on top of the current stack.
            // This means captured bindings take priority over the caller's,
            // but vars not in the capture fall through to the caller's frames.
            let _guard = crate::dynamics::push_frame(bf_ref.captured_bindings.clone());
            apply_value(&bf_ref.wrapped, args, env)
        }
        Value::ProtocolFn(pf) => {
            let pf_ref = pf.get();
            let dispatch_val = args.first().ok_or_else(|| {
                EvalError::Runtime(format!(
                    "{}: requires at least 1 argument",
                    pf_ref.method_name
                ))
            })?;
            let tag = type_tag_of(dispatch_val);
            let impls = pf_ref.protocol.get().impls.lock().unwrap();
            let impl_fn = impls
                .get(tag.as_ref())
                .and_then(|m| m.get(pf_ref.method_name.as_ref()))
                .cloned()
                .ok_or_else(|| {
                    EvalError::Runtime(format!(
                        "No implementation of protocol {} for type {}",
                        pf_ref.protocol.get().name,
                        tag
                    ))
                })?;
            drop(impls);
            apply_value(&impl_fn, args, env)
        }
        Value::MultiFn(mf) => {
            let mf_ref = mf.get();
            let dispatch_val = apply_value(&mf_ref.dispatch_fn, args.clone(), env)?;
            // Check for GC cancellation after dispatch
            check_cancellation().map_err(|_| {
                EvalError::Runtime("GC in progress, operation cancelled".to_string())
            })?;
            let key = format!("{}", dispatch_val);
            let methods = mf_ref.methods.lock().unwrap();
            let impl_fn = methods
                .get(&key)
                .or_else(|| methods.get(&mf_ref.default_dispatch))
                .cloned()
                .ok_or_else(|| {
                    EvalError::Runtime(format!(
                        "No method in multimethod '{}' for dispatch value {}",
                        mf_ref.name, key
                    ))
                })?;
            drop(methods);
            apply_value(&impl_fn, args, env)
        }
        Value::Keyword(_kw) => {
            // (kw map-or-record) → map.get(kw)
            let default = || args.get(1).cloned().unwrap_or(Value::Nil);
            let target = args.first().map(|a| a.unwrap_meta());
            match target {
                Some(Value::Map(m)) => Ok(m.get(callee).unwrap_or_else(default)),
                Some(Value::TypeInstance(ti)) => {
                    Ok(ti.get().fields.get(callee).unwrap_or_else(default))
                }
                Some(Value::Nil) => Ok(default()),
                _ => Ok(Value::Nil),
            }
        }
        Value::Map(m) => {
            // (map key) → map.get(key)
            match args.first() {
                Some(k) => Ok(m
                    .get(k)
                    .unwrap_or(args.get(1).cloned().unwrap_or(Value::Nil))),
                None => Ok(Value::Nil),
            }
        }
        Value::Set(s) => match args.first() {
            Some(k) => {
                if s.contains(k) {
                    Ok(k.clone())
                } else {
                    Ok(Value::Nil)
                }
            }
            None => Ok(Value::Nil),
        },
        other => Err(EvalError::NotCallable(format!(
            "<{}> is not callable",
            other.type_name()
        ))),
    }
}

/// Call a `CljxFn` with pre-evaluated args, with recur trampoline.
pub fn call_cljrs_fn(f: &CljxFn, args: Vec<Value>, caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    // Create a fresh env with closure bindings, executing in the defining namespace.
    // This ensures macros qualify symbols relative to their definition site.
    let mut env = Env::with_closure(caller_env.globals.clone(), &f.defining_ns, f);

    let mut current_args = args;
    loop {
        // Check for GC cancellation before entering function body
        // Check for GC cancellation before entering function body
        check_cancellation()
            .map_err(|_| EvalError::Runtime("GC in progress, operation cancelled".to_string()))?;

        env.push_frame();

        // Bind params.
        bind_fn_params(arity, &current_args, &mut env)?;

        // Self-reference for named functions.
        if let Some(ref name) = f.name {
            let self_val = Value::Fn(GcPtr::new(f.clone()));
            env.bind(name.clone(), self_val);
        }

        // Eval body, catching Recur.
        let result = eval_body_recur_fn(&arity.body, &mut env);
        env.pop_frame();

        // Check for GC cancellation after function body (before recur)
        check_cancellation()
            .map_err(|_| EvalError::Runtime("GC in progress, operation cancelled".to_string()))?;

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_args)) => {
                current_args = new_args;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Bind function parameters in the current (top) frame.
fn bind_fn_params(arity: &CljxFnArity, args: &[Value], env: &mut Env) -> EvalResult<()> {
    let n = arity.params.len();
    // Bind positional params.
    for (i, name) in arity.params.iter().enumerate() {
        let val = args.get(i).cloned().unwrap_or(Value::Nil);
        env.bind(name.clone(), val);
    }
    // Bind rest param.
    if let Some(ref rest) = arity.rest_param {
        let rest_items = args[n..].to_vec();
        let rest_val = if rest_items.is_empty() {
            Value::Nil
        } else {
            Value::List(GcPtr::new(PersistentList::from_iter(rest_items)))
        };
        env.bind(rest.clone(), rest_val.clone());
        // Apply rest destructuring if present.
        if let Some(ref pattern) = arity.destructure_rest {
            crate::destructure::bind_pattern(pattern, rest_val, env)?;
        }
    }
    // Apply positional destructuring patterns.
    for (idx, pattern) in &arity.destructure_params {
        let val = args.get(*idx).cloned().unwrap_or(Value::Nil);
        crate::destructure::bind_pattern(pattern, val, env)?;
    }
    Ok(())
}

/// Eval a function body, propagating Recur up (does not catch it).
fn eval_body_recur_fn(body: &[cljrs_reader::Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in body {
        result = eval(form, env)?;
    }
    Ok(result)
}

/// Select the matching arity for the given argument count.
pub fn select_arity(f: &CljxFn, argc: usize) -> EvalResult<&CljxFnArity> {
    let name = f.name.as_deref().unwrap_or("fn");
    // Try fixed arities first.
    for arity in &f.arities {
        if arity.rest_param.is_none() && arity.params.len() == argc {
            return Ok(arity);
        }
    }
    // Try variadic arities.
    for arity in &f.arities {
        if arity.rest_param.is_some() && argc >= arity.params.len() {
            return Ok(arity);
        }
    }
    // Build expected string.
    let expected: Vec<String> = f
        .arities
        .iter()
        .map(|a| {
            if a.rest_param.is_some() {
                format!("{}+", a.params.len())
            } else {
                a.params.len().to_string()
            }
        })
        .collect();
    Err(EvalError::Arity {
        name: name.to_string(),
        expected: expected.join(" or "),
        got: argc,
    })
}

fn check_arity(arity: &Arity, argc: usize, name: &str) -> EvalResult<()> {
    match arity {
        Arity::Fixed(n) if argc != *n => Err(EvalError::Arity {
            name: name.to_string(),
            expected: n.to_string(),
            got: argc,
        }),
        Arity::Variadic { min } if argc < *min => Err(EvalError::Arity {
            name: name.to_string(),
            expected: format!("{}+", min),
            got: argc,
        }),
        _ => Ok(()),
    }
}

/// Expand a macro: convert unevaluated arg forms to values, call the macro fn,
/// then convert the resulting Value back to a Form.
///
/// Clojure macros receive two implicit leading arguments:
/// - `&form`: the entire call expression as a quoted value
/// - `&env`: a map of local bindings at the call site (symbol → value)
fn macro_apply(
    mfn: &CljxFn,
    func_form: &Form,
    arg_forms: &[Form],
    env: &mut Env,
) -> EvalResult<Form> {
    // &form: the whole call expression as a list value.
    let form_val = {
        let mut items = vec![crate::eval::form_to_value(func_form)];
        for f in arg_forms {
            items.push(crate::eval::form_to_value(f));
        }
        Value::List(GcPtr::new(PersistentList::from_iter(items)))
    };

    // &env: local variable bindings at call site as a map (symbol → value).
    let env_val = {
        let (names, vals) = env.all_local_bindings();
        let mut m = MapValue::empty();
        for (name, val) in names.iter().zip(vals.iter()) {
            m = m.assoc(Value::symbol(Symbol::simple(name.as_ref())), val.clone());
        }
        Value::Map(m)
    };

    // Prepend &form and &env, then pass remaining arg forms as unevaluated values.
    let mut args = vec![form_val, env_val];
    args.extend(arg_forms.iter().map(crate::eval::form_to_value));

    let expanded_val = call_cljrs_fn(mfn, args, env)?;
    let dummy_span = cljrs_types::span::Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
    crate::macros::value_to_form(&expanded_val, dummy_span)
}

/// Handle `(apply f arg1 ... last-coll)` — spread the last arg.
fn handle_apply_call(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    let mut evaled: Vec<Value> = arg_forms
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    if evaled.len() < 2 {
        return Err(EvalError::Arity {
            name: "apply".into(),
            expected: "2+".into(),
            got: evaled.len(),
        });
    }

    let f = evaled.remove(0);
    let last = evaled.pop().unwrap();
    // Spread last arg.
    let spread = value_to_seq_vec(&last);
    evaled.extend(spread);
    apply_value(&f, evaled, env)
}

/// Handle `(make-lazy-seq f)` — wraps a zero-arg fn in a lazy sequence.
pub fn handle_make_lazy_seq(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() != 1 {
        return Err(EvalError::Arity {
            name: "make-lazy-seq".into(),
            expected: "1".into(),
            got: arg_forms.len(),
        });
    }
    let f_val = eval(&arg_forms[0], env)?;
    let f = match f_val {
        Value::Fn(f) => f.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "make-lazy-seq requires a fn, got {}",
                other.type_name()
            )));
        }
    };
    let thunk = ClosureThunk {
        f,
        globals: env.globals.clone(),
        ns: env.current_ns.clone(),
    };
    Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))))
}

/// Handle `(make-delay f)` — wraps a zero-arg fn in a Delay.
fn handle_make_delay(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() != 1 {
        return Err(EvalError::Arity {
            name: "make-delay".into(),
            expected: "1".into(),
            got: arg_forms.len(),
        });
    }
    let f_val = eval(&arg_forms[0], env)?;
    let f = match f_val {
        Value::Fn(f) => f.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "make-delay requires a fn, got {}",
                other.type_name()
            )));
        }
    };
    let thunk = ClosureThunk {
        f,
        globals: env.globals.clone(),
        ns: env.current_ns.clone(),
    };
    Ok(Value::Delay(GcPtr::new(Delay::new(Box::new(thunk)))))
}

/// Handle `(vswap! vol f & args)` — apply f to current volatile value and store.
fn handle_vswap(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "vswap!".into(),
            expected: "2+".into(),
            got: arg_forms.len(),
        });
    }
    let vol_val = eval(&arg_forms[0], env)?;
    let f = eval(&arg_forms[1], env)?;
    let extra: Vec<Value> = arg_forms[2..]
        .iter()
        .map(|a| eval(a, env))
        .collect::<EvalResult<_>>()?;

    match vol_val {
        Value::Volatile(v) => {
            let cur = v.get().deref();
            let mut call_args = vec![cur];
            call_args.extend(extra);
            let new_val = apply_value(&f, call_args, env)?;
            v.get().reset(new_val.clone());
            Ok(new_val)
        }
        other => Err(EvalError::Runtime(format!(
            "vswap!: expected volatile, got {}",
            other.type_name()
        ))),
    }
}

/// Handle `(send agent f & extra)` / `(send-off agent f & extra)`.
fn handle_send(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "send".into(),
            expected: "2+".into(),
            got: arg_forms.len(),
        });
    }
    let agent_val = eval(&arg_forms[0], env)?;
    let f = eval(&arg_forms[1], env)?;
    let extra: Vec<Value> = arg_forms[2..]
        .iter()
        .map(|a| eval(a, env))
        .collect::<EvalResult<_>>()?;
    let globals = env.globals.clone();
    let ns = env.current_ns.clone();

    match &agent_val {
        Value::Agent(a) => {
            let agent_clone = a.clone();
            let agent_val_clone = agent_val.clone();
            let agent_fn: AgentFn = Box::new(move |state| {
                let mut call_args = vec![state.clone()];
                call_args.extend(extra);
                let mut call_env = Env::new(globals, &ns);
                let new_val =
                    apply_value(&f, call_args, &mut call_env).map_err(eval_error_to_value)?;
                // Fire watches (agent watches fire on the agent thread)
                fire_watches(
                    &agent_clone.get().watches,
                    &agent_val_clone,
                    &state,
                    &new_val,
                    &mut call_env,
                );
                // Watch errors become agent errors
                if let Err(e) = check_watch_error() {
                    return Err(eval_error_to_value(e));
                }
                Ok(new_val)
            });
            a.get()
                .sender
                .lock()
                .unwrap()
                .send(AgentMsg::Update(agent_fn))
                .map_err(|_| EvalError::Runtime("send: agent is shut down".into()))?;
            Ok(agent_val.clone())
        }
        other => Err(EvalError::Runtime(format!(
            "send: expected agent, got {}",
            other.type_name()
        ))),
    }
}

// ── atom ──────────────────────────────────────────────────────────────────────

/// Handle `(swap! atom f & args)`.
fn handle_atom_call(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "atom".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    let initial = eval(&arg_forms[0], env)?;

    // Evaluate and parse keyword options; unknown keys / nil keys are ignored.
    let options: Vec<Value> = arg_forms[1..]
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    let mut meta_opt: Option<Value> = None;
    let mut validator_opt: Option<Value> = None;
    let mut i = 0;
    while i + 1 < options.len() {
        match &options[i] {
            Value::Keyword(k) if k.get().name.as_ref() == "meta" => {
                meta_opt = Some(options[i + 1].clone());
                i += 2;
            }
            Value::Keyword(k) if k.get().name.as_ref() == "validator" => {
                let vf = options[i + 1].clone();
                validator_opt = if vf == Value::Nil { None } else { Some(vf) };
                i += 2;
            }
            _ => {
                i += 2;
            }
        }
    }

    // Validate :meta must be nil or a map.
    if let Some(ref m) = meta_opt
        && !matches!(m, Value::Nil | Value::Map(_))
    {
        return Err(EvalError::Thrown(Value::string(
            "Atom metadata must be a map or nil".to_string(),
        )));
    }

    // Check validator on the initial value.
    if let Some(ref vf) = validator_opt {
        let result = apply_value(vf, vec![initial.clone()], env)?;
        if result == Value::Nil || result == Value::Bool(false) {
            return Err(EvalError::Thrown(Value::string(
                "Invalid initial value for atom".to_string(),
            )));
        }
    }

    let atom = GcPtr::new(Atom::new(initial));
    if let Some(m) = meta_opt {
        atom.get()
            .set_meta(if m == Value::Nil { None } else { Some(m) });
    }
    if let Some(vf) = validator_opt {
        atom.get().set_validator(Some(vf));
    }
    Ok(Value::Atom(atom))
}

// ── reset! ────────────────────────────────────────────────────────────────────

fn handle_reset_bang(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "reset!".into(),
            expected: "2".into(),
            got: arg_forms.len(),
        });
    }
    let atom_val = eval(&arg_forms[0], env)?;
    let new_val = eval(&arg_forms[1], env)?;

    let atom = match &atom_val {
        Value::Atom(a) => a.clone(),
        v => {
            return Err(EvalError::Runtime(format!(
                "reset! requires an atom, got {}",
                v.type_name()
            )));
        }
    };

    validate_atom_value(&atom, &new_val, env)?;
    let old_val = atom.get().deref();
    atom.get().reset(new_val.clone());
    fire_watches(&atom.get().watches, &atom_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

/// Call the atom's validator (if any) on `new_val`. Throws if invalid.
fn validate_atom_value(atom: &GcPtr<Atom>, new_val: &Value, env: &mut Env) -> EvalResult<()> {
    if let Some(vf) = atom.get().get_validator() {
        let result = apply_value(&vf, vec![new_val.clone()], env)?;
        if result == Value::Nil || result == Value::Bool(false) {
            return Err(EvalError::Thrown(Value::string(
                "Invalid value for atom".to_string(),
            )));
        }
    }
    Ok(())
}

// ── swap! ─────────────────────────────────────────────────────────────────────

fn handle_swap_call(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    let mut evaled: Vec<Value> = arg_forms
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    if evaled.len() < 2 {
        return Err(EvalError::Arity {
            name: "swap!".into(),
            expected: "2+".into(),
            got: evaled.len(),
        });
    }

    let atom_val = evaled.remove(0);
    let f = evaled.remove(0);

    let atom = match &atom_val {
        Value::Atom(a) => a.clone(),
        v => {
            return Err(EvalError::Runtime(format!(
                "swap! requires an atom, got {}",
                v.type_name()
            )));
        }
    };

    let old_val = atom.get().deref();
    let mut args = vec![old_val.clone()];
    args.extend(evaled);
    let new_val = apply_value(&f, args, env)?;
    validate_atom_value(&atom, &new_val, env)?;
    atom.get().reset(new_val.clone());
    fire_watches(&atom.get().watches, &atom_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

// ── with-bindings* ────────────────────────────────────────────────────────────

/// `(with-bindings* {#'var val ...} fn)` — push a binding frame, call fn with
/// no args, pop the frame, return the result.
fn handle_with_bindings(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "with-bindings*".into(),
            expected: "2".into(),
            got: arg_forms.len(),
        });
    }
    let map_val = eval(&arg_forms[0], env)?;
    let func_val = eval(&arg_forms[1], env)?;

    let mut frame: HashMap<usize, Value> = HashMap::new();
    if let Value::Map(m) = &map_val {
        m.for_each(|k, v| {
            if let Value::Var(vp) = k {
                frame.insert(crate::dynamics::var_key_of(vp), v.clone());
            }
            // non-Var keys silently ignored
        });
    } else {
        return Err(EvalError::Runtime(
            "with-bindings*: first arg must be a map".into(),
        ));
    }

    let _guard = crate::dynamics::push_frame(frame);
    apply_value(&func_val, vec![], env)
}

// ── alter-var-root ────────────────────────────────────────────────────────────

/// `(alter-var-root #'v f & args)` — atomically apply `f` to the root value.
fn handle_alter_var_root(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "alter-var-root".into(),
            expected: "2+".into(),
            got: arg_forms.len(),
        });
    }
    let var_val = eval(&arg_forms[0], env)?;
    let f = eval(&arg_forms[1], env)?;
    let extra: Vec<Value> = arg_forms[2..]
        .iter()
        .map(|form| eval(form, env))
        .collect::<EvalResult<_>>()?;

    let vp = match &var_val {
        Value::Var(vp) => vp.clone(),
        v => {
            return Err(EvalError::Runtime(format!(
                "alter-var-root: expected var, got {}",
                v.type_name()
            )));
        }
    };
    let old_val = vp.get().deref().unwrap_or(Value::Nil);
    let mut call_args = vec![old_val.clone()];
    call_args.extend(extra);
    let new_val = apply_value(&f, call_args, env)?;
    vp.get().bind(new_val.clone());
    fire_watches(&vp.get().watches, &var_val, &old_val, &new_val, env);
    check_watch_error()?;
    Ok(new_val)
}

// ── vary-meta ────────────────────────────────────────────────────────────────

/// `(vary-meta obj f & args)` — apply `f` to obj's metadata, store result as new meta.
fn handle_vary_meta(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "vary-meta".into(),
            expected: "2+".into(),
            got: arg_forms.len(),
        });
    }
    let obj = eval(&arg_forms[0], env)?;
    let f = eval(&arg_forms[1], env)?;
    let extra: Vec<Value> = arg_forms[2..]
        .iter()
        .map(|form| eval(form, env))
        .collect::<EvalResult<_>>()?;

    let current_meta = match &obj {
        Value::Var(vp) => vp.get().get_meta().unwrap_or(Value::Nil),
        _ => Value::Nil,
    };
    let mut call_args = vec![current_meta];
    call_args.extend(extra);
    let new_meta = apply_value(&f, call_args, env)?;
    if let Value::Var(vp) = &obj {
        vp.get().set_meta(new_meta);
    }
    Ok(obj)
}

// ── Namespace reflection (env-needing) ────────────────────────────────────────

fn ns_name_from_val(v: &Value) -> Result<String, EvalError> {
    match v {
        Value::Symbol(s) => Ok(s.get().name.as_ref().to_string()),
        Value::Str(s) => Ok(s.get().clone()),
        Value::Namespace(ns) => Ok(ns.get().name.as_ref().to_string()),
        Value::Keyword(k) => Ok(k.get().name.as_ref().to_string()),
        other => Err(EvalError::Runtime(format!(
            "expected symbol, string, or namespace, got {}",
            other.type_name()
        ))),
    }
}

/// `(find-ns sym)` / `(the-ns sym)` — look up a namespace by name; nil if not found.
fn handle_find_ns(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "find-ns".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    let arg = eval(&arg_forms[0], env)?;
    let name = ns_name_from_val(&arg)?;
    let map = env.globals.namespaces.read().unwrap();
    match map.get(name.as_str()) {
        Some(ns) => Ok(Value::Namespace(ns.clone())),
        None => Ok(Value::Nil),
    }
}

/// `(all-ns)` — lazy sequence of all live namespaces.
fn handle_all_ns(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if !arg_forms.is_empty() {
        let _ = eval(&arg_forms[0], env)?; // tolerate extra args
    }
    let map = env.globals.namespaces.read().unwrap();
    let items: Vec<Value> = map
        .values()
        .map(|ns| Value::Namespace(ns.clone()))
        .collect();
    drop(map);
    Ok(Value::List(cljrs_gc::GcPtr::new(
        cljrs_value::PersistentList::from_iter(items),
    )))
}

/// `(create-ns sym)` — create (or return existing) namespace, return it.
fn handle_create_ns(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "create-ns".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    let arg = eval(&arg_forms[0], env)?;
    let name = ns_name_from_val(&arg)?;
    let ns = env.globals.get_or_create_ns(&name);
    Ok(Value::Namespace(ns))
}

/// `(ns-aliases ns)` — map of Symbol → Namespace for all aliases in ns.
fn handle_ns_aliases(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "ns-aliases".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    let ns_val = eval(&arg_forms[0], env)?;
    let ns_name = ns_name_from_val(&ns_val)?;
    let map = env.globals.namespaces.read().unwrap();
    let ns = match map.get(ns_name.as_str()) {
        Some(ns) => ns.clone(),
        None => return Ok(Value::Map(cljrs_value::MapValue::empty())),
    };
    let aliases = ns.get().aliases.lock().unwrap().clone();
    drop(map);
    let mut m = cljrs_value::MapValue::empty();
    for (alias, full_ns_name) in &aliases {
        let sym = Value::Symbol(cljrs_gc::GcPtr::new(cljrs_value::Symbol {
            namespace: None,
            name: alias.clone(),
        }));
        let nsmap = env.globals.namespaces.read().unwrap();
        if let Some(target_ns) = nsmap.get(full_ns_name.as_ref()) {
            m = m.assoc(sym, Value::Namespace(target_ns.clone()));
        }
    }
    Ok(Value::Map(m))
}

/// `(remove-ns sym)` — remove a namespace (returns nil; used sparingly in tests).
fn handle_remove_ns(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "remove-ns".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    let arg = eval(&arg_forms[0], env)?;
    let name = ns_name_from_val(&arg)?;
    env.globals
        .namespaces
        .write()
        .unwrap()
        .remove(name.as_str());
    Ok(Value::Nil)
}

/// `(alter-meta! ref f & args)` — apply f to ref's current meta + args, store and return new meta.
fn handle_alter_meta(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "alter-meta!".into(),
            expected: "2+".into(),
            got: arg_forms.len(),
        });
    }
    let obj = eval(&arg_forms[0], env)?;
    let f = eval(&arg_forms[1], env)?;
    let extra: Vec<Value> = arg_forms[2..]
        .iter()
        .map(|form| eval(form, env))
        .collect::<EvalResult<_>>()?;

    let current_meta = match &obj {
        Value::Var(vp) => vp
            .get()
            .get_meta()
            .unwrap_or(Value::Map(cljrs_value::MapValue::empty())),
        _ => Value::Map(cljrs_value::MapValue::empty()),
    };
    let mut call_args = vec![current_meta];
    call_args.extend(extra);
    let new_meta = apply_value(&f, call_args, env)?;
    if let Value::Var(vp) = &obj {
        vp.get().set_meta(new_meta.clone());
    }
    Ok(new_meta)
}

/// `(ns-resolve ns sym)` — return the Var for sym in ns, or nil if not found.
fn handle_ns_resolve(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "ns-resolve".into(),
            expected: "2".into(),
            got: arg_forms.len(),
        });
    }
    let ns_arg = eval(&arg_forms[0], env)?;
    let sym_arg = eval(&arg_forms[1], env)?;
    let ns_name = ns_name_from_val(&ns_arg)?;
    let sym_name = match &sym_arg {
        Value::Symbol(s) => s.get().name.as_ref().to_string(),
        Value::Str(s) => s.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "ns-resolve: second arg must be symbol or string, got {}",
                other.type_name()
            )));
        }
    };
    match env.globals.lookup_var(&ns_name, &sym_name) {
        Some(var_ptr) => Ok(Value::Var(var_ptr)),
        None => Ok(Value::Nil),
    }
}

/// Get the namespace name from `*ns*` (dynamic var), falling back to `env.current_ns`.
/// This is important for `resolve` inside macros, where `env.current_ns` is the
/// macro's defining namespace but `*ns*` is the caller's namespace.
fn resolve_current_ns(env: &Env) -> Arc<str> {
    if let Some(var) = env.globals.lookup_var("clojure.core", "*ns*") {
        let val = crate::dynamics::deref_var(&var);
        if let Some(Value::Namespace(ns_ptr)) = val {
            return ns_ptr.get().name.clone();
        }
    }
    env.current_ns.clone()
}

/// `(resolve sym)` — return the Var for sym in the current namespace, or nil.
fn handle_resolve(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() != 1 {
        return Err(EvalError::Arity {
            name: "resolve".into(),
            expected: "1".into(),
            got: arg_forms.len(),
        });
    }
    let resolve_ns = resolve_current_ns(env);
    let sym_arg = eval(&arg_forms[0], env)?;
    let sym_name = match &sym_arg {
        Value::Symbol(s) => {
            let sym = s.get();
            // If qualified (ns/name), use the given ns; otherwise current ns.
            if let Some(ns) = &sym.namespace {
                let full_ns = env
                    .globals
                    .resolve_alias(&resolve_ns, ns.as_ref())
                    .unwrap_or_else(|| ns.clone());
                return Ok(
                    match env.globals.lookup_var_in_ns(&full_ns, sym.name.as_ref()) {
                        Some(var_ptr) => Value::Var(var_ptr),
                        None => Value::Nil,
                    },
                );
            }
            sym.name.as_ref().to_string()
        }
        Value::Str(s) => s.get().clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "resolve: arg must be symbol or string, got {}",
                other.type_name()
            )));
        }
    };
    Ok(match env.globals.lookup_var_in_ns(&resolve_ns, &sym_name) {
        Some(var_ptr) => Value::Var(var_ptr),
        None => Value::Nil,
    })
}

fn handle_intern(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 || arg_forms.len() > 3 {
        return Err(EvalError::Runtime("intern expects 2 or 3 arguments".into()));
    }
    let ns_val = eval(&arg_forms[0], env)?;
    let ns_name: Arc<str> = match &ns_val {
        Value::Symbol(s) => s.get().name.clone(),
        Value::Namespace(ns) => ns.get().name.clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "intern: first arg must be namespace or symbol, got {}",
                other.type_name()
            )));
        }
    };
    let var_name: Arc<str> = match eval(&arg_forms[1], env)? {
        Value::Symbol(s) => s.get().name.clone(),
        other => {
            return Err(EvalError::Runtime(format!(
                "intern: second arg must be symbol, got {}",
                other.type_name()
            )));
        }
    };
    // Namespace must already exist (Clojure throws if it doesn't)
    let ns = {
        let map = env.globals.namespaces.read().unwrap();
        map.get(ns_name.as_ref()).cloned()
    };
    let ns = ns.ok_or_else(|| EvalError::Runtime(format!("No namespace: {ns_name} found")))?;
    let var = if arg_forms.len() == 3 {
        let val = eval(&arg_forms[2], env)?;
        let mut interns = ns.get().interns.lock().unwrap();
        if let Some(var) = interns.get(&var_name) {
            var.get().bind(val);
            var.clone()
        } else {
            let var =
                cljrs_gc::GcPtr::new(cljrs_value::Var::new(ns_name.clone(), var_name.clone()));
            var.get().bind(val);
            interns.insert(var_name, var.clone());
            var
        }
    } else {
        let mut interns = ns.get().interns.lock().unwrap();
        if let Some(var) = interns.get(&var_name) {
            var.clone()
        } else {
            let var =
                cljrs_gc::GcPtr::new(cljrs_value::Var::new(ns_name.clone(), var_name.clone()));
            interns.insert(var_name, var.clone());
            var
        }
    };
    Ok(Value::Var(var))
}

// ── bound-fn* ────────────────────────────────────────────────────────────────

/// `(bound-fn* f)` — capture current dynamic bindings and wrap `f` so that
/// when the wrapper is called, those bindings are installed.
fn handle_bound_fn_star(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() != 1 {
        return Err(EvalError::Arity {
            name: "bound-fn*".into(),
            expected: "1".into(),
            got: arg_forms.len(),
        });
    }
    let f = eval(&arg_forms[0], env)?;
    // Merge all binding frames into a single flat frame (bottom-up so inner wins)
    let frames = crate::dynamics::capture_current();
    let mut merged = std::collections::HashMap::new();
    for frame in &frames {
        merged.extend(frame.iter().map(|(k, v)| (*k, v.clone())));
    }
    Ok(Value::BoundFn(cljrs_gc::GcPtr::new(cljrs_value::BoundFn {
        wrapped: f,
        captured_bindings: merged,
    })))
}
