//! Versioned symbol resolution — tree-walker entry point.
//!
//! The implementation lives in `cljrs_env::versioned` so that every execution
//! tier (tree-walker, IR interpreter, JIT/AOT runtime bridges) shares one
//! resolver.  Resolving `ns/name@commit` loads the whole versioned namespace
//! `"ns@commit"` (from embedded source or git history) and then performs a
//! plain lookup in it; native (Rust-backed) functions with no Clojure source
//! fall back to the HEAD implementation.

use cljrs_env::env::Env;
use cljrs_env::error::EvalResult;
use cljrs_value::Symbol;

/// Resolve `sym` (which may or may not carry a version) at `commit` within
/// `env`.
pub fn resolve_versioned_symbol(sym: &Symbol, commit: &str, env: &mut Env) -> EvalResult {
    cljrs_env::versioned::resolve_versioned_value(
        &env.globals,
        &env.current_ns,
        sym.namespace.as_deref(),
        &sym.name,
        commit,
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cljrs_env::env::{Env, GlobalEnv};
    use cljrs_gc::GcPtr;
    use cljrs_value::{NativeFn, Value};

    fn make_env(ns: &str) -> (Arc<GlobalEnv>, Env) {
        let globals = crate::standard_env_minimal(None, None, None);
        globals.get_or_create_ns(ns);
        let env = Env::new(globals.clone(), ns);
        (globals, env)
    }

    fn fake_commit() -> &'static str {
        "abc1234def56"
    }

    /// Build a trivial NativeFn that returns a fixed Long value.
    fn const_native(tag: i64) -> NativeFn {
        NativeFn {
            name: Arc::from("test-fn"),
            arity: cljrs_value::Arity::Fixed(0),
            func: Arc::new(move |_args| Ok(Value::Long(tag))),
        }
    }

    // ── HEAD fallback ─────────────────────────────────────────────────────────

    /// When a versioned symbol is requested for a native function, the HEAD
    /// implementation is returned (no Clojure source exists at the commit).
    #[test]
    fn head_fallback_for_native_function() {
        let (globals, mut env) = make_env("mylib");
        let commit = "deadbeef01234";

        // Register the function at HEAD.  No Clojure source exists for mylib
        // at any commit, so the git-context path will fail.
        let nf = const_native(99);
        globals.intern(
            "mylib",
            Arc::from("stable-fn"),
            Value::NativeFunction(GcPtr::new(nf)),
        );

        let sym = cljrs_value::Symbol {
            namespace: Some(Arc::from("mylib")),
            name: Arc::from("stable-fn"),
            version: Some(Arc::from(commit)),
        };
        let result = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect("HEAD fallback should succeed");

        assert!(matches!(result, Value::NativeFunction(_)));
    }

    /// When the symbol doesn't exist anywhere, we get UnboundSymbol — not the
    /// confusing "Cannot find definition" message that the bare git-source
    /// path would produce.
    #[test]
    fn missing_symbol_gives_unbound_error() {
        let (_globals, mut env) = make_env("mylib");
        let commit = fake_commit();

        let sym = cljrs_value::Symbol {
            namespace: Some(Arc::from("mylib")),
            name: Arc::from("does-not-exist"),
            version: Some(Arc::from(commit)),
        };
        let err = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect_err("should error for unknown symbol");

        assert!(
            matches!(err, cljrs_env::error::EvalError::UnboundSymbol(_)),
            "expected UnboundSymbol, got {err:?}"
        );
    }
}
