//! Function application and the recur trampoline.

use cljrs_builtins::form::form_to_value;
use cljrs_gc::GcPtr;
use cljrs_reader::{Form, FormKind};
use cljrs_value::{
    Agent, AgentFn, AgentMsg, Atom, CljxFn, CljxFnArity, Delay, LazySeq, MapValue, PersistentList,
    Symbol, Thunk, Value, Volatile,
};
use std::collections::HashMap;
use std::sync::Arc;

use crate::destructure::value_to_seq_vec;
use crate::eval::eval;
use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};

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
        if let Err(e) = cljrs_env::apply::apply_value(f, args, env) {
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

// ── ClosureThunk ──────────────────────────────────────────────────────────────

/// A Thunk that calls a zero-arg Clojure closure when forced.
#[derive(Debug)]
pub struct ClosureThunk {
    pub f: CljxFn,
    pub globals: std::sync::Arc<cljrs_env::env::GlobalEnv>,
    pub ns: std::sync::Arc<str>,
}

/// A Thunk that wraps any zero-arg callable `Value` (a `Value::Fn`, a
/// `Value::NativeFunction`, etc.) and forces by routing through
/// `apply_value`.  Used by [`make_lazy_seq_from_fn`] when the supplied
/// value isn't a plain Clojure fn — for example the IR interpreter's
/// `AllocClosure` produces a `NativeFunction` wrapping the IR closure.
struct CallableValueThunk {
    callee: Value,
    globals: std::sync::Arc<cljrs_env::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
}

impl std::fmt::Debug for CallableValueThunk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallableValueThunk")
            .field("callee", &self.callee.type_name())
            .field("ns", &self.ns)
            .finish()
    }
}

impl cljrs_gc::Trace for CallableValueThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.callee.trace(visitor);
    }
}

impl Thunk for CallableValueThunk {
    fn force(&self) -> Result<Value, String> {
        let mut env = Env::new(self.globals.clone(), &self.ns);
        cljrs_env::apply::apply_value(&self.callee, Vec::new(), &mut env)
            .map_err(|e| format!("{e}"))
    }
}

/// Wrap a zero-arg callable value in a `Value::LazySeq` whose `force`
/// calls it.
///
/// Accepts any `Value` whose type-name is "fn" — `Value::Fn`,
/// `Value::NativeFunction`, `Value::BoundFn`, etc. — and unwraps any
/// surrounding `WithMeta`.  The fast path stays in `Value::Fn` (a direct
/// `ClosureThunk`); other callables route through a thunk that dispatches
/// via `apply_value` on force.
///
/// This is the value-level analogue of [`handle_make_lazy_seq`], usable
/// from contexts that already have a `Value` (e.g. the IR interpreter)
/// rather than a `Form`.
pub fn make_lazy_seq_from_fn(
    f_val: &Value,
    globals: std::sync::Arc<cljrs_env::env::GlobalEnv>,
    ns: std::sync::Arc<str>,
) -> EvalResult {
    let unwrapped = f_val.unwrap_meta();
    if let Value::Fn(g) = unwrapped {
        let thunk = ClosureThunk {
            f: g.get().clone(),
            globals,
            ns,
        };
        return Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))));
    }
    // Anything else with type-name "fn" is acceptable; route through
    // apply_value at force-time.  Reject non-callable values up front.
    if unwrapped.type_name() != "fn" {
        return Err(EvalError::Runtime(format!(
            "make-lazy-seq requires a fn, got {}",
            unwrapped.type_name(),
        )));
    }
    let thunk = CallableValueThunk {
        callee: unwrapped.clone(),
        globals,
        ns,
    };
    Ok(Value::LazySeq(GcPtr::new(LazySeq::new(Box::new(thunk)))))
}

impl cljrs_gc::Trace for ClosureThunk {
    fn trace(&self, visitor: &mut cljrs_gc::MarkVisitor) {
        self.f.trace(visitor);
    }
}

