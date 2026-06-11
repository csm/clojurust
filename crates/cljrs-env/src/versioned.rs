//! Shared versioned-symbol/namespace resolution service.
//!
//! This is the single implementation used by every execution tier — the
//! tree-walking interpreter, the IR interpreter, JIT-compiled code (via the
//! `rt_load_global*` runtime bridges), and AOT binaries.
//!
//! ## Model
//!
//! Resolving `ns/name@commit` means: ensure the **versioned namespace**
//! `"ns@commit"` is loaded (lazily, from an embedded builtin source or from
//! git history), then perform a plain `lookup_in_ns("ns@commit", name)`.
//! Versioned namespaces are immutable once loaded, so a resolved value never
//! changes for the lifetime of the process.
//!
//! ## Native (Rust-backed) functions
//!
//! Native functions registered via `cljrs-interop::Registry` have no Clojure
//! source to fetch, and the binary is a single unit at runtime — we can't
//! "go back in time" to a previous compiled implementation.  The current
//! contract is: versioned lookups of a native symbol resolve to the HEAD
//! (current) implementation regardless of the requested commit (see
//! `native_head_fallback`).

use std::path::Path;
use std::sync::Arc;

use crate::env::{Env, GlobalEnv};
use crate::error::{EvalError, EvalResult};
use cljrs_value::Value;

/// Strip a trailing `@<commit>` suffix from a namespace name, returning the
/// base namespace.  `"my.lib@abc1234"` → `"my.lib"`; names without a valid
/// commit-hash suffix are returned unchanged.
pub fn base_ns_name(ns: &str) -> &str {
    cljrs_value::symbol::split_version(ns).0
}

/// Resolve the value of `name` pinned at `commit`.
///
/// - `defining_ns` — the namespace the reference appears in (used for alias
///   resolution and for unqualified symbols).
/// - `ns_part` — the symbol's namespace qualifier, if any (may be an alias).
///
/// Resolution order:
/// 1. Determine the base namespace (alias-resolved, `@`-suffix stripped).
/// 2. Fast path: the versioned namespace `"base@commit"` already has the
///    binding (it is loaded, or is being loaded right now by this thread).
/// 3. Version-cache hit (caches native HEAD fallbacks).
/// 4. Load the versioned namespace (embedded source or git), then look up.
/// 5. Fall back to the HEAD native binding when no Clojure source exists.
pub fn resolve_versioned_value(
    globals: &Arc<GlobalEnv>,
    defining_ns: &str,
    ns_part: Option<&str>,
    name: &str,
    commit: &str,
) -> EvalResult {
    // 1. Base namespace: resolve alias if qualified, else the defining ns.
    //    Strip any existing `@hash` so an explicit version always wins (e.g.
    //    a qualified self-reference inside an already-versioned namespace).
    let base_ns: Arc<str> = match ns_part {
        Some(p) => {
            let resolved = globals
                .resolve_alias(defining_ns, p)
                .unwrap_or_else(|| Arc::from(p));
            Arc::from(base_ns_name(&resolved))
        }
        None => Arc::from(base_ns_name(defining_ns)),
    };
    let versioned_ns: Arc<str> = Arc::from(format!("{base_ns}@{commit}"));

    // 2. Fast path: binding already present in the versioned namespace.
    //    This also serves lookups made *while* the namespace is being loaded
    //    on this thread (defs intern sequentially during load).
    if let Some(val) = globals.lookup_in_ns(&versioned_ns, name) {
        return Ok(val);
    }

    // 3. Cached native fallback from a previous resolution.
    if let Some(cached) = globals.get_cached_versioned(&base_ns, name, commit) {
        return Ok(cached);
    }

    // 4. Load the versioned namespace if we have any source for it.  When no
    //    source exists at all (pure-Rust namespace), go straight to the
    //    native fallback; load *failures* (bad commit, signature rejection,
    //    parse error) propagate rather than being masked by the fallback.
    if !globals.is_loaded(&versioned_ns) {
        if !versioned_source_available(globals, &base_ns, &versioned_ns) {
            return native_head_fallback(globals, &base_ns, name, commit);
        }
        ensure_versioned_ns_loaded(globals, &base_ns, commit)?;
    }

    if let Some(val) = globals.lookup_in_ns(&versioned_ns, name) {
        return Ok(val);
    }

    // 5. The historical source exists but does not define `name`: the var may
    //    be backed by a native Rust function rather than Clojure source.
    native_head_fallback(globals, &base_ns, name, commit)
}

