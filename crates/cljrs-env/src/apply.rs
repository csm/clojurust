use crate::env::Env;
use crate::error::{EvalError, EvalResult};
use cljrs_value::{Arity, Value};
use std::sync::Arc;

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

/// Allocation-free check that `val`'s protocol dispatch tag equals `tag`.
///
/// Must agree exactly with [`type_tag_of`] — it exists so inline caches
/// (`rt_call_ic` in `cljrs-compiler`'s rt_abi) can validate a cached dispatch
/// tag on the hot path without building a fresh `Arc<str>` per call.
pub fn type_tag_matches(val: &Value, tag: &str) -> bool {
    match val {
        Value::TypeInstance(ti) => &*ti.get().type_tag == tag,
        Value::NativeObject(obj) => obj.get().type_tag() == tag,
        _ => {
            // All remaining variants map to a static tag; compare without
            // allocating.  `type_tag_of` is the source of truth.
            match val {
                Value::Nil => "nil",
                Value::Bool(_) => "Boolean",
                Value::Long(_) => "Long",
                Value::Double(_) => "Double",
                Value::BigInt(_) => "BigInt",
                Value::BigDecimal(_) => "BigDecimal",
                Value::Ratio(_) => "Ratio",
                Value::Char(_) => "Character",
                Value::Str(_) => "String",
                Value::Keyword(_) => "Keyword",
                Value::Symbol(_) => "Symbol",
                Value::List(_) | Value::Cons(_) | Value::LazySeq(_) => "List",
                Value::Vector(_) => "Vector",
                Value::Map(_) => "Map",
                Value::Set(_) => "Set",
                Value::Fn(_)
                | Value::NativeFunction(_)
                | Value::ProtocolFn(_)
                | Value::MultiFn(_) => "Fn",
                Value::Atom(_) => "Atom",
                Value::Var(_) => "Var",
                Value::Protocol(_) => "Protocol",
                Value::Volatile(_) => "Volatile",
                Value::Delay(_) => "Delay",
                Value::Promise(_) => "Promise",
                Value::Future(_) => "Future",
                Value::Agent(_) => "Agent",
                Value::Resource(_) => "Resource",
                _ => "Object",
            }
        }
        .eq(tag),
    }
}

/// If `callee` is an `^:async` Clojure function and an async runtime is
/// registered, spawn its body as a task and return a `Value::Future`.
///
/// Returns `None` when there is no async runtime or the callee is not an async
/// function, in which case the caller proceeds with the normal synchronous
/// call path. This is the single dispatch point shared by `apply_value` here
/// and `eval_call` in `cljrs-interp`.
pub fn dispatch_if_async(callee: &Value, args: &[Value], env: &Env) -> Option<Value> {
    let Value::Fn(f) = callee else { return None };
    if !f.get().is_async {
        return None;
    }
    let rt = env.globals.async_runtime()?;
    let call_env = Env::new(env.globals.clone(), &env.current_ns);
    Some(rt.spawn_async_call(callee.clone(), args.to_vec(), call_env))
}

/// Apply `callee` to the already-evaluated `args`.
pub fn apply_value(callee: &Value, args: Vec<Value>, env: &mut Env) -> EvalResult {
    // Root the callee and args so they survive any GC triggered at the safepoint.
    // These values are on the Rust stack but not yet in any Env frame.
    let _callee_root = crate::gc_roots::root_value(callee);
    let _args_root = crate::gc_roots::root_values(&args);

    // GC safepoint at function application boundary — blocks if collection is in progress,
    // and initiates collection if one was requested (memory pressure).
    crate::gc_roots::gc_safepoint(env);

    match callee {
        Value::NativeFunction(nf) => {
            check_arity(&nf.get().arity, args.len(), &nf.get().name)?;
            // Register the caller's env as a GC root: native functions may
            // call back into Clojure (via invoke()), which creates a fresh Env
            // and may trigger GC.
            let _caller_root = crate::gc_roots::push_env_root(env);
            crate::callback::push_eval_context(env);
            let result = (nf.get().func)(&args).map_err(crate::error::value_error_to_eval_error);
            crate::callback::pop_eval_context();
            result
        }
        Value::Fn(f) => {
            if let Some(fut) = dispatch_if_async(callee, &args, env) {
                return Ok(fut);
            }
            env.call_cljrs_fn(f.get(), &args)
        }
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

            // `(defprotocol P :extend-via-metadata true ...)` — an instance
            // implements the protocol by carrying an impl fn in its metadata,
            // keyed by the fully-qualified symbol naming the protocol method
            // (e.g. `` (with-meta {} {`my-method (fn [this] ...)}) ``, which
            // syntax-quote expands to `{my.ns/my-method (fn [this] ...)}`).
            // This mirrors real Clojure's `MethodImplCache` dispatch, which
            // looks the method up in `(meta x)` by `(.sym cache)` — the var's
            // qualified symbol, not the callable itself.  Metadata impls win
            // over type-tag impls, and apply even to values (like a plain
            // map) with no `extend-type`.
            if pf_ref.protocol.get().extend_via_metadata
                && let Some(Value::Map(m)) = dispatch_val.get_meta()
            {
                let proto = pf_ref.protocol.get();
                let method_sym = Value::Symbol(cljrs_gc::GcPtr::new(
                    cljrs_value::Symbol::qualified(proto.ns.clone(), pf_ref.method_name.clone()),
                ));
                if let Some(impl_fn) = m.get(&method_sym) {
                    let _impl_root = crate::gc_roots::root_value(&impl_fn);
                    return apply_value(&impl_fn, args, env);
                }
            }

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
            let _impl_root = crate::gc_roots::root_value(&impl_fn);
            apply_value(&impl_fn, args, env)
        }
        Value::MultiFn(mf) => {
            let mf_ref = mf.get();
            let dispatch_val = apply_value(&mf_ref.dispatch_fn, args.clone(), env)?;
            let _dispatch_root = crate::gc_roots::root_value(&dispatch_val);
            cljrs_gc::safepoint();
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
            let _impl_root = crate::gc_roots::root_value(&impl_fn);
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
        Value::WithMeta(inner, _) => apply_value(inner, args, env),
        Value::Var(v) => {
            // Vars in function position are transparently deref'd (IFn on Var).
            // The IR interpreter uses DefVar to create per-call mutable cells for
            // letfn / named-fn self-recursion; those cells are captured as
            // Value::Var and called directly.
            let inner = crate::dynamics::deref_var(v).ok_or_else(|| {
                EvalError::Runtime(format!(
                    "unbound var {}/{} used as function",
                    v.get().namespace,
                    v.get().name,
                ))
            })?;
            apply_value(&inner, args, env)
        }
        other => Err(EvalError::NotCallable(format!(
            "<{}> is not callable",
            other.type_name()
        ))),
    }
}