impl Thunk for ClosureThunk {
    fn force(&self) -> Result<Value, String> {
        // Root the closed-over values so they survive GC.  The thunk may live
        // on the Rust stack outside any Env frame (e.g., after LazySeq::realize
        // drops its Mutex guard), so GC wouldn't trace them otherwise.
        let _closed_root = cljrs_env::gc_roots::root_values(&self.f.closed_over_vals);
        let mut env = Env::with_closure(self.globals.clone(), &self.ns, &self.f);
        call_cljrs_fn(&self.f, &[], &mut env).map_err(|e| format!("{e}"))
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
    // Interop: (.methodName target args...) — method call syntax.
    if let FormKind::Symbol(s) = &func_form.kind
        && let Some(method) = s.strip_prefix('.')
        && !method.is_empty()
        && method != "."
    {
        return eval_method_call(method, arg_forms, env);
    }

    // Evaluate the callee first.
    let callee = eval(func_form, env)?;

    // Root the callee so it survives any GC triggered during argument evaluation.
    let _callee_root = cljrs_env::gc_roots::root_value(&callee);

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
            "volatile!" => return handle_volatile(arg_forms, env),
            "vreset!" => return handle_vreset(arg_forms, env),
            "agent" => return handle_agent_call(arg_forms, env),
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

    // Evaluate arguments one-at-a-time, rooting partial results so that
    // previously-evaluated args survive any GC triggered during later evals.
    let mut args: Vec<Value> = Vec::with_capacity(arg_forms.len());
    for f in arg_forms {
        // Root the already-evaluated args before each eval that could trigger GC.
        let _args_root = cljrs_env::gc_roots::root_values(&args);
        args.push(eval(f, env)?);
    }

    // For Clojure functions, call directly to avoid the extra stack frames
    // from cljrs_env::apply → env.call_cljrs_fn → globals.call_cljrs_fn → fn ptr.
    //
    // NOTE: this also bypasses the IR cache lookup in
    // `cljrs_eval::apply::call_cljrs_fn`, so prebuilt-IR / eagerly-lowered
    // arities never run through the tier-1 IR interpreter on direct fn
    // calls — they only fire via `apply_value` paths (e.g. higher-order
    // calls in `apply` / `reduce` / callbacks).  The bypass exists because
    // the IR interpreter currently has incomplete coverage of the
    // syntax-level special operations that `eval_call` short-circuits
    // (e.g. `swap!` / `vswap!` / `make-lazy-seq` / `make-delay` /
    // `alter-var-root` / etc.) — when cached IR uses them transitively,
    // dispatch hits a sentinel that intentionally errors, breaking
    // anything as common as `(map …)` from inside a prebuilt arity.
    if let Value::Fn(f) = &callee {
        let _args_root = cljrs_env::gc_roots::root_values(&args);
        cljrs_env::gc_roots::gc_safepoint(env);
        return call_cljrs_fn(f.get(), &args, env);
    }

    cljrs_env::apply::apply_value(&callee, args, env)
}

// ── Interop method calls ─────────────────────────────────────────────────────

/// Evaluate `(.methodName target args...)` interop syntax.
///
/// Currently supports a small set of methods on built-in types:
/// - `.indexOf` on strings and vectors
/// - `.startsWith`, `.endsWith`, `.contains`, `.substring`, `.length`,
///   `.charAt`, `.toUpperCase`, `.toLowerCase`, `.trim`, `.replace`,
///   `.split` on strings
fn eval_method_call(method: &str, arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Runtime(format!(
            ".{method} requires a target object"
        )));
    }
    let target = eval(&arg_forms[0], env)?;
    let args: Vec<Value> = arg_forms[1..]
        .iter()
        .map(|f| eval(f, env))
        .collect::<EvalResult<_>>()?;

    dispatch_method(method, &target, &args)
}

fn dispatch_method(method: &str, target: &Value, args: &[Value]) -> EvalResult {
    match target {
        Value::Str(s) => dispatch_string_method(method, s.get(), args),
        Value::Vector(v) => dispatch_vector_method(method, v, args),
        Value::List(_) | Value::Cons(_) | Value::LazySeq(_) => {
            dispatch_seq_method(method, target, args)
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on type {}",
            target.type_name()
        ))),
    }
}

