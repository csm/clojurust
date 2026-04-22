//! Thread-local eval context for Rust→Clojure callbacks.
//!
//! When a native (Rust) builtin needs to call a Clojure function — for example,
//! a comparator passed to `sort-by` — it can use [`invoke`] to do so.  The eval
//! context is pushed automatically before every native function call and popped
//! afterward, so `invoke` is always available inside builtins.

use std::cell::RefCell;
use std::sync::Arc;

use cljrs_value::{Value, ValueError, ValueResult};

use crate::env::{Env, GlobalEnv};

// ── Thread-local context stack ───────────────────────────────────────────────

struct EvalContext {
    globals: Arc<GlobalEnv>,
    current_ns: Arc<str>,
}

thread_local! {
    static EVAL_CONTEXT: RefCell<Vec<EvalContext>> = const { RefCell::new(Vec::new()) };
}

/// Push the current eval context before calling a native function.
pub fn push_eval_context(env: &Env) {
    EVAL_CONTEXT.with(|stack| {
        stack.borrow_mut().push(EvalContext {
            globals: env.globals.clone(),
            current_ns: env.current_ns.clone(),
        });
    });
}

/// Pop the eval context after a native function returns.
pub fn pop_eval_context() {
    EVAL_CONTEXT.with(|stack| {
        stack.borrow_mut().pop();
    });
}

/// Capture the current eval context so it can be installed on another thread.
///
/// Returns `None` if there is no active context.
pub fn capture_eval_context() -> Option<(Arc<GlobalEnv>, Arc<str>)> {
    EVAL_CONTEXT.with(|stack| {
        let s = stack.borrow();
        let ec = s.last()?;
        Some((ec.globals.clone(), ec.current_ns.clone()))
    })
}

/// Install a previously captured eval context on the current thread.
///
/// Call this at the start of a spawned thread so that `invoke` works.
pub fn install_eval_context(globals: Arc<GlobalEnv>, ns: Arc<str>) {
    EVAL_CONTEXT.with(|stack| {
        stack.borrow_mut().push(EvalContext {
            globals,
            current_ns: ns,
        });
    });
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Call a Clojure-callable `Value` with the given arguments.
///
/// This can be called from any Rust code running inside an active evaluation
/// (i.e., inside a builtin function, a `Thunk::force`, etc.).
///
/// # Errors
///
/// Returns `Err` if called outside an eval context, or if the callee raises
/// an error.
/// Execute a closure with access to a temporary `Env` constructed from the
/// current eval context.
///
/// This is used by the IR interpreter for calling builtins that need an `Env`
/// (e.g., for nested `apply_value` calls from inside a `NativeFunction`
/// closure).
///
/// # Errors
///
/// Returns `Err` if called outside an eval context.
pub fn with_eval_context<F, R>(f: F) -> Result<R, crate::error::EvalError>
where
    F: FnOnce(&mut Env) -> Result<R, crate::error::EvalError>,
{
    let (globals, ns) = EVAL_CONTEXT.with(|stack| {
        let s = stack.borrow();
        let ec = s.last().ok_or_else(|| {
            crate::error::EvalError::Runtime(
                "with_eval_context called outside eval context".to_string(),
            )
        })?;
        Ok::<_, crate::error::EvalError>((ec.globals.clone(), ec.current_ns.clone()))
    })?;
    let mut env = Env::new(globals, &ns);
    f(&mut env)
}

pub fn invoke(f: &Value, args: Vec<Value>) -> ValueResult<Value> {
    let (globals, ns) = EVAL_CONTEXT.with(|stack| {
        let s = stack.borrow();
        let ec = s
            .last()
            .ok_or_else(|| ValueError::Other("invoke called outside eval context".into()))?;
        Ok((ec.globals.clone(), ec.current_ns.clone()))
    })?;
    let mut env = Env::new(globals, &ns);
    // Fast path for Clojure functions: call directly through the GlobalEnv
    // function pointer, bypassing the large apply_value stack frame.
    // Unwrap metadata so a WithMeta-wrapped fn is callable.
    let f = f.unwrap_meta();
    let result = if let Value::Fn(cljx_fn) = f {
        env.call_cljrs_fn(cljx_fn.get(), &args)
    } else {
        crate::apply::apply_value(f, args, &mut env)
    };
    result.map_err(|e| match e {
        crate::error::EvalError::Thrown(v) => ValueError::Thrown(v),
        other => ValueError::Other(format!("{other}")),
    })
}
