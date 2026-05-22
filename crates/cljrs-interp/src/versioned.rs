//! Versioned symbol and namespace resolution.
//!
//! When a symbol carries an `@<commit>` suffix — or when evaluation is
//! happening inside a versioned namespace body — this module fetches the
//! historical source from git and evaluates the relevant definition in a
//! snapshot environment.

use std::path::Path;
use std::sync::Arc;

use cljrs_env::env::Env;
use cljrs_env::error::{EvalError, EvalResult};
use cljrs_reader::Form;
use cljrs_reader::form::FormKind;
use cljrs_value::{Symbol, Value};

// ── Public entry points ───────────────────────────────────────────────────────

/// Resolve `sym` (which may or may not carry a version) at `commit` within
/// `env`.
///
/// Resolution order:
/// 1. Version cache hit → return immediately.
/// 2. Determine the owning namespace for the symbol.
/// 3. Fetch the source file at `commit` via `cljrs-vcs`.
/// 4. Scan forms for `(def name …)` / `(defn name …)` / `(defmacro name …)`.
/// 5. Evaluate the found form in a snapshot env with `versioned_eval_commit`
///    set to `commit`, so that same-namespace helpers also resolve historically.
/// 6. Cache the result.
pub fn resolve_versioned_symbol(sym: &Symbol, commit: &str, env: &mut Env) -> EvalResult {
    // Determine the owning namespace.
    let ns_name: Arc<str> = match &sym.namespace {
        Some(ns_part) => env
            .globals
            .resolve_alias(&env.current_ns, ns_part)
            .unwrap_or_else(|| Arc::clone(ns_part)),
        None => Arc::clone(&env.current_ns),
    };

    let name = sym.name.as_ref();

    // Cache check — covers both Clojure-defined and native-versioned bindings.
    if let Some(cached) = env.globals.get_cached_versioned(&ns_name, name, commit) {
        return Ok(cached);
    }

    // Explicit native version registry: populated by Registry::define_versioned.
    // Checked before git-source lookup so that native-only namespaces (with no
    // Clojure source file) can still resolve versioned symbols.
    if let Some(native_val) = env.globals.get_native_versioned(&ns_name, name, commit) {
        env.globals
            .cache_versioned(&ns_name, name, commit, native_val.clone());
        return Ok(native_val);
    }

    // Get git context for the owning namespace.  If the namespace hasn't been
    // loaded yet, try to load it so the context gets populated.
    // For pure-Rust namespaces with no Clojure source this may fail; in that
    // case fall back to the HEAD native binding (if any).
    let git_ctx = git_context_for_ns(&ns_name, env);
    let (source_file, repo_root) = match git_ctx {
        Ok(ctx) => ctx,
        Err(_) => return native_head_fallback(&ns_name, name, commit, env),
    };

    // Verify commit signature before loading any historical code.
    env.globals.check_commit_signature(&repo_root, commit)?;

    // Compute repo-relative path.
    let abs_file = Path::new(source_file.as_ref());
    let repo_path = Path::new(repo_root.as_ref());
    let rel_file = abs_file.strip_prefix(repo_path).map_err(|_| {
        EvalError::Runtime(format!(
            "Cannot compute relative path for {source_file} within {repo_root}"
        ))
    })?;
    let rel_file_str = rel_file.to_string_lossy();

    // Fetch source at commit.
    let src = cljrs_vcs::get_file_at_commit(repo_path, &rel_file_str, commit)
        .map_err(|e| EvalError::Runtime(format!("{e}")))?;

    // Parse.
    let file_label = format!("<{ns_name}@{commit}>");
    let mut parser = cljrs_reader::Parser::new(src, file_label);
    let forms = parser.parse_all().map_err(EvalError::Read)?;

    // Find the definition of `name`.  If it's absent, the var may be backed
    // by a native Rust function rather than Clojure source; try the HEAD
    // fallback before giving up.
    let Some(def_form) = find_def_form(&forms, name) else {
        return native_head_fallback(&ns_name, name, commit, env);
    };

    // Evaluate in a snapshot env: same namespace, versioned commit.
    let val = eval_in_snapshot(def_form, &ns_name, commit, env)?;

    // Cache and return.
    env.globals
        .cache_versioned(&ns_name, name, commit, val.clone());
    Ok(val)
}