fn dispatch_string_method(method: &str, s: &str, args: &[Value]) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = match args.first() {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                Some(v) => {
                    return Err(EvalError::Runtime(format!(
                        ".indexOf expects string or char argument, got {}",
                        v.type_name()
                    )));
                }
                None => return Err(EvalError::Runtime(".indexOf requires an argument".into())),
            };
            match s.find(&needle) {
                Some(pos) => Ok(Value::Long(pos as i64)),
                None => Ok(Value::Long(-1)),
            }
        }
        "lastIndexOf" => {
            let needle = match args.first() {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                _ => {
                    return Err(EvalError::Runtime(
                        ".lastIndexOf requires a string or char argument".into(),
                    ));
                }
            };
            match s.rfind(&needle) {
                Some(pos) => Ok(Value::Long(pos as i64)),
                None => Ok(Value::Long(-1)),
            }
        }
        "startsWith" => {
            let prefix = require_str_arg(args, ".startsWith")?;
            Ok(Value::Bool(s.starts_with(&prefix)))
        }
        "endsWith" => {
            let suffix = require_str_arg(args, ".endsWith")?;
            Ok(Value::Bool(s.ends_with(&suffix)))
        }
        "contains" => {
            let sub = require_str_arg(args, ".contains")?;
            Ok(Value::Bool(s.contains(&sub)))
        }
        "length" => Ok(Value::Long(s.len() as i64)),
        "isEmpty" => Ok(Value::Bool(s.is_empty())),
        "charAt" => {
            let idx = require_long_arg(args, ".charAt")? as usize;
            s.chars()
                .nth(idx)
                .map(Value::Char)
                .ok_or_else(|| EvalError::Runtime(format!(".charAt index {idx} out of bounds")))
        }
        "substring" => {
            let start = require_long_arg(args, ".substring")? as usize;
            let end = args
                .get(1)
                .map(|v| match v {
                    Value::Long(n) => Ok(*n as usize),
                    _ => Err(EvalError::Runtime(
                        ".substring end must be an integer".into(),
                    )),
                })
                .transpose()?;
            let result = match end {
                Some(e) => &s[start..e.min(s.len())],
                None => &s[start..],
            };
            Ok(Value::Str(GcPtr::new(result.to_string())))
        }
        "toUpperCase" => Ok(Value::Str(GcPtr::new(s.to_uppercase()))),
        "toLowerCase" => Ok(Value::Str(GcPtr::new(s.to_lowercase()))),
        "trim" => Ok(Value::Str(GcPtr::new(s.trim().to_string()))),
        "replace" => {
            let from = require_str_arg(args, ".replace")?;
            let to = match args.get(1) {
                Some(Value::Str(s)) => s.get().to_string(),
                Some(Value::Char(c)) => c.to_string(),
                _ => {
                    return Err(EvalError::Runtime(
                        ".replace requires two string arguments".into(),
                    ));
                }
            };
            Ok(Value::Str(GcPtr::new(s.replace(&from, &to))))
        }
        "split" => {
            let sep = require_str_arg(args, ".split")?;
            let parts: Vec<Value> = s
                .split(&sep)
                .map(|p| Value::Str(GcPtr::new(p.to_string())))
                .collect();
            Ok(Value::Vector(GcPtr::new(
                cljrs_value::PersistentVector::from_iter(parts),
            )))
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on String"
        ))),
    }
}

fn dispatch_vector_method(
    method: &str,
    v: &GcPtr<cljrs_value::PersistentVector>,
    args: &[Value],
) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = args
                .first()
                .ok_or_else(|| EvalError::Runtime(".indexOf requires an argument".into()))?;
            for (i, item) in v.get().iter().enumerate() {
                if item == needle {
                    return Ok(Value::Long(i as i64));
                }
            }
            Ok(Value::Long(-1))
        }
        "size" | "count" => Ok(Value::Long(v.get().count() as i64)),
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on Vector"
        ))),
    }
}

fn dispatch_seq_method(method: &str, target: &Value, args: &[Value]) -> EvalResult {
    match method {
        "indexOf" => {
            let needle = args
                .first()
                .ok_or_else(|| EvalError::Runtime(".indexOf requires an argument".into()))?;
            let items = crate::destructure::value_to_seq_vec(target);
            for (i, item) in items.iter().enumerate() {
                if item == needle {
                    return Ok(Value::Long(i as i64));
                }
            }
            Ok(Value::Long(-1))
        }
        _ => Err(EvalError::Runtime(format!(
            ".{method} not supported on {}",
            target.type_name()
        ))),
    }
}

