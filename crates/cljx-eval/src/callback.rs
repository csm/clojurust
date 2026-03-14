//! Thread-local eval context for Rust→Clojure callbacks.
//!
//! When a native (Rust) builtin needs to call a Clojure function — for example,
//! a comparator passed to `sort-by` — it can use [`invoke`] to do so.  The eval
//! context is pushed automatically before every native function call and popped
//! afterward, so `invoke` is always available inside builtins.

use std::cell::RefCell;
use std::sync::Arc;

use cljx_value::{Value, ValueError, ValueResult};

use crate::apply::apply_value;
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
pub fn invoke(f: &Value, args: Vec<Value>) -> ValueResult<Value> {
    let (globals, ns) = EVAL_CONTEXT.with(|stack| {
        let s = stack.borrow();
        let ec = s
            .last()
            .ok_or_else(|| ValueError::Other("invoke called outside eval context".into()))?;
        Ok((ec.globals.clone(), ec.current_ns.clone()))
    })?;
    let mut env = Env::new(globals, &ns);
    apply_value(f, args, &mut env).map_err(|e| ValueError::Other(format!("{e}")))
}