/// Fall back to the HEAD value for a native Rust function when no explicit
/// versioned binding has been registered and no Clojure source definition
/// exists for the symbol at the requested commit.
///
/// Returns the HEAD `NativeFunction` value (caching it under the requested
/// commit so later lookups are fast), or a descriptive `EvalError` otherwise.
fn native_head_fallback(
    ns_name: &Arc<str>,
    name: &str,
    commit: &str,
    env: &mut Env,
) -> EvalResult {
    match env.globals.lookup_in_ns(ns_name, name) {
        Some(val) if matches!(val, Value::NativeFunction(_)) => {
            // The var is a native function with no historical source definition.
            // Return the current (HEAD) implementation and cache it for this
            // commit so repeated versioned lookups are O(1).
            env.globals
                .cache_versioned(ns_name, name, commit, val.clone());
            Ok(val)
        }
        Some(_) => Err(EvalError::Runtime(format!(
            "Cannot find definition of `{name}` in `{ns_name}@{commit}`"
        ))),
        None => Err(EvalError::UnboundSymbol(format!("{ns_name}/{name}"))),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Return `(source_file, git_repo_root)` for `ns_name`, triggering a load of
/// the namespace if it has not yet been loaded (so the loader can populate the
/// git context fields).
fn git_context_for_ns(ns_name: &Arc<str>, env: &mut Env) -> EvalResult<(Arc<str>, Arc<str>)> {
    // Fast path: already populated.
    if let Some(ctx) = env.globals.get_ns_git_context(ns_name) {
        return Ok(ctx);
    }

    // Try loading the namespace (idempotent if already loaded).
    let spec = cljrs_env::env::RequireSpec {
        ns: Arc::clone(ns_name),
        version: None,
        alias: None,
        refer: cljrs_env::env::RequireRefer::None,
    };
    cljrs_env::loader::load_ns(Arc::clone(&env.globals), &spec, &env.current_ns)?;

    // Try again after load.
    env.globals.get_ns_git_context(ns_name).ok_or_else(|| {
        EvalError::Runtime(format!(
            "Namespace `{ns_name}` has no git context (built-in or not in a git repo); \
             cannot resolve versioned symbols from it"
        ))
    })
}

/// Evaluate `form` inside a snapshot environment: the current namespace is set
/// to `ns_name` and `versioned_eval_commit` is set to `commit`.
fn eval_in_snapshot(form: &Form, ns_name: &str, commit: &str, env: &mut Env) -> EvalResult {
    let mut snap = Env::new_versioned(Arc::clone(&env.globals), ns_name, commit);
    (env.globals.eval_fn)(form, &mut snap)
}

/// Scan `forms` for the first top-level `def`-like form that binds `name`.
///
/// Recognises: `(def name …)`, `(defn name …)`, `(defmacro name …)`,
/// `(defn- name …)`, `(def- name …)`.
fn find_def_form<'a>(forms: &'a [Form], name: &str) -> Option<&'a Form> {
    for form in forms {
        if let FormKind::List(items) = &form.kind {
            if items.len() < 2 {
                continue;
            }
            let head = match &items[0].kind {
                FormKind::Symbol(s) => s.as_str(),
                _ => continue,
            };
            let is_def_like = matches!(
                head,
                "def" | "defn" | "defn-" | "def-" | "defmacro" | "defmulti"
            );
            if !is_def_like {
                continue;
            }
            // The name is the second element, possibly wrapped in metadata.
            let name_form = &items[1];
            let actual_name = def_form_name(name_form);
            if actual_name.as_deref() == Some(name) {
                return Some(form);
            }
        }
    }
    None
}

/// Extract the symbol name from the second element of a `def`-like form.
/// Handles `^metadata name` wrapping.
fn def_form_name(form: &Form) -> Option<String> {
    match &form.kind {
        FormKind::Symbol(s) => Some(s.clone()),
        // `^{:doc "…"} name`  or  `^:private name`
        FormKind::Meta(_, inner) => def_form_name(inner),
        _ => None,
    }
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

    // ── Explicit versioned native registry ────────────────────────────────────

    /// define_versioned + versioned symbol lookup returns the registered value.
    #[test]
    fn explicit_versioned_native_roundtrip() {
        let (globals, mut env) = make_env("mylib");
        let commit = fake_commit();

        // Register an explicit versioned binding (simulates Registry::define_versioned).
        let val = Value::NativeFunction(GcPtr::new(const_native(42)));
        globals.register_native_versioned("mylib", "my-fn", commit, val);

        // Resolve the versioned symbol.
        let sym = cljrs_value::Symbol {
            namespace: Some(Arc::from("mylib")),
            name: Arc::from("my-fn"),
            version: Some(Arc::from(commit)),
        };
        let result = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect("should resolve");

        assert!(
            matches!(result, Value::NativeFunction(_)),
            "expected NativeFunction, got {result:?}"
        );
    }

    /// A second lookup hits the version_cache, not the native_version_registry again.
    #[test]
    fn explicit_versioned_native_is_cached() {
        let (globals, mut env) = make_env("mylib");
        let commit = fake_commit();

        let val = Value::NativeFunction(GcPtr::new(const_native(7)));
        globals.register_native_versioned("mylib", "cached-fn", commit, val);

        let sym = cljrs_value::Symbol {
            namespace: Some(Arc::from("mylib")),
            name: Arc::from("cached-fn"),
            version: Some(Arc::from(commit)),
        };
        super::resolve_versioned_symbol(&sym, commit, &mut env).unwrap();

        // Confirm the result is now in version_cache.
        let cached = globals.get_cached_versioned("mylib", "cached-fn", commit);
        assert!(cached.is_some(), "result should be in version_cache after first lookup");
    }

    // ── HEAD fallback ─────────────────────────────────────────────────────────

    /// When no explicit versioned entry exists but the var is a NativeFunction at
    /// HEAD, resolve_versioned_symbol returns the HEAD value rather than erroring.
    #[test]
    fn head_fallback_for_unregistered_commit() {
        let (globals, mut env) = make_env("mylib");
        let commit = "deadbeef01234";

        // Register the function at HEAD (no versioned entry for this commit).
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
        // This should succeed via HEAD fallback even though no Clojure source
        // exists for mylib at this commit.
        let result = super::resolve_versioned_symbol(&sym, commit, &mut env)
            .expect("HEAD fallback should succeed");

        assert!(matches!(result, Value::NativeFunction(_)));
    }

    /// When the symbol doesn't exist at all (neither versioned nor HEAD), we get
    /// an UnboundSymbol error — not a confusing "Cannot find definition" message.
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
