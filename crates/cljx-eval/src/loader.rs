//! Namespace file loader: resolves `require` to source files and evaluates them.

use std::sync::Arc;

use crate::env::{Env, GlobalEnv, RequireRefer, RequireSpec};
use crate::error::EvalError;
use crate::eval;

/// Find, load, and wire up the source file for `spec.ns`.
///
/// - Idempotent: if already loaded, skips file evaluation but still applies
///   alias/refer in the *current* namespace.
/// - Cycle detection: returns an error if `spec.ns` is currently being loaded.
pub fn load_ns(
    globals: Arc<GlobalEnv>,
    spec: &RequireSpec,
    current_ns: &str,
) -> Result<(), String> {
    let ns_name = &spec.ns;

    // Skip file loading if already done, but still apply alias/refer.
    let already_loaded = globals.is_loaded(ns_name);
    if !already_loaded {
        // Cycle detection.
        {
            let mut loading = globals.loading.lock().unwrap();
            if loading.contains(ns_name.as_ref()) {
                return Err(format!("circular require: {ns_name}"));
            }
            loading.insert(ns_name.clone());
        }

        // Resolve namespace name to a file path.
        let rel_path = ns_name.replace('.', "/");
        let src_paths = globals.source_paths.read().unwrap().clone();
        let (src, file_path) = find_source_file(&rel_path, &src_paths)
            .ok_or_else(|| format!("Could not find namespace {ns_name} on source path"))?;

        // Pre-refer clojure.core so code in the file can use core fns before (ns ...).
        if ns_name.as_ref() != "clojure.core" {
            globals.refer_all(ns_name, "clojure.core");
        }

        // Evaluate the file in a new Env rooted at the namespace being loaded.
        {
            let mut env = Env::new(globals.clone(), ns_name);
            let mut parser = cljx_reader::Parser::new(src, file_path);
            let forms = parser
                .parse_all()
                .map_err(|e| format!("load error parsing {ns_name}: {e}"))?;
            for form in forms {
                eval::eval(&form, &mut env)
                    .map_err(|e| format!("load error in {ns_name}: {}", eval_err_msg(e)))?;
            }
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

fn eval_err_msg(e: EvalError) -> String {
    match e {
        EvalError::Runtime(s) => s,
        EvalError::UnboundSymbol(s) => format!("unbound symbol: {s}"),
        EvalError::Thrown(v) => format!("exception: {v}"),
        EvalError::Arity {
            name,
            expected,
            got,
        } => {
            format!("arity error in {name}: expected {expected} got {got}")
        }
        EvalError::NotCallable(s) => format!("not callable: {s}"),
        EvalError::Read(e) => format!("read error: {e}"),
        EvalError::Recur(_) => "unexpected recur".to_string(),
    }
}
