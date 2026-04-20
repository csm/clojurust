//! Pre-build tool: lowers Clojure functions to IR and serializes them.
//!
//! Boots a full eval environment, loads the Clojure compiler, then iterates
//! all vars in the requested namespaces. For each function, every arity is
//! lowered to IR and stored in an `IrBundle`. The bundle is serialized to
//! an output file that can be loaded at startup to skip compiler bootstrapping.

use std::path::PathBuf;
use std::sync::Arc;

use clap::Parser;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_ir::{serialize_bundle, IrBundle};
use cljrs_value::{CljxFn, Value};

/// Pre-lower Clojure namespaces to serialized IR bundles.
#[derive(Parser)]
#[command(name = "cljrs-ir-prebuild")]
struct Cli {
    /// Namespaces to lower (e.g. "clojure.core"). If none given, defaults to clojure.core.
    #[arg(short, long)]
    ns: Vec<String>,

    /// Output file path for the serialized IR bundle.
    #[arg(short, long, default_value = "ir_bundle.bin")]
    output: PathBuf,

    /// Additional source paths for namespace resolution.
    #[arg(long)]
    src_path: Vec<PathBuf>,

    /// Print verbose progress information.
    #[arg(short, long)]
    verbose: bool,
}

fn main() {
    let cli = Cli::parse();

    let namespaces = if cli.ns.is_empty() {
        vec!["clojure.core".to_string()]
    } else {
        cli.ns
    };

    // The compiler uses deep recursion; run on a large-stack thread.
    let result = std::thread::Builder::new()
        .name("prebuild-main".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || run_prebuild(&namespaces, &cli.output, &cli.src_path, cli.verbose))
        .expect("failed to spawn prebuild thread")
        .join()
        .expect("prebuild thread panicked");

    match result {
        Ok(stats) => {
            eprintln!(
                "Wrote {} functions ({} unsupported) to {}",
                stats.lowered,
                stats.unsupported,
                stats.output.display()
            );
        }
        Err(e) => {
            eprintln!("Error: {e}");
            std::process::exit(1);
        }
    }
}

struct PrebuildStats {
    lowered: usize,
    unsupported: usize,
    output: PathBuf,
}

fn run_prebuild(
    namespaces: &[String],
    output: &PathBuf,
    src_paths: &[PathBuf],
    verbose: bool,
) -> Result<PrebuildStats, String> {
    // 1. Boot the environment with compiler sources.
    let globals = if src_paths.is_empty() {
        cljrs_eval::standard_env()
    } else {
        cljrs_eval::standard_env_with_paths(src_paths.to_vec())
    };

    let mut env = Env::new(globals.clone(), "user");

    // 2. Load the Clojure compiler.
    if verbose {
        eprintln!("Loading compiler...");
    }
    if !cljrs_eval::ensure_compiler_loaded(&globals, &mut env) {
        return Err("Failed to load compiler namespaces".to_string());
    }
    if verbose {
        eprintln!("Compiler loaded.");
    }

    // 3. Load any non-core namespaces that were requested.
    for ns_name in namespaces {
        if ns_name != "clojure.core" {
            load_namespace(&globals, &mut env, ns_name, verbose)?;
        }
    }

    // 4. Walk all vars and lower functions to IR.
    let mut bundle = IrBundle::new();
    let mut lowered = 0usize;
    let mut unsupported = 0usize;

    for ns_name in namespaces {
        if verbose {
            eprintln!("Lowering namespace: {ns_name}");
        }
        let (ns_lowered, ns_unsupported) =
            lower_namespace(&globals, &mut env, ns_name, &mut bundle, verbose)?;
        lowered += ns_lowered;
        unsupported += ns_unsupported;
    }

    if verbose {
        eprintln!(
            "Lowering complete: {lowered} functions lowered, {unsupported} unsupported."
        );
    }

    // 5. Serialize and write to output file.
    let bytes = serialize_bundle(&bundle).map_err(|e| format!("serialization failed: {e}"))?;
    std::fs::write(output, &bytes).map_err(|e| format!("failed to write {}: {e}", output.display()))?;

    if verbose {
        eprintln!("Wrote {} bytes to {}", bytes.len(), output.display());
    }

    Ok(PrebuildStats {
        lowered,
        unsupported,
        output: output.clone(),
    })
}

