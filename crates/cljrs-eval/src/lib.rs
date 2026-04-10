//! Tree-walking interpreter for clojurust.
//!
//! Phase 4 implements:
//! - Lexical environments and namespace-level vars
//! - All Clojure special forms (`def`, `let*`, `fn*`, `if`, `do`, `quote`, …)
//! - Macro expansion pipeline
//! - Tail-call optimization via `recur`
//! - Sequential destructuring in `let*`/`fn*`/`loop*`

// EvalError::Thrown wraps a full Value; boxing would require pervasive changes.
#![allow(clippy::result_large_err)]
// Namespace/GlobalEnv use Mutex<HashMap<Arc<str>, GcPtr<Var>>> — intentionally verbose for clarity.
#![allow(clippy::type_complexity)]

pub mod apply;
mod array_list;
mod bitops;
pub mod builtins;
pub mod callback;
pub mod destructure;
pub mod dynamics;
pub mod env;
pub mod error;
pub mod eval;
pub mod ir_interp;
pub mod loader;
pub mod macros;
mod new;
mod regex;
pub mod special;
pub mod syntax_quote;
pub mod taps;
mod transients;
mod util;
mod virtualize;

pub use callback::invoke;
pub use env::{Env, GlobalEnv};
pub use error::{EvalError, EvalResult};
pub use eval::eval;
pub use loader::load_ns;

use std::cell::RefCell;
use std::sync::Arc;

// ── Thread-local Env root registry ──────────────────────────────────────────
//
// When the interpreter enters a function call, the caller's Env stays on the
// Rust stack but the callee creates a fresh Env.  If GC triggers inside the
// callee, only the callee's Env is passed to `gc_safepoint`.  To keep the
// caller's local bindings alive we maintain a thread-local stack of pointers
// to all active Envs on this thread's call stack.
//
// SAFETY: the raw pointers are valid during STW collection because:
// - The collecting thread's own Envs are in earlier (still-live) stack frames.
// - Other threads are parked at safepoints; their stacks (and Envs) are frozen.

thread_local! {
    static ENV_ROOTS: RefCell<Vec<*const Env>> = const { RefCell::new(Vec::new()) };
    /// Shadow stack of Value pointers on the Rust call stack that need to
    /// survive GC.  Each entry is a `(ptr, count)` pair pointing to a
    /// contiguous slice of Values (e.g., a Vec's backing storage or a single
    /// Value on the stack).
    static VALUE_ROOTS: RefCell<Vec<(*const cljrs_value::Value, usize)>> =
        const { RefCell::new(Vec::new()) };
}

/// RAII guard that pops the Env pointer on drop.
pub struct EnvRootGuard;

impl Drop for EnvRootGuard {
    fn drop(&mut self) {
        ENV_ROOTS.with(|roots| {
            roots.borrow_mut().pop();
        });
    }
}

/// RAII guard that pops one entry from the value shadow stack on drop.
pub struct ValueRootGuard {
    pushed: bool,
}

impl Drop for ValueRootGuard {
    fn drop(&mut self) {
        if self.pushed {
            VALUE_ROOTS.with(|roots| {
                roots.borrow_mut().pop();
            });
        }
    }
}

/// Register an Env as a GC root for the duration of its use.
/// Returns a guard that unregisters on drop.
pub fn push_env_root(env: &Env) -> EnvRootGuard {
    ENV_ROOTS.with(|roots| {
        roots.borrow_mut().push(env as *const Env);
    });
    EnvRootGuard
}

/// Register a single Value as a GC root.
pub fn root_value(val: &cljrs_value::Value) -> ValueRootGuard {
    VALUE_ROOTS.with(|roots| {
        roots
            .borrow_mut()
            .push((val as *const cljrs_value::Value, 1));
    });
    ValueRootGuard { pushed: true }
}

