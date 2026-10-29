use crate::env::env::{Env, GlobalEnv};
use crate::env::error::{EvalError, EvalResult};
use cljrs_value::{Arity, MapValue, Value};
use std::collections::{HashMap, HashSet};
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

#[derive(Default)]
pub(crate) struct HierarchySnapshot {
    pub(crate) generation: u64,
    parents: Option<MapValue>,
    ancestors: Option<MapValue>,
}

/// Snapshot the root hierarchy and its Var generation once per method-cache
/// miss. Reading the generation before the root means a concurrent bind can
/// only cache new data under an old generation, which the next lookup clears.
pub(crate) fn global_hierarchy_snapshot(globals: &GlobalEnv) -> HierarchySnapshot {
    let Some(var) = globals.lookup_var("clojure.core", "global-hierarchy") else {
        return HierarchySnapshot::default();
    };
    let generation = var.get().binding_generation();
    let Some(Value::Map(hierarchy)) = var.get().deref() else {
        return HierarchySnapshot {
            generation,
            ..HierarchySnapshot::default()
        };
    };
    let relation = |name| {
        hierarchy
            .get(&Value::keyword(cljrs_value::Keyword::simple(name)))
            .and_then(|value| match value {
                Value::Map(map) => Some(map),
                _ => None,
            })
    };
    HierarchySnapshot {
        generation,
        parents: relation("parents"),
        ancestors: relation("ancestors"),
    }
}

fn isa_with_ancestors(child: &Value, parent: &Value, ancestors: Option<&MapValue>) -> bool {
    if child == parent {
        return true;
    }
    if let Some(ancestors) = ancestors
        && let Some(Value::Set(of_child)) = ancestors.get(child)
        && of_child.contains(parent)
    {
        return true;
    }
    if let (Value::Vector(c), Value::Vector(p)) = (child, parent) {
        let (c, p) = (c.get(), p.get());
        return c.count() == p.count()
            && (0..c.count()).all(|i| match (c.nth(i), p.nth(i)) {
                (Some(cv), Some(pv)) => isa_with_ancestors(cv, pv, ancestors),
                _ => false,
            });
    }
    false
}

fn prefers_with_table(
    prefers: &HashMap<String, Vec<String>>,
    parents: Option<&MapValue>,
    x: &Value,
    y: &Value,
) -> bool {
    fn recur(
        prefers: &HashMap<String, Vec<String>>,
        parents: Option<&MapValue>,
        x: &Value,
        y: &Value,
        seen: &mut HashSet<(String, String)>,
    ) -> bool {
        let x_key = format!("{x}");
        let y_key = format!("{y}");
        if !seen.insert((x_key.clone(), y_key.clone())) {
            return false;
        }
        if prefers
            .get(&x_key)
            .is_some_and(|over| over.contains(&y_key))
        {
            return true;
        }
        let Some(parents) = parents else {
            return false;
        };
        if let Some(Value::Set(y_parents)) = parents.get(y)
            && y_parents
                .iter()
                .any(|parent| recur(prefers, Some(parents), x, parent, seen))
        {
            return true;
        }
        if let Some(Value::Set(x_parents)) = parents.get(x)
            && x_parents
                .iter()
                .any(|parent| recur(prefers, Some(parents), parent, y, seen))
        {
            return true;
        }
        false
    }

    recur(prefers, parents, x, y, &mut HashSet::new())
}

/// Clojure-compatible preference lookup, including preferences inherited
/// through the parents of either dispatch value.
pub(crate) fn prefers_in_hierarchy(
    mf: &cljrs_value::MultiFn,
    x: &Value,
    y: &Value,
    hierarchy: &HierarchySnapshot,
) -> bool {
    if cljrs_value::realization::is_inert(x)
        && cljrs_value::realization::is_inert(y)
        && hierarchy_is_inert(hierarchy)
    {
        return prefers_with_table(
            &mf.prefers.lock().unwrap(),
            hierarchy.parents.as_ref(),
            x,
            y,
        );
    }
    let roots: Vec<Value> = hierarchy
        .parents
        .iter()
        .chain(hierarchy.ancestors.iter())
        .cloned()
        .map(Value::Map)
        .collect();
    let _roots = crate::env::gc_roots::root_values(&roots);
    let prefers = mf.prefers.lock().unwrap().clone();
    prefers_with_table(&prefers, hierarchy.parents.as_ref(), x, y)
}

