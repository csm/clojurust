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
pub mod builtins;
pub mod destructure;
pub mod env;
pub mod error;
pub mod eval;
pub mod macros;
pub mod special;
pub mod syntax_quote;

pub use env::{Env, GlobalEnv};
pub use error::{EvalError, EvalResult};
pub use eval::eval;

use std::sync::Arc;

/// Create a `GlobalEnv` pre-populated with `clojure.core` built-ins and
/// the bootstrap HOF Clojure source, then set up a `user` namespace that
/// refers everything from `clojure.core`.
pub fn standard_env() -> Arc<GlobalEnv> {
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
        let mut parser = cljx_reader::Parser::new(src.to_string(), "<bootstrap>".to_string());
        match parser.parse_all() {
            Ok(forms) => {
                for form in forms {
                    if let Err(e) = eval::eval(&form, &mut env) {
                        // Non-fatal: log and continue.
                        eprintln!("[bootstrap warning] {}: {:?}", form.span.start, e);
                    }
                }
            }
            Err(e) => eprintln!("[bootstrap parse error] {:?}", e),
        }
    }

    // Re-refer clojure.core after bootstrap defines HOFs.
    globals.refer_all("user", "clojure.core");

    globals
}