/// Register a slice of Values as GC roots (e.g., a Vec<Value>).
pub fn root_values(vals: &[cljrs_value::Value]) -> ValueRootGuard {
    if vals.is_empty() {
        return ValueRootGuard { pushed: false };
    }
    VALUE_ROOTS.with(|roots| {
        roots.borrow_mut().push((vals.as_ptr(), vals.len()));
    });
    ValueRootGuard { pushed: true }
}

/// Interpreter-level GC safepoint.
///
/// Called at function entry, loop heads, and application boundaries.
/// If a GC collection is in progress, blocks until it completes.
/// If a GC has been requested (memory pressure), this thread becomes
/// the collector: initiates STW, traces roots, collects, then resumes.
pub fn gc_safepoint(env: &Env) {
    // Fast path: no GC activity at all.
    if !cljrs_gc::gc_requested() && !cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        return;
    }

    // If a GC is already in progress (another thread is collecting), just park.
    if cljrs_gc::CONFIG_CANCELLATION.in_progress() {
        cljrs_gc::safepoint();
        return;
    }

    // A GC was requested (memory pressure). Try to become the collector.
    if !cljrs_gc::take_gc_request() {
        // Another thread took the request; if collection started, park.
        cljrs_gc::safepoint();
        return;
    }

    // We won the request. Initiate STW collection.
    let Some(_stw_guard) = cljrs_gc::begin_stw() else {
        // Race: another thread started collecting between our take and begin.
        cljrs_gc::safepoint();
        return;
    };

    // All other threads are now parked. Collect with registered roots
    // plus ALL of this thread's active environments and dynamic bindings.
    cljrs_gc::HEAP.collect(|visitor| {
        // Trace globally registered roots (GlobalEnv, etc.)
        cljrs_gc::HEAP.trace_registered_roots(visitor);
        // Trace the current (innermost) env
        trace_env_roots(env, visitor);
        // Trace all caller Envs registered on this thread's stack
        trace_thread_env_roots(visitor);
        // Trace values on the Rust call stack (shadow stack)
        trace_value_roots(visitor);
        // Trace dynamic variable bindings on this thread
        dynamics::trace_current(visitor);
        // Trace the global tap system (functions and queued values)
        taps::trace_roots(visitor);
        // Trace in-flight allocations from this thread's alloc root frames
        cljrs_gc::trace_thread_alloc_roots(visitor);
    });
    // _stw_guard drop clears in_progress, waking parked threads.
}

/// Trace all GcPtr values reachable from an Env's local frames.
fn trace_env_roots(env: &Env, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    // Trace local frame bindings
    for frame in &env.frames {
        for (_name, val) in &frame.bindings {
            val.trace(visitor);
        }
    }
    // Trace the globals (namespaces, vars) — these are also registered
    // as root tracers, but it's safe to trace twice (idempotent marking).
    trace_globals(&env.globals, visitor);
}

/// Trace all Values registered in the thread-local value shadow stack.
fn trace_value_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    VALUE_ROOTS.with(|roots| {
        for &(ptr, count) in roots.borrow().iter() {
            // SAFETY: pointers are valid — they point to Values on this thread's
            // still-live stack frames or heap-allocated Vecs whose owners are
            // on still-live stack frames.
            let slice = unsafe { std::slice::from_raw_parts(ptr, count) };
            for val in slice {
                val.trace(visitor);
            }
        }
    });
}

/// Trace all Envs registered in the thread-local root stack.
fn trace_thread_env_roots(visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::Trace;
    ENV_ROOTS.with(|roots| {
        for env_ptr in roots.borrow().iter() {
            // SAFETY: pointers are valid — they point to Envs on this thread's
            // still-live stack frames (we are the collector, so our stack is active).
            let env = unsafe { &**env_ptr };
            for frame in &env.frames {
                for (_name, val) in &frame.bindings {
                    val.trace(visitor);
                }
            }
        }
    });
}

