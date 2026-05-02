//! Namespace file loader: resolves `require` to source files and evaluates them.

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
pub fn load_ns(globals: Arc<GlobalEnv>, spec: &RequireSpec, current_ns: &str) -> EvalResult<()> {
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