/// True if source for `base_ns` at some commit could be obtained: either an
/// embedded builtin source registered under the versioned name, or a source
/// file on the source path that lives inside a git repository.
fn versioned_source_available(globals: &GlobalEnv, base_ns: &str, versioned_ns: &str) -> bool {
    if globals.builtin_source(versioned_ns).is_some() {
        return true;
    }
    let rel_path = base_ns.replace('.', "/").replace('-', "_");
    let src_paths = globals.source_paths.read().unwrap().clone();
    match crate::loader::find_source_file(&rel_path, &src_paths) {
        Some((_, file_path)) => cljrs_vcs::find_repo_root(Path::new(&file_path)).is_some(),
        None => false,
    }
}

/// Ensure the versioned namespace `"<base_ns>@<commit>"` is loaded, returning
/// its name.
///
/// Source is taken from the embedded builtin registry first (AOT binaries
/// embed pinned sources under the versioned name; embedded sources were
/// signature-checked at compile time), falling back to fetching the file from
/// git history.  Idempotent, with the same same-thread cycle detection and
/// cross-thread coordination as the unversioned loader.
pub fn ensure_versioned_ns_loaded(
    globals: &Arc<GlobalEnv>,
    base_ns: &str,
    commit: &str,
) -> EvalResult<Arc<str>> {
    let versioned_ns_name: Arc<str> = Arc::from(format!("{base_ns}@{commit}"));

    if globals.is_loaded(&versioned_ns_name) {
        return Ok(versioned_ns_name);
    }

    let should_load = crate::loader::claim_or_wait(globals, &versioned_ns_name)?;
    if !should_load {
        return Ok(versioned_ns_name);
    }

    let result = do_versioned_load(globals, base_ns, commit, &versioned_ns_name);

    globals
        .loading
        .lock()
        .unwrap()
        .remove(versioned_ns_name.as_ref());
    if result.is_ok() {
        globals.mark_loaded(&versioned_ns_name);
    }
    globals.loading_done.notify_all();

    result?;
    Ok(versioned_ns_name)
}

/// Fetch (or look up) the pinned source and evaluate it into the versioned
/// namespace.  The caller owns the loading claim.
fn do_versioned_load(
    globals: &Arc<GlobalEnv>,
    base_ns: &str,
    commit: &str,
    versioned_ns_name: &Arc<str>,
) -> EvalResult<()> {
    // Embedded source first: AOT binaries register pinned sources under the
    // versioned namespace name.  These were fetched and (optionally)
    // signature-verified at compile time, so the git path is skipped
    // entirely.
    let (src, git_location): (String, Option<(String, String)>) =
        if let Some(builtin) = globals.builtin_source(versioned_ns_name) {
            (builtin.to_owned(), None)
        } else {
            let (src, location) = fetch_versioned_source(globals, base_ns, commit)?;
            (src, Some(location))
        };

    // Create the versioned namespace (immutable).
    {
        use cljrs_value::Namespace;
        let ns = cljrs_gc::GcPtr::new(Namespace::new_versioned(versioned_ns_name.as_ref()));
        if let Some((ref file_path, ref repo_root)) = git_location {
            ns.get().set_source_location(file_path, Some(repo_root));
        }
        let mut map = globals.namespaces.write().unwrap();
        map.entry(versioned_ns_name.clone()).or_insert(ns);
    }

    // Pre-refer clojure.core.
    globals.refer_all(versioned_ns_name, "clojure.core");

    // Evaluate all forms with a versioned commit context so that
    // same-namespace calls inside the historical source also resolve at
    // `commit` rather than HEAD.
    let saved_ns = globals
        .lookup_var("clojure.core", "*ns*")
        .and_then(|v| crate::dynamics::deref_var(&v));
    {
        let mut env = Env::new_versioned(globals.clone(), versioned_ns_name, commit);
        let file_label = format!("<{base_ns}@{commit}>");
        let mut parser = cljrs_reader::Parser::new(src, file_label);
        let forms = parser.parse_all().map_err(EvalError::Read)?;
        for form in forms {
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            globals
                .eval(&form, &mut env)
                .map_err(|e| crate::loader::annotate(e, versioned_ns_name))?;
        }
    }
    if let Some(saved) = saved_ns
        && let Some(var) = globals.lookup_var("clojure.core", "*ns*")
    {
        var.get().bind(saved);
    }

    Ok(())
}