fn require_str_arg(args: &[Value], method: &str) -> Result<String, EvalError> {
    match args.first() {
        Some(Value::Str(s)) => Ok(s.get().to_string()),
        Some(Value::Char(c)) => Ok(c.to_string()),
        _ => Err(EvalError::Runtime(format!(
            "{method} requires a string argument"
        ))),
    }
}

fn require_long_arg(args: &[Value], method: &str) -> Result<i64, EvalError> {
    match args.first() {
        Some(Value::Long(n)) => Ok(*n),
        _ => Err(EvalError::Runtime(format!(
            "{method} requires an integer argument"
        ))),
    }
}

/// Resolve a type symbol from `extend-type` to a canonical tag.
/// Canonical tags ARE the short names, so this just passes through.
pub fn resolve_type_tag(sym: &str) -> Arc<str> {
    Arc::from(sym)
}

/// Tree-walking execution path (original implementation).
pub fn call_cljrs_fn(f: &CljxFn, args: &[Value], caller_env: &mut Env) -> EvalResult {
    let arity = select_arity(f, args.len())?;

    // Register the caller's env as a GC root so its local bindings survive
    // any collection triggered while we're executing the callee's body.
    let _caller_root = cljrs_env::gc_roots::push_env_root(caller_env);

    // Create a fresh env with closure bindings, executing in the defining namespace.
    // This ensures macros qualify symbols relative to their definition site.
    let mut env = Env::with_closure(caller_env.globals.clone(), &f.defining_ns, f);

    let mut current_args = Vec::from(args);
    loop {
        // Root current_args on the shadow stack so they survive GC.
        // They haven't been bound into the env yet.
        let _args_root = cljrs_env::gc_roots::root_values(&current_args);

        // GC safepoint before entering function body
        cljrs_env::gc_roots::gc_safepoint(&env);

        env.push_frame();

        // Bind params.
        bind_fn_params(arity, &current_args, &mut env)?;

        // Self-reference for named functions.
        if let Some(ref name) = f.name {
            let self_val = Value::Fn(GcPtr::new(f.clone()));
            env.bind(name.clone(), self_val);
        }

        // Eval body, catching Recur.
        // Under no-gc: push a scratch region; evaluate all-but-last in it,
        // then pop scratch before the tail expression so the return value
        // lands in the caller's allocation context.
        #[cfg(not(feature = "no-gc"))]
        let result = eval_body_recur_fn(&arity.body, &mut env);
        #[cfg(feature = "no-gc")]
        let result = {
            let mut scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
            // scratch drops here: resets the region (frees intermediates)
            eval_body_with_scratch(&arity.body, &mut scratch, &mut env)
        };
        env.pop_frame();

        match result {
            Ok(v) => return Ok(v),
            Err(EvalError::Recur(new_args)) => {
                // For variadic arities, recur provides n+1 values where the
                // last value IS the rest collection (not spread args to be
                // re-collected). Flatten it so bind_fn_params sees the right
                // number of individual args.
                if arity.rest_param.is_some() {
                    let n = arity.params.len();
                    if new_args.len() == n + 1 {
                        let mut flat = new_args[..n].to_vec();
                        // Spread the rest collection back into individual args.
                        let rest_val = &new_args[n];
                        match rest_val {
                            Value::Nil => {} // no extra args
                            _ => {
                                let rest_items = value_to_seq_vec(rest_val);
                                flat.extend(rest_items);
                            }
                        }
                        current_args = flat;
                    } else {
                        current_args = new_args;
                    }
                } else {
                    current_args = new_args;
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// Bind function parameters in the current (top) frame.
pub fn bind_fn_params(arity: &CljxFnArity, args: &[Value], env: &mut Env) -> EvalResult<()> {
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
            // When the rest pattern is a map destructure (e.g. `& {:keys [bar]}`),
            // convert the rest args list into a map of alternating key-value pairs,
            // matching Clojure's keyword-arguments convention.
            let destructure_val = if matches!(pattern.kind, FormKind::Map(_)) {
                let items = value_to_seq_vec(&rest_val);
                Value::Map(MapValue::from_flat_entries(items))
            } else {
                rest_val
            };
            crate::destructure::bind_pattern(pattern, destructure_val, env)?;
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
#[cfg(not(feature = "no-gc"))]
fn eval_body_recur_fn(body: &[cljrs_reader::Form], env: &mut Env) -> EvalResult {
    let mut result = Value::Nil;
    for form in body {
        result = eval(form, env)?;
    }
    Ok(result)
}

/// Under `no-gc`: evaluate body forms with the scratch region active for all
/// non-tail forms, then pop the scratch before the tail expression so the
/// return value (or `recur` args) are allocated in the caller's context.
#[cfg(feature = "no-gc")]
fn eval_body_with_scratch(
    body: &[cljrs_reader::Form],
    scratch: &mut cljrs_gc::alloc_ctx::ScratchGuard,
    env: &mut Env,
) -> EvalResult {
    if body.is_empty() {
        scratch.pop_for_return();
        return Ok(Value::Nil);
    }
    // Eval all non-tail forms in the scratch region.
    for form in &body[..body.len() - 1] {
        eval(form, env)?;
    }
    // Pop scratch so the tail expression allocates in the caller's context.
    scratch.pop_for_return();
    eval(&body[body.len() - 1], env)
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
        let mut items = vec![form_to_value(func_form)];
        for f in arg_forms {
            items.push(form_to_value(f));
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
    args.extend(arg_forms.iter().map(form_to_value));

    let expanded_val = call_cljrs_fn(mfn, args.as_ref(), env)?;
    let dummy_span = cljrs_types::span::Span::new(Arc::new("<macro>".to_string()), 0, 0, 1, 1);
    crate::macros::value_to_form(&expanded_val, dummy_span)
}

/// Handle `(apply f arg1 ... last-coll)` — spread the last arg.
fn handle_apply_call(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    let mut evaled: Vec<Value> = Vec::with_capacity(arg_forms.len());
    for f in arg_forms {
        let _root = cljrs_env::gc_roots::root_values(&evaled);
        evaled.push(eval(f, env)?);
    }

    if evaled.len() < 2 {
        return Err(EvalError::Arity {
            name: "apply".into(),
            expected: "2+".into(),
            got: evaled.len(),
        });
    }

    let f = evaled.remove(0);
    let last = evaled.pop().unwrap();
    // Root f, last, and remaining evaled args during spread (which may realize lazy seqs).
    let _f_root = cljrs_env::gc_roots::root_value(&f);
    let _last_root = cljrs_env::gc_roots::root_value(&last);
    let _evaled_root = cljrs_env::gc_roots::root_values(&evaled);
    // Spread last arg.
    let spread = value_to_seq_vec(&last);
    evaled.extend(spread);
    cljrs_env::apply::apply_value(&f, evaled, env)
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
            // Under no-gc: the value written into the volatile must live in the
            // StaticArena since the volatile outlives all scratch regions.
            #[cfg(feature = "no-gc")]
            let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
            let new_val = cljrs_env::apply::apply_value(&f, call_args, env)?;
            v.get().reset(new_val.clone());
            Ok(new_val)
        }
        other => Err(EvalError::Runtime(format!(
            "vswap!: expected volatile, got {}",
            other.type_name()
        ))),
    }
}

// ── volatile! ────────────────────────────────────────────────────────────────

/// Handle `(volatile! init-val)`.
fn handle_volatile(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "volatile!".into(),
            expected: "1".into(),
            got: 0,
        });
    }
    // Under no-gc: volatile initial value must live in the StaticArena since
    // the Volatile container outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let initial = eval(&arg_forms[0], env)?;
    Ok(Value::Volatile(GcPtr::new(Volatile::new(initial))))
}

// ── vreset! ──────────────────────────────────────────────────────────────────

/// Handle `(vreset! vol new-val)`.
fn handle_vreset(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.len() < 2 {
        return Err(EvalError::Arity {
            name: "vreset!".into(),
            expected: "2".into(),
            got: arg_forms.len(),
        });
    }
    let vol_val = eval(&arg_forms[0], env)?;
    // Under no-gc: the new value written into the volatile must live in the
    // StaticArena since the volatile outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = eval(&arg_forms[1], env)?;
    match &vol_val {
        Value::Volatile(v) => {
            v.get().reset(new_val.clone());
            Ok(new_val)
        }
        other => Err(EvalError::Runtime(format!(
            "vreset!: expected volatile, got {}",
            other.type_name()
        ))),
    }
}

// ── agent ────────────────────────────────────────────────────────────────────

/// Handle `(agent init-val & opts)`.
fn handle_agent_call(arg_forms: &[Form], env: &mut Env) -> EvalResult {
    if arg_forms.is_empty() {
        return Err(EvalError::Arity {
            name: "agent".into(),
            expected: "1+".into(),
            got: 0,
        });
    }
    // Under no-gc: agent initial value must live in the StaticArena since the
    // Agent container outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let init = eval(&arg_forms[0], env)?;

    let (tx, rx) = std::sync::mpsc::sync_channel::<AgentMsg>(1024);
    let state_arc = std::sync::Arc::new(std::sync::Mutex::new(init));
    let error_arc: std::sync::Arc<std::sync::Mutex<Option<Value>>> =
        std::sync::Arc::new(std::sync::Mutex::new(None));
    let worker_state = state_arc.clone();
    let worker_error = error_arc.clone();
    std::thread::spawn(move || {
        while let Ok(msg) = rx.recv() {
            match msg {
                AgentMsg::Update(f) => {
                    let cur = worker_state.lock().unwrap().clone();
                    match f(cur) {
                        Ok(next) => *worker_state.lock().unwrap() = next,
                        Err(e) => *worker_error.lock().unwrap() = Some(e),
                    }
                }
                AgentMsg::Shutdown => break,
            }
        }
    });
    Ok(Value::Agent(GcPtr::new(Agent {
        state: state_arc,
        error: error_arc,
        sender: std::sync::Mutex::new(tx),
        watches: std::sync::Mutex::new(Vec::new()),
    })))
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
                let new_val = cljrs_env::apply::apply_value(&f, call_args, &mut call_env)
                    .map_err(eval_error_to_value)?;
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
    // Under no-gc: atom initial value must live in the StaticArena since the
    // Atom container outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
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
        let result = cljrs_env::apply::apply_value(vf, vec![initial.clone()], env)?;
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
    // Under no-gc: the new value written into the atom must live in the
    // StaticArena since the atom outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
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
        let result = cljrs_env::apply::apply_value(&vf, vec![new_val.clone()], env)?;
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
    // Under no-gc: the value written into the atom must live in the StaticArena
    // since the atom outlives all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = cljrs_env::apply::apply_value(&f, args, env)?;
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
                frame.insert(cljrs_env::dynamics::var_key_of(vp), v.clone());
            }
            // non-Var keys silently ignored
        });
    } else {
        return Err(EvalError::Runtime(
            "with-bindings*: first arg must be a map".into(),
        ));
    }

    let _guard = cljrs_env::dynamics::push_frame(frame);
    cljrs_env::apply::apply_value(&func_val, vec![], env)
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
    // Under no-gc: the new Var root value must live in the StaticArena since
    // Vars outlive all scratch regions.
    #[cfg(feature = "no-gc")]
    let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
    let new_val = cljrs_env::apply::apply_value(&f, call_args, env)?;
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
    let new_meta = cljrs_env::apply::apply_value(&f, call_args, env)?;
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
    let new_meta = cljrs_env::apply::apply_value(&f, call_args, env)?;
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
        let val = cljrs_env::dynamics::deref_var(&var);
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
        // Under no-gc: interned Var values live in the StaticArena since they
        // are namespace-scoped and outlive all scratch regions.
        #[cfg(feature = "no-gc")]
        let _static_ctx = cljrs_gc::alloc_ctx::StaticCtxGuard::new();
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
    let frames = cljrs_env::dynamics::capture_current();
    let mut merged = std::collections::HashMap::new();
    for frame in &frames {
        merged.extend(frame.iter().map(|(k, v)| (*k, v.clone())));
    }
    Ok(Value::BoundFn(cljrs_gc::GcPtr::new(cljrs_value::BoundFn {
        wrapped: f,
        captured_bindings: merged,
    })))
}
