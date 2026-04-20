use std::sync::Arc;
use cljrs_builtins::builtins;
use cljrs_env::env::{Env, GlobalEnv};
use cljrs_env::error::EvalResult;
use cljrs_reader::Form;
use cljrs_value::{CljxFn, Value};

pub mod eval;
pub mod apply;
pub mod special;
pub mod macros;
pub mod syntax_quote;
pub mod destructure;
mod virtualize;
mod arity;

/// Create a minimal `GlobalEnv` with `clojure.core` builtins and bootstrap
/// HOFs, but without any stdlib namespaces pre-loaded.
///
/// Used by `cljrs-stdlib` as a foundation; also useful for lightweight tests
/// that don't need stdlib.  Call [`standard_env`] for a batteries-included
/// environment suitable for eval-crate tests.
pub fn standard_env_minimal(
    eval_fn: Option<fn(&Form, &mut Env) -> EvalResult>,
    call_cljrs_fn: Option<fn(&CljxFn, &[Value], &mut Env) -> EvalResult>,
    on_fn_defined: Option<fn(&CljxFn, &mut Env)>,
) -> Arc<GlobalEnv> {
    let globals = GlobalEnv::new(
        eval_fn.unwrap_or(eval::eval),
        call_cljrs_fn.unwrap_or(apply::call_cljrs_fn),
        on_fn_defined,
    );

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
pub fn standard_env(
    eval_fn: Option<fn(&Form, &mut Env) -> EvalResult>,
    call_cljrs_fn: Option<fn(&CljxFn, &[Value], &mut Env) -> EvalResult>,
    on_fn_defined: Option<fn(&CljxFn, &mut Env)>,
) -> Arc<GlobalEnv> {
    let globals = standard_env_minimal(eval_fn, call_cljrs_fn, on_fn_defined);

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

    // Compiler namespaces are NOT loaded here — the Clojure compiler's deep
    // recursion can overflow the default 8MB test thread stack.  Instead,
    // callers that need IR lowering should call `ensure_compiler_loaded()`
    // on a thread with sufficient stack (see cljrs-stdlib / cljrs binary).

    // Restore *ns* to "user" — loading above may change it.
    {
        let mut env = Env::new(globals.clone(), "user");
        special::sync_star_ns(&mut env);
    }

    globals
}

/// Create a `GlobalEnv` with built-ins, bootstrap HOFs, and configured source paths.
pub fn standard_env_with_paths(
    eval_fn: Option<fn(&Form, &mut Env) -> EvalResult>,
    call_cljrs_fn: Option<fn(&CljxFn, &[Value], &mut Env) -> EvalResult>,
    on_fn_defined: Option<fn(&CljxFn, &mut Env)>,
    source_paths: Vec<std::path::PathBuf>
) -> Arc<GlobalEnv> {
    let globals = standard_env(eval_fn, call_cljrs_fn, on_fn_defined);
    globals.set_source_paths(source_paths);
    globals
}