/// Fetch the source of `base_ns` at `commit` from git history, returning
/// `(source_text, (file_path, repo_root))`.  Records the fetched text in
/// `GlobalEnv::versioned_sources` so the AOT compiler can embed it.
fn fetch_versioned_source(
    globals: &Arc<GlobalEnv>,
    base_ns: &str,
    commit: &str,
) -> EvalResult<(String, (String, String))> {
    // Locate the source file for the base namespace.
    let rel_path = base_ns.replace('.', "/").replace('-', "_");
    let src_paths = globals.source_paths.read().unwrap().clone();
    let (_, file_path) =
        crate::loader::find_source_file(&rel_path, &src_paths).ok_or_else(|| {
            EvalError::Runtime(format!(
                "Cannot find source for namespace {base_ns} (needed for {base_ns}@{commit})"
            ))
        })?;

    // Locate the git repository.
    let repo_root = cljrs_vcs::find_repo_root(Path::new(&file_path)).ok_or_else(|| {
        EvalError::Runtime(format!(
            "Namespace {base_ns} (file {file_path}) is not in a git repository; \
             cannot resolve {base_ns}@{commit}"
        ))
    })?;

    // Verify commit signature before loading any historical code.
    globals.check_commit_signature(&repo_root.to_string_lossy(), commit)?;

    // Compute the path relative to the repo root.
    let abs_file = Path::new(&file_path);
    let rel_file = abs_file.strip_prefix(&repo_root).map_err(|_| {
        EvalError::Runtime(format!(
            "Cannot compute relative path for {file_path} within {}",
            repo_root.display()
        ))
    })?;
    let rel_file_str = rel_file.to_string_lossy();

    // Fetch the source at the requested commit.
    let src = cljrs_vcs::get_file_at_commit(&repo_root, &rel_file_str, commit)
        .map_err(|e| EvalError::Runtime(format!("{e}")))?;

    // Record for AOT embedding.
    globals.record_versioned_source(&format!("{base_ns}@{commit}"), &src);

    Ok((src, (file_path, repo_root.display().to_string())))
}

/// Fall back to the HEAD value for a native Rust function when no Clojure
/// source definition exists for the symbol at the requested commit.
///
/// Native functions live in the running binary; we can't fetch and execute a
/// historical compiled implementation, so versioned lookups of a native
/// symbol resolve to the current implementation.
///
/// Returns the HEAD `NativeFunction` value (caching it under the requested
/// commit so later lookups are fast), or a descriptive `EvalError` otherwise.
fn native_head_fallback(
    globals: &GlobalEnv,
    base_ns: &str,
    name: &str,
    commit: &str,
) -> EvalResult {
    match globals.lookup_in_ns(base_ns, name) {
        Some(val) if matches!(val, Value::NativeFunction(_)) => {
            globals.cache_versioned(base_ns, name, commit, val.clone());
            Ok(val)
        }
        Some(_) => Err(EvalError::Runtime(format!(
            "Cannot find definition of `{name}` in `{base_ns}@{commit}`"
        ))),
        None => Err(EvalError::UnboundSymbol(format!("{base_ns}/{name}"))),
    }
}