fn join_conflicts(matches: &[&(String, Value)]) -> String {
    match matches {
        [] => String::new(),
        [only] => only.0.clone(),
        [first, second] => format!("{} and {}", first.0, second.0),
        many => {
            let (last, initial) = many.split_last().unwrap();
            format!(
                "{}, and {}",
                initial
                    .iter()
                    .map(|entry| entry.0.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                last.0
            )
        }
    }
}

/// The method key a dispatch value inherits, or `None` when no registered
/// dispatch value is an ancestor of it.
///
/// Errors when several unrelated methods match and no `prefer-method` call
/// separates them — the same ambiguity Clojure reports.
fn hierarchy_method_key(
    mf: &cljrs_value::MultiFn,
    dispatch_val: &Value,
    hierarchy: &HierarchySnapshot,
) -> EvalResult<Option<String>> {
    let dispatch_vals = mf.dispatch_vals.lock().unwrap();
    if cljrs_value::realization::is_inert(dispatch_val)
        && dispatch_vals
            .values()
            .all(cljrs_value::realization::is_inert)
        && hierarchy_is_inert(hierarchy)
    {
        let prefers = mf.prefers.lock().unwrap();
        return hierarchy_method_key_with_tables(
            mf,
            dispatch_val,
            hierarchy,
            &dispatch_vals,
            &prefers,
        );
    }
    let snapshot = dispatch_vals.clone();
    drop(dispatch_vals);
    let dispatch_vals = snapshot;
    let mut roots: Vec<Value> = dispatch_vals.values().cloned().collect();
    roots.extend(
        hierarchy
            .parents
            .iter()
            .chain(hierarchy.ancestors.iter())
            .cloned()
            .map(Value::Map),
    );
    let _roots = crate::env::gc_roots::root_values(&roots);
    let prefers = mf.prefers.lock().unwrap().clone();
    hierarchy_method_key_with_tables(mf, dispatch_val, hierarchy, &dispatch_vals, &prefers)
}

fn hierarchy_is_inert(hierarchy: &HierarchySnapshot) -> bool {
    hierarchy.parents.iter().chain(hierarchy.ancestors.iter()).all(|map| {
        map.iter().all(|(key, value)| {
            cljrs_value::realization::is_inert(key)
                && matches!(value, Value::Set(set) if set.iter().all(cljrs_value::realization::is_inert))
        })
    })
}

fn hierarchy_method_key_with_tables(
    mf: &cljrs_value::MultiFn,
    dispatch_val: &Value,
    hierarchy: &HierarchySnapshot,
    dispatch_vals: &HashMap<String, Value>,
    prefers: &HashMap<String, Vec<String>>,
) -> EvalResult<Option<String>> {
    let mut matches: Vec<(String, Value)> = dispatch_vals
        .iter()
        .filter(|(k, v)| {
            k.as_str() != mf.default_dispatch
                && isa_with_ancestors(dispatch_val, v, hierarchy.ancestors.as_ref())
        })
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    matches.sort_by(|a, b| a.0.cmp(&b.0));
    if matches.len() <= 1 {
        return Ok(matches.into_iter().next().map(|(k, _)| k));
    }

    let dominates = |a: &(String, Value), b: &(String, Value)| {
        prefers_with_table(prefers, hierarchy.parents.as_ref(), &a.1, &b.1)
            || isa_with_ancestors(&a.1, &b.1, hierarchy.ancestors.as_ref())
    };
    let best: Vec<&(String, Value)> = matches
        .iter()
        .filter(|a| {
            !matches
                .iter()
                .any(|b| a.0 != b.0 && dominates(b, a) && !dominates(a, b))
        })
        .collect();
    match best.as_slice() {
        [only] => Ok(Some(only.0.clone())),
        _ => Err(EvalError::Runtime(format!(
            "Multiple methods in multimethod '{}' match dispatch value {}: {}, and {} is preferred",
            mf.name,
            dispatch_val,
            join_conflicts(&best),
            if best.len() == 2 { "neither" } else { "none" }
        ))),
    }
}

/// Return the canonical type tag for a value (used by protocol dispatch).
pub fn type_tag_of(val: &Value) -> Arc<str> {
    match val.unwrap_meta() {
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

/// What to add to a "No implementation" error when the method is registered
/// under the unqualified name of the type being dispatched on.
///
/// A record's tag is `my.ns.Point`. An `extend-type` or `extend-protocol` that
/// names `Point` from a namespace where the symbol does not resolve cannot tell
/// it from a native type's tag, so it registers under bare `Point` and
/// succeeds; the call is where that surfaces. Empty when there is no such
/// registration.
fn unqualified_impl_hint<V>(
    impls: &std::collections::HashMap<Arc<str>, std::collections::HashMap<Arc<str>, V>>,
    tag: &str,
    method: &str,
) -> String {
    let Some((_, bare)) = tag.rsplit_once('.') else {
        return String::new();
    };
    if !impls.get(bare).is_some_and(|m| m.contains_key(method)) {
        return String::new();
    }
    format!(
        " (an implementation is registered under the unqualified tag {bare}: \
         where it was extended, {bare} did not resolve to {tag}; \
         :import or :refer the type there)"
    )
}

/// Allocation-free check that `val`'s protocol dispatch tag equals `tag`.
///
/// Must agree exactly with [`type_tag_of`] — it exists so inline caches
/// (`rt_call_ic` in `cljrs-compiler`'s rt_abi) can validate a cached dispatch
/// tag on the hot path without building a fresh `Arc<str>` per call.
pub fn type_tag_matches(val: &Value, tag: &str) -> bool {
    // `type_tag_of` unwraps a metadata wrapper; so must this, or an annotated
    // dispatch value is a permanent inline-cache miss that re-resolves and
    // rewrites the entry under lock on every call.
    let val = val.unwrap_meta();
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
/// and `eval_call` in [`crate::interp`].
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
    cljrs_value::realization::catch(|| apply_value_inner(callee, args, env)).unwrap_or_else(
        |message| {
            Err(crate::env::error::value_error_to_eval_error(
                cljrs_value::ValueError::Other(message),
            ))
        },
    )
}

fn apply_value_inner(callee: &Value, args: Vec<Value>, env: &mut Env) -> EvalResult {
    // Root the callee and args so they survive any GC triggered at the safepoint.
    // These values are on the Rust stack but not yet in any Env frame.
    let _callee_root = crate::env::gc_roots::root_value(callee);
    let _args_root = crate::env::gc_roots::root_values(&args);

    // GC safepoint at function application boundary — blocks if collection is in progress,
    // and initiates collection if one was requested (memory pressure).
    crate::env::gc_roots::gc_safepoint(env)?;

    match callee {
        Value::NativeFunction(nf) => {
            crate::env::policy::check_native(&nf.get().name)?;
            check_arity(&nf.get().arity, args.len(), &nf.get().name)?;
            // A direct `(apply ...)` call is intercepted while forms are still
            // available, but apply is itself an IFn and can be passed to apply.
            // Handle that already-evaluated path here instead of invoking the
            // registration sentinel.
            if nf.get().name.as_ref() == "apply" {
                let callee = args[0].clone();
                let mut applied = args[1..args.len() - 1].to_vec();
                let spread = crate::builtins::builtins::value_to_seq(&args[args.len() - 1])
                    .map_err(crate::env::error::value_error_to_eval_error)?;
                applied.extend(spread);
                return apply_value(&callee, applied, env);
            }
            // Register the caller's env as a GC root: native functions may
            // call back into Clojure (via invoke()), which creates a fresh Env
            // and may trigger GC.
            let _caller_root = crate::env::gc_roots::push_env_root(env);
            let _context = crate::env::callback::scoped_eval_context(env);
            (nf.get().func)(&args).map_err(crate::env::error::value_error_to_eval_error)
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
            let _guard = crate::env::dynamics::push_frame(bf_ref.captured_bindings.clone());
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
                    let _impl_root = crate::env::gc_roots::root_value(&impl_fn);
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
                        "No implementation of protocol {} for type {}{}",
                        pf_ref.protocol.get().name,
                        tag,
                        unqualified_impl_hint(&impls, &tag, &pf_ref.method_name)
                    ))
                })?;
            drop(impls);
            let _impl_root = crate::env::gc_roots::root_value(&impl_fn);
            apply_value(&impl_fn, args, env)
        }
        Value::MultiFn(mf) => {
            let mf_ref = mf.get();
            let dispatch_val = apply_value(&mf_ref.dispatch_fn, args.clone(), env)?;
            let _dispatch_root = crate::env::gc_roots::root_value(&dispatch_val);
            cljrs_gc::safepoint();
            let key = format!("{}", dispatch_val);
            let exact = mf_ref.methods.lock().unwrap().get(&key).cloned();
            let impl_fn = match exact {
                Some(f) => f,
                None => {
                    let hierarchy = global_hierarchy_snapshot(&env.globals);
                    let method_generation = mf_ref.method_generation();
                    let cached =
                        mf_ref.cached_method(&key, hierarchy.generation, method_generation);
                    let method_key = match cached {
                        Some(cached) => cached,
                        None => {
                            let inherited =
                                hierarchy_method_key(mf_ref, &dispatch_val, &hierarchy)?;
                            let resolved = match inherited {
                                Some(k) => k,
                                None if mf_ref
                                    .methods
                                    .lock()
                                    .unwrap()
                                    .contains_key(&mf_ref.default_dispatch) =>
                                {
                                    mf_ref.default_dispatch.clone()
                                }
                                None => {
                                    return Err(EvalError::Runtime(format!(
                                        "No method in multimethod '{}' for dispatch value {}",
                                        mf_ref.name, key
                                    )));
                                }
                            };
                            mf_ref.cache_method(
                                key.clone(),
                                resolved.clone(),
                                hierarchy.generation,
                                method_generation,
                            );
                            resolved
                        }
                    };
                    mf_ref
                        .methods
                        .lock()
                        .unwrap()
                        .get(&method_key)
                        .cloned()
                        .ok_or_else(|| {
                            EvalError::Runtime(format!(
                                "No method in multimethod '{}' for dispatch value {}",
                                mf_ref.name, key
                            ))
                        })?
                }
            };
            let _impl_root = crate::env::gc_roots::root_value(&impl_fn);
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
                Some(Value::Set(set)) => {
                    if set.contains(callee) {
                        Ok(callee.clone())
                    } else {
                        Ok(default())
                    }
                }
                Some(Value::TransientMap(map)) => Ok(map.get().get(callee).unwrap_or_else(default)),
                Some(Value::TransientSet(set)) => {
                    if set.get().contains(callee) {
                        Ok(callee.clone())
                    } else {
                        Ok(default())
                    }
                }
                Some(Value::Nil) => Ok(default()),
                _ => Ok(Value::Nil),
            }
        }
        Value::Vector(vector) => {
            if args.len() != 1 {
                return Err(EvalError::Arity {
                    name: "vector".into(),
                    expected: "1".into(),
                    got: args.len(),
                });
            }
            let Value::Long(index) = args[0] else {
                return Err(EvalError::Runtime(format!(
                    "vector index must be an integer, got {}",
                    args[0].type_name()
                )));
            };
            let found = usize::try_from(index)
                .ok()
                .and_then(|index| vector.get().nth(index).cloned());
            match found {
                Some(value) => Ok(value),
                None => Err(EvalError::Runtime(format!(
                    "vector index out of bounds: {index}"
                ))),
            }
        }
        Value::TransientVector(vector) => {
            if !(1..=2).contains(&args.len()) {
                return Err(EvalError::Arity {
                    name: "transient-vector".into(),
                    expected: "1 or 2".into(),
                    got: args.len(),
                });
            }
            let Value::Long(index) = args[0] else {
                return Err(EvalError::Runtime(format!(
                    "vector index must be an integer, got {}",
                    args[0].type_name()
                )));
            };
            let found = usize::try_from(index)
                .ok()
                .and_then(|index| vector.get().get(index));
            match (found, args.get(1)) {
                (Some(value), _) => Ok(value),
                (None, Some(default)) => Ok(default.clone()),
                (None, None) => Err(EvalError::Runtime(format!(
                    "vector index out of bounds: {index}"
                ))),
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
        Value::TransientMap(map) => match args.first() {
            Some(key) => Ok(map
                .get()
                .get(key)
                .unwrap_or(args.get(1).cloned().unwrap_or(Value::Nil))),
            None => Ok(Value::Nil),
        },
        Value::TransientSet(set) => match args.first() {
            Some(value) if set.get().contains(value) => Ok(value.clone()),
            Some(_) | None => Ok(Value::Nil),
        },
        Value::WithMeta(inner, _) => apply_value(inner, args, env),
        Value::Var(v) => {
            // Vars in function position are transparently deref'd (IFn on Var).
            // The IR interpreter uses DefVar to create per-call mutable cells for
            // letfn / named-fn self-recursion; those cells are captured as
            // Value::Var and called directly.
            let inner = crate::env::dynamics::deref_var(v).ok_or_else(|| {
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

#[cfg(test)]
mod unqualified_impl_hint_tests {
    use super::unqualified_impl_hint;
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    proptest! {
        /// A hint is given exactly when the dispatch tag is qualified and the
        /// method called is registered under the tag's last segment; it then
        /// names both tags. An impl of another method under the bare name, or
        /// of this method under another tag, does not earn one.
        #[test]
        fn a_hint_is_given_iff_the_method_sits_under_the_bare_name(
            ns in "[a-z]{1,4}(\\.[a-z]{1,4}){0,2}",
            name in "[A-Z][a-z]{0,4}",
            method in "[a-z]{1,4}",
            qualified in any::<bool>(),
            method_under_bare in any::<bool>(),
            other_method_under_bare in any::<bool>(),
            method_under_other_tag in any::<bool>(),
        ) {
            let mut impls: HashMap<Arc<str>, HashMap<Arc<str>, ()>> = HashMap::new();
            let mut register = |tag: &str, method: &str| {
                impls
                    .entry(Arc::from(tag))
                    .or_default()
                    .insert(Arc::from(method), ());
            };
            if method_under_bare {
                register(&name, &method);
            }
            if other_method_under_bare {
                register(&name, &format!("{method}-other"));
            }
            if method_under_other_tag {
                register(&format!("{name}Other"), &method);
                register(&format!("other.{ns}.{name}"), &method);
            }
            let tag = if qualified { format!("{ns}.{name}") } else { name.clone() };

            let hint = unqualified_impl_hint(&impls, &tag, &method);

            prop_assert_eq!(!hint.is_empty(), qualified && method_under_bare, "{}", hint);
            if !hint.is_empty() {
                prop_assert!(hint.contains(&format!("unqualified tag {name}:")), "{}", hint);
                prop_assert!(hint.contains(&format!("resolve to {tag};")), "{}", hint);
            }
        }
    }
}