/// Load a namespace by evaluating `(require 'ns-name)`.
fn load_namespace(
    globals: &Arc<GlobalEnv>,
    env: &mut Env,
    ns_name: &str,
    verbose: bool,
) -> Result<(), String> {
    if verbose {
        eprintln!("Loading namespace: {ns_name}");
    }

    let span = cljrs_types::span::Span::new(
        Arc::new("<prebuild>".to_string()),
        0,
        0,
        1,
        1,
    );
    let require_form = cljrs_reader::Form::new(
        cljrs_reader::form::FormKind::List(vec![
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Symbol("require".into()),
                span.clone(),
            ),
            cljrs_reader::Form::new(
                cljrs_reader::form::FormKind::Quote(Box::new(cljrs_reader::Form::new(
                    cljrs_reader::form::FormKind::Symbol(ns_name.into()),
                    span.clone(),
                ))),
                span,
            ),
        ]),
        cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1),
    );

    cljrs_eval::eval(&require_form, env)
        .map_err(|e| format!("failed to load namespace {ns_name}: {e:?}"))?;

    if !globals.is_loaded(ns_name) {
        return Err(format!("namespace {ns_name} was not marked as loaded after require"));
    }

    Ok(())
}

/// Lower all functions in a namespace to IR and store them in the bundle.
/// Returns (lowered_count, unsupported_count).
fn lower_namespace(
    globals: &Arc<GlobalEnv>,
    env: &mut Env,
    ns_name: &str,
    bundle: &mut IrBundle,
    verbose: bool,
) -> Result<(usize, usize), String> {
    // Collect all var names and their values from the namespace's interns.
    let var_entries: Vec<(Arc<str>, Value)> = {
        let ns_map = globals.namespaces.read().unwrap();
        let ns = ns_map
            .get(ns_name)
            .ok_or_else(|| format!("namespace {ns_name} not found"))?;
        let interns = ns.get().interns.lock().unwrap();
        interns
            .iter()
            .filter_map(|(name, var)| {
                let val = var.get().deref().unwrap_or(Value::Nil);
                Some((name.clone(), val))
            })
            .collect()
    };

    let mut lowered = 0usize;
    let mut unsupported = 0usize;

    for (var_name, val) in &var_entries {
        let f = match val {
            Value::Fn(gc_fn) => gc_fn.get().clone(),
            _ => continue,
        };

        // Skip macros — they operate on forms, not values.
        if f.is_macro {
            continue;
        }

        let fn_lowered = lower_function(ns_name, var_name, &f, env, bundle, verbose);
        lowered += fn_lowered.0;
        unsupported += fn_lowered.1;
    }

    if verbose {
        eprintln!("  {ns_name}: {lowered} lowered, {unsupported} unsupported");
    }

    Ok((lowered, unsupported))
}

/// Lower all arities of a single function.
/// Returns (lowered_count, unsupported_count).
fn lower_function(
    ns_name: &str,
    var_name: &str,
    f: &CljxFn,
    env: &mut Env,
    bundle: &mut IrBundle,
    verbose: bool,
) -> (usize, usize) {
    let mut lowered = 0;
    let mut unsupported = 0;

    for (arity_idx, arity) in f.arities.iter().enumerate() {
        let param_count = arity.params.len();
        let is_variadic = arity.rest_param.is_some();

        // Build a stable key: "ns/name:param_count" or "ns/name:param_count+"
        // for variadic arities. If there are multiple arities with different
        // param counts, each gets a unique key.
        let key = if is_variadic {
            format!("{ns_name}/{var_name}:{param_count}+")
        } else {
            format!("{ns_name}/{var_name}:{param_count}")
        };

        let ns_arc: Arc<str> = Arc::from(ns_name);
        match cljrs_eval::lower::lower_arity(
            f.name.as_deref(),
            &arity.params,
            arity.rest_param.as_ref(),
            &arity.body,
            &ns_arc,
            env,
        ) {
            Ok(ir_func) => {
                if verbose {
                    eprintln!("    lowered {key} ({} blocks)", ir_func.blocks.len());
                }
                bundle.insert(key, ir_func);
                lowered += 1;
            }
            Err(e) => {
                if verbose {
                    eprintln!("    unsupported {key}: {e}");
                }
                // Also try to give a more specific key if multiple arities
                // collide (shouldn't happen with param_count encoding, but
                // be safe).
                let _ = arity_idx;
                unsupported += 1;
            }
        }
    }

    (lowered, unsupported)
}
