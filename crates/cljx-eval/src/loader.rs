//! Namespace file loader: resolves `require` to source files and evaluates them.

use std::sync::Arc;

use crate::env::{Env, GlobalEnv, RequireRefer, RequireSpec};
use crate::error::{EvalError, EvalResult};
use crate::eval;

/// Find, load, and wire up the source file for `spec.ns`.
///
/// - Idempotent: if already loaded, skips file evaluation but still applies
///   alias/refer in the *current* namespace.
/// - Cycle detection: returns an error if `spec.ns` is currently being loaded.
pub fn load_ns(globals: Arc<GlobalEnv>, spec: &RequireSpec, current_ns: &str) -> EvalResult<()> {
    let ns_name = &spec.ns;

    // Skip file loading if already done, but still apply alias/refer.
    let already_loaded = globals.is_loaded(ns_name);
    if !already_loaded {
        // Cycle detection.
        {
            let mut loading = globals.loading.lock().unwrap();
            if loading.contains(ns_name.as_ref()) {
                return Err(EvalError::Runtime(format!("circular require: {ns_name}")));
            }
            loading.insert(ns_name.clone());
        }

        // Resolve namespace name: check built-in registry first, then disk.
        // Clojure convention: dots → path separators, hyphens → underscores.
        let rel_path = ns_name.replace('.', "/").replace('-', "_");
        let src_paths = globals.source_paths.read().unwrap().clone();
        let (src, file_path): (String, String) =
            if let Some(builtin) = globals.builtin_source(ns_name) {
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
            let mut parser = cljx_reader::Parser::new(src, file_path);
            let forms = parser.parse_all().map_err(EvalError::Read)?;
            for form in forms {
                eval::eval(&form, &mut env).map_err(|e| annotate(e, ns_name))?;
            }
        }
        // Restore *ns* to the caller's namespace.
        if let Some(saved) = saved_ns
            && let Some(var) = globals.lookup_var("clojure.core", "*ns*")
        {
            var.get().bind(saved);
        }

        // Mark loaded and remove from in-progress set.
        globals.loading.lock().unwrap().remove(ns_name.as_ref());
        globals.mark_loaded(ns_name);
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