/// Trace all namespaces and their contents.
fn trace_globals(globals: &GlobalEnv, visitor: &mut cljrs_gc::MarkVisitor) {
    use cljrs_gc::GcVisitor as _;
    let namespaces = globals.namespaces.read().unwrap();
    for (_name, ns_ptr) in namespaces.iter() {
        visitor.visit(ns_ptr);
    }
}

/// Create a minimal `GlobalEnv` with `clojure.core` builtins and bootstrap
/// HOFs, but without any stdlib namespaces pre-loaded.
///
/// Used by `cljrs-stdlib` as a foundation; also useful for lightweight tests
/// that don't need stdlib.  Call [`standard_env`] for a batteries-included
/// environment suitable for eval-crate tests.
pub fn standard_env_minimal() -> Arc<GlobalEnv> {
    let globals = GlobalEnv::new();

    // Register all native builtins in clojure.core.
    builtins::register_all(&globals, "clojure.core");

    // Set up user namespace referring clojure.core.
    globals.get_or_create_ns("user");
    globals.refer_all("user", "clojure.core");

    // Eval bootstrap Clojure source in clojure.core.
    {
        let mut env = Env::new(globals.clone(), "clojure.core");
        let src = builtins::BOOTSTRAP_SOURCE;
        let mut parser = cljrs_reader::Parser::new(src.to_string(), "<bootstrap>".to_string());
        match parser.parse_all() {
            Ok(forms) => {
                for form in forms {
                    let _alloc_frame = cljrs_gc::push_alloc_frame();
                    if let Err(e) = eval::eval(&form, &mut env) {
                        eprintln!("[bootstrap warning] {}: {:?}", form.span.start, e);
                    }
                }
            }
            Err(e) => eprintln!("[bootstrap parse error] {:?}", e),
        }
    }

    // Re-refer clojure.core after bootstrap defines HOFs.
    globals.refer_all("user", "clojure.core");

    // Mark clojure.core as loaded so (require 'clojure.core) is a no-op.
    globals.mark_loaded("clojure.core");

    // Set *ns* to the "user" namespace (the default REPL namespace).
    {
        let mut env = Env::new(globals.clone(), "user");
        special::sync_star_ns(&mut env);
    }

    globals
}

/// Create a `GlobalEnv` pre-populated with `clojure.core` built-ins,
/// bootstrap HOFs, and `clojure.test` (eagerly loaded so eval-crate tests
/// can use `(require '[clojure.test ...])` without a source path).
///
/// For the `cljrs` binary, prefer `cljrs_stdlib::standard_env()` which loads
/// `clojure.test` and other stdlib namespaces lazily via the registry.
pub fn standard_env() -> Arc<GlobalEnv> {
    let globals = standard_env_minimal();

    // Eagerly load clojure.test so eval-crate tests can `require` it.
    {
        let mut env = Env::new(globals.clone(), "clojure.core");
        let src = builtins::CLOJURE_TEST_SOURCE;
        let mut parser = cljrs_reader::Parser::new(src.to_string(), "<clojure.test>".to_string());
        match parser.parse_all() {
            Ok(forms) => {
                for form in forms {
                    let _alloc_frame = cljrs_gc::push_alloc_frame();
                    if let Err(e) = eval::eval(&form, &mut env) {
                        eprintln!("[clojure.test warning] {}: {:?}", form.span.start, e);
                    }
                }
            }
            Err(e) => eprintln!("[clojure.test parse error] {:?}", e),
        }
        globals.mark_loaded("clojure.test");
    }

    // Restore *ns* to "user" — loading clojure.test leaves it as "clojure.test".
    {
        let mut env = Env::new(globals.clone(), "user");
        special::sync_star_ns(&mut env);
    }

    globals
}

/// Create a `GlobalEnv` with built-ins, bootstrap HOFs, and configured source paths.
pub fn standard_env_with_paths(source_paths: Vec<std::path::PathBuf>) -> Arc<GlobalEnv> {
    let globals = standard_env();
    globals.set_source_paths(source_paths);
    globals
}
