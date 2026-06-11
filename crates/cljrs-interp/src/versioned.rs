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

    // ── Native provenance (verified HEAD binding) ─────────────────────────────

    fn versioned_sym(ns: &str, name: &str, commit: &str) -> cljrs_value::Symbol {
        cljrs_value::Symbol {
            namespace: Some(Arc::from(ns)),
            name: Arc::from(name),
            version: Some(Arc::from(commit)),
        }
    }

    /// Matching provenance (either side may be abbreviated) resolves silently
    /// — no entry lands in the warned set.
    #[test]
    fn matching_provenance_is_silent() {
        let (globals, mut env) = make_env("provlib");
        let full_commit = "deadbeef0123456789abcdef0123456789abcdef";

        globals.intern(
            "provlib",
            Arc::from("f"),
            Value::NativeFunction(GcPtr::new(const_native(1))),
        );
        globals.set_native_provenance("provlib", full_commit);

        // Pin with an abbreviated prefix of the recorded commit.
        let sym = versioned_sym("provlib", "f", "deadbeef012");
        super::resolve_versioned_symbol(&sym, "deadbeef012", &mut env).expect("should resolve");

        assert!(
            globals.provenance_warned.lock().unwrap().is_empty(),
            "matching provenance must not warn"
        );
    }

    /// Mismatching provenance still resolves (HEAD binding) but records a
    /// once-per-pin warning.
    #[test]
    fn mismatched_provenance_warns_once() {
        let (globals, mut env) = make_env("provlib2");
        globals.intern(
            "provlib2",
            Arc::from("f"),
            Value::NativeFunction(GcPtr::new(const_native(2))),
        );
        globals.set_native_provenance("provlib2", "1111111111111111");

        let commit = "2222222222222222";
        let sym = versioned_sym("provlib2", "f", commit);
        let val = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect("mismatch still resolves to HEAD by default");
        assert!(matches!(val, Value::NativeFunction(_)));

        let warned = globals.provenance_warned.lock().unwrap();
        assert_eq!(warned.len(), 1, "exactly one warning per pin");
        assert!(warned.contains(&Arc::<str>::from("provlib2@2222222222222222")));
    }

    /// Under --enforce-native-versions a provenance mismatch is an error.
    #[test]
    fn enforce_native_versions_makes_mismatch_an_error() {
        let (globals, mut env) = make_env("provlib3");
        globals.intern(
            "provlib3",
            Arc::from("f"),
            Value::NativeFunction(GcPtr::new(const_native(3))),
        );
        globals.set_native_provenance("provlib3", "1111111111111111");
        globals.set_enforce_native_versions(true);

        let commit = "2222222222222222";
        let sym = versioned_sym("provlib3", "f", commit);
        let err = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect_err("strict mode must reject a provenance mismatch");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("provlib3") && msg.contains("1111111111111111"),
            "error should describe the mismatch: {msg}"
        );
    }
}
