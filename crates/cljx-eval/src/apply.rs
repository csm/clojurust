//! Function application and the recur trampoline.

use std::sync::Arc;

use cljx_gc::GcPtr;
use cljx_reader::Form;
use cljx_value::{
    AgentFn, AgentMsg, Arity, CljxFn, CljxFnArity, Delay, LazySeq, PersistentList, Thunk, Value,
};

use crate::destructure::value_to_seq_vec;
use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use crate::eval::eval;

// ── ClosureThunk ──────────────────────────────────────────────────────────────

/// A Thunk that calls a zero-arg Clojure closure when forced.
#[derive(Debug)]
struct ClosureThunk {
    f: CljxFn,
    globals: std::sync::Arc<crate::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
}

impl Thunk for ClosureThunk {
    fn force(&self) -> Value {
        let mut env = Env::with_closure(self.globals.clone(), &self.ns, &self.f);
        call_cljx_fn(&self.f, vec![], &mut env).unwrap_or(Value::Nil)
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
        let expanded = macro_apply(mfn.get(), arg_forms, env)?;
        return eval(&expanded, env);
    }

    // Special case: `apply` native fn — spread last arg.
    if let Value::NativeFunction(nf) = &callee {
        match nf.get().name.as_ref() {
            "apply" => return handle_apply_call(arg_forms, env),
            "swap!" => return handle_swap_call(arg_forms, env),
            "make-lazy-seq" => return handle_make_lazy_seq(arg_forms, env),
            "make-delay" => return handle_make_delay(arg_forms, env),
            "vswap!" => return handle_vswap(arg_forms, env),
            "send" | "send-off" => return handle_send(arg_forms, env),
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
    match callee {
        Value::NativeFunction(nf) => {
            check_arity(&nf.get().arity, args.len(), &nf.get().name)?;
            (nf.get().func)(&args).map_err(|e| EvalError::Runtime(e.to_string()))
        }
        Value::Fn(f) => call_cljx_fn(f.get(), args, env),
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
            match args.first() {
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
                if s.get().contains(k) {
                    Ok(k.clone())
                } else {
                    Ok(Value::Nil)
                }
            }
            None => Ok(Value::Nil),
        },
        other => Err(EvalError::NotCallable(format!("{}", other))),
    }
}

/// Call a `CljxFn` with pre-evaluated args, with recur trampoline.
pub fn call_cljx_fn(f: &CljxFn, args: Vec<Value>, caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    // Create a fresh env with closure bindings.
    let mut env = Env::with_closure(caller_env.globals.clone(), &caller_env.current_ns, f);

    let mut current_args = args;
    loop {
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
        env.bind(
            rest.clone(),
            Value::List(GcPtr::new(PersistentList::from_iter(rest_items))),
        );
    }
    Ok(())
}

/// Eval a function body, propagating Recur up (does not catch it).
fn eval_body_recur_fn(body: &[cljx_reader::Form], env: &mut Env) -> EvalResult {
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
fn macro_apply(mfn: &CljxFn, arg_forms: &[Form], env: &mut Env) -> EvalResult<Form> {
    // Convert forms to values (unevaluated).
    let args: Vec<Value> = arg_forms
        .iter()
        .map(|f| Ok(crate::eval::form_to_value(f)))
        .collect::<EvalResult<_>>()?;

    let expanded_val = call_cljx_fn(mfn, args, env)?;
    let dummy_span = cljx_types::span::Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
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
            let agent_fn: AgentFn = Box::new(move |state| {
                let mut call_args = vec![state];
                call_args.extend(extra);
                let mut call_env = Env::new(globals, &ns);
                apply_value(&f, call_args, &mut call_env).map_err(|e| format!("{e}"))
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

/// Handle `(swap! atom f & args)`.
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

    let mut args = vec![atom.get().deref()];
    args.extend(evaled);
    let new_val = apply_value(&f, args, env)?;
    atom.get().reset(new_val.clone());
    Ok(new_val)
}
