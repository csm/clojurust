//! Namespace file loader: resolves `require` to source files and evaluates them.

use std::path::Path;
use std::sync::Arc;

use crate::env::{Env, GlobalEnv, RequireRefer, RequireSpec};
use crate::error::{EvalError, EvalResult};

/// Find, load, and wire up the source file for `spec.ns`.
///
/// - Idempotent: if already loaded, skips file evaluation but still applies
///   alias/refer in the *current* namespace.
/// - Same-thread cycle detection: returns an error if the current thread is
///   already loading `spec.ns` (true circular require).
/// - Cross-thread coordination: if a *different* thread is loading `spec.ns`,
///   waits for it to finish (via `GlobalEnv::loading_done`) instead of
///   reporting a spurious "circular require" error.
/// - Versioned require: if `spec.version` is set, delegates to
///   `load_versioned_ns` which fetches source at the given commit.
pub fn load_ns(globals: Arc<GlobalEnv>, spec: &RequireSpec, current_ns: &str) -> EvalResult<()> {
    // Versioned require: delegate entirely to the versioned loader.
    if let Some(ref commit) = spec.version {
        return load_versioned_ns(globals, spec, commit, current_ns);
    }

    let ns_name = &spec.ns;

    if !globals.is_loaded(ns_name) {
        // Try to claim this namespace for loading, or wait if another thread
        // is already loading it.
        let should_load = claim_or_wait(&globals, ns_name)?;

        if should_load {
            let result = do_load(&globals, ns_name);

            // Release the claim and notify any waiting threads.
            globals.loading.lock().unwrap().remove(ns_name.as_ref());
            if result.is_ok() {
                globals.mark_loaded(ns_name);
            }
            globals.loading_done.notify_all();

            result?;
        }
    }

    // Apply alias.
    if let Some(alias) = &spec.alias {
        globals.add_alias(current_ns, alias, ns_name);
    }

    // Apply refer.
    match &spec.refer {
        RequireRefer::None => {}
        RequireRefer::All => globals.refer_all(current_ns, ns_name),
        RequireRefer::Named(names) => globals.refer_named(current_ns, ns_name, names),
    }

    Ok(())
}

/// Claim `ns_name` for loading by the current thread, or wait until another
/// thread that claimed it finishes.
///
/// Returns `Ok(true)` if the caller claimed the namespace and must load it.
/// Returns `Ok(false)` if another thread loaded it while we waited.
/// Returns `Err` on a genuine circular require (same thread).
fn claim_or_wait(globals: &Arc<GlobalEnv>, ns_name: &Arc<str>) -> EvalResult<bool> {
    let tid = std::thread::current().id();
    loop {
        let mut loading = globals.loading.lock().unwrap();
        match loading.get(ns_name.as_ref()) {
            None => {
                loading.insert(ns_name.clone(), tid);
                return Ok(true);
            }
            Some(&owner) if owner == tid => {
                return Err(EvalError::Runtime(format!("circular require: {ns_name}")));
            }
            Some(_) => {
                // A different thread is loading this namespace.  Wait for it
                // to finish (the Condvar releases `loading` while sleeping).
                let _guard = globals.loading_done.wait(loading).unwrap();
                // After waking, the namespace may now be fully loaded.
                if globals.is_loaded(ns_name) {
                    return Ok(false);
                }
                // Otherwise loop and try to claim again.
            }
        }
    }
}

/// Evaluate the source file for `ns_name`, returning Ok(()) or an error.
/// The caller is responsible for claiming/releasing the namespace in the
/// `loading` map.
fn do_load(globals: &Arc<GlobalEnv>, ns_name: &Arc<str>) -> EvalResult<()> {
    // Resolve namespace name: check built-in registry first, then disk.
    // Clojure convention: dots → path separators, hyphens → underscores.
    let rel_path = ns_name.replace('.', "/").replace('-', "_");
    let src_paths = globals.source_paths.read().unwrap().clone();
    let (src, file_path): (String, String) = if let Some(builtin) = globals.builtin_source(ns_name)
    {
        (builtin.to_owned(), format!("<builtin:{ns_name}>"))
    } else {
        find_source_file(&rel_path, &src_paths).ok_or_else(|| {
            EvalError::Runtime(format!("Could not find namespace {ns_name} on source path"))
        })?
    };

    // Record source location on the namespace for versioned resolution.
    // Only meaningful for real files (not builtins).
    if !file_path.starts_with("<builtin:") {
        let repo_root =
            cljrs_vcs::find_repo_root(Path::new(&file_path)).map(|p| p.display().to_string());
        let ns_ptr = globals.get_or_create_ns(ns_name);
        ns_ptr
            .get()
            .set_source_location(&file_path, repo_root.as_deref());
    }

    // Pre-refer clojure.core so code in the file can use core fns before (ns ...).
    if ns_name.as_ref() != "clojure.core" {
        globals.refer_all(ns_name, "clojure.core");
    }

    // Evaluate the file in a new Env rooted at the namespace being loaded.
    // Save and restore *ns* so the caller's namespace is not disturbed.
    let saved_ns = globals
        .lookup_var("clojure.core", "*ns*")
        .and_then(|v| crate::dynamics::deref_var(&v));
    {
        let mut env = Env::new(globals.clone(), ns_name);
        let mut parser = cljrs_reader::Parser::new(src, file_path);
        let forms = parser.parse_all().map_err(EvalError::Read)?;
        for form in forms {
            // Alloc frame per top-level form: all allocations during this
            // form's evaluation are rooted.  Frame pops between forms,
            // allowing GC to collect temporaries from previous forms.
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            (*globals)
                .eval(&form, &mut env)
                .map_err(|e| annotate(e, ns_name))?;
        }
    }
    // Restore *ns* to the caller's namespace.
    if let Some(saved) = saved_ns
        && let Some(var) = globals.lookup_var("clojure.core", "*ns*")
    {
        var.get().bind(saved);
    }

    Ok(())
}

