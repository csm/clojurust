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
mod bitops;
pub mod builtins;
pub mod callback;
pub mod destructure;
pub mod dynamics;
pub mod env;
pub mod error;
pub mod eval;
pub mod loader;
pub mod macros;
pub mod special;
pub mod syntax_quote;
pub mod taps;
mod transients;
mod util;

pub use callback::invoke;
pub use env::{Env, GlobalEnv};
pub use error::{EvalError, EvalResult};
pub use eval::eval;
pub use loader::load_ns;

use std::sync::Arc;

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