// ── Versioned namespace loading ───────────────────────────────────────────────

/// Load `spec.ns` at `commit`, registering the result as the namespace
/// `"<spec.ns>@<commit>"` in the global namespace table.
///
/// Idempotent: if the versioned namespace is already loaded, only applies the
/// alias/refer from `spec` in `current_ns`.
pub fn load_versioned_ns(
    globals: Arc<GlobalEnv>,
    spec: &RequireSpec,
    commit: &str,
    current_ns: &str,
) -> EvalResult<()> {
    let base_ns = &spec.ns;
    let versioned_ns_name: Arc<str> = Arc::from(format!("{base_ns}@{commit}"));

    if globals.is_loaded(&versioned_ns_name) {
        apply_alias_refer(&globals, &versioned_ns_name, current_ns, spec);
        return Ok(());
    }

    // Locate the source file for the base namespace.
    let rel_path = base_ns.replace('.', "/").replace('-', "_");
    let src_paths = globals.source_paths.read().unwrap().clone();
    let (_, file_path) = find_source_file(&rel_path, &src_paths).ok_or_else(|| {
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
    let abs_file = std::path::Path::new(&file_path);
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

    // Create the versioned namespace (immutable).
    {
        use cljrs_value::Namespace;
        let ns = cljrs_gc::GcPtr::new(Namespace::new_versioned(versioned_ns_name.as_ref()));
        ns.get()
            .set_source_location(&file_path, Some(&repo_root.display().to_string()));
        let mut map = globals.namespaces.write().unwrap();
        map.entry(versioned_ns_name.clone()).or_insert(ns);
    }

    // Pre-refer clojure.core.
    globals.refer_all(&versioned_ns_name, "clojure.core");

    // Evaluate all forms with a versioned commit context so that
    // same-namespace calls inside the historical source also resolve at
    // `commit` rather than HEAD.
    let saved_ns = globals
        .lookup_var("clojure.core", "*ns*")
        .and_then(|v| crate::dynamics::deref_var(&v));
    {
        let mut env = Env::new_versioned(globals.clone(), &versioned_ns_name, commit);
        let file_label = format!("<{base_ns}@{commit}>");
        let mut parser = cljrs_reader::Parser::new(src, file_label);
        let forms = parser.parse_all().map_err(EvalError::Read)?;
        for form in forms {
            let _alloc_frame = cljrs_gc::push_alloc_frame();
            globals
                .eval(&form, &mut env)
                .map_err(|e| annotate(e, &versioned_ns_name))?;
        }
    }
    if let Some(saved) = saved_ns
        && let Some(var) = globals.lookup_var("clojure.core", "*ns*")
    {
        var.get().bind(saved);
    }

    globals.mark_loaded(&versioned_ns_name);
    apply_alias_refer(&globals, &versioned_ns_name, current_ns, spec);
    Ok(())
}

/// Apply the alias and refer clauses from `spec` into `current_ns`, using
/// `effective_ns` as the source namespace (which may be `"base@commit"`).
fn apply_alias_refer(
    globals: &GlobalEnv,
    effective_ns: &Arc<str>,
    current_ns: &str,
    spec: &RequireSpec,
) {
    if let Some(alias) = &spec.alias {
        globals.add_alias(current_ns, alias, effective_ns);
    }
    match &spec.refer {
        RequireRefer::None => {}
        RequireRefer::All => globals.refer_all(current_ns, effective_ns),
        RequireRefer::Named(names) => globals.refer_named(current_ns, effective_ns, names),
    }
}

fn find_source_file(rel: &str, src_paths: &[std::path::PathBuf]) -> Option<(String, String)> {
    for dir in src_paths {
        for ext in &[".cljrs", ".cljc"] {
            let path = dir.join(format!("{rel}{ext}"));
            if path.exists() {
                let src = std::fs::read_to_string(&path).ok()?;
                return Some((src, path.display().to_string()));
            }
        }
    }
    None
}

/// Wrap an EvalError with namespace context.  Read errors (which carry
/// file/line/col in CljxError) are passed through unchanged so the CLI can
/// render them with full location information.
fn annotate(e: EvalError, ns_name: &Arc<str>) -> EvalError {
    match e {
        // Preserve read errors — they carry source location.
        EvalError::Read(_) => e,
        // Propagate recur unchanged (internal signal).
        EvalError::Recur(_) => e,
        // Annotate everything else with the namespace being loaded.
        other => EvalError::Runtime(format!("in {ns_name}: {other}")),
    }
}
