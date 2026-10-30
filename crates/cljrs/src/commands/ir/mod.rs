//! The `cljrs ir` subcommand: pre-lower namespaces to a serialized IR bundle,
//! dump a bundle, and render optimized IR to an HTML page.
//!
//! `build` boots a standard environment, walks every var in the requested
//! namespaces, lowers every function arity to IR, and writes the resulting
//! [`IrBundle`] to disk. A bundle can be replayed into a live environment with
//! [`cljrs_runtime::tiered::load_prebuilt_ir`], which matches bundle entries to the
//! `ir_arity_id`s assigned when the target functions are defined and populates
//! the IR cache directly — so those functions execute at Tier 1 (IR
//! interpreter) from their first call instead of waiting for background
//! lowering to promote them.
//!
//! No `cljrs` runtime path loads a bundle today; `build` and `dump` are
//! diagnostics for the lowerer and a starting point for embedders that cannot
//! run the background lowering worker, such as a `wasm32` build.

use std::path::PathBuf;
use std::sync::Arc;

pub mod viz;

use clap::Subcommand;

use cljrs_ir::IrBundle;
use cljrs_runtime::tiered::{Env, GlobalEnv};
use cljrs_value::{CljxFn, Value};

#[derive(Subcommand)]
pub enum IrCommands {
    /// Lower namespaces to IR and write a serialized bundle.
    ///
    /// The bundle is replayed into a live environment with the public
    /// `cljrs_runtime::tiered::load_prebuilt_ir` API, which matches bundle entries to the
    /// live `ir_arity_id`s assigned when the target functions are defined and
    /// populates the IR cache directly, so the functions execute at Tier 1
    /// (IR interpreter) from their very first call - skipping the warmup that
    /// background lowering normally needs. Most useful for cutting cold-start
    /// latency on targets that can't run the background lowering worker, such
    /// as an embedder built for `wasm32`.
    Build {
        /// Namespaces to lower (e.g. "clojure.core"). If none given, defaults to clojure.core.
        #[arg(short, long)]
        ns: Vec<String>,
        /// Output file path for the serialized IR bundle.
        #[arg(short, long, default_value = "ir_bundle.bin")]
        output: PathBuf,
        /// Additional source paths for namespace resolution.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// Print verbose progress information.
        #[arg(short, long)]
        verbose: bool,
    },
    /// Print a human-readable dump of a serialized IR bundle.
    Dump {
        /// Path to a bundle written by `ir build`.
        input: PathBuf,
    },
    /// Render the optimized IR for a source file to a self-contained HTML
    /// page (source ↔ IR with region color-coding and escape annotations).
    ///
    /// Useful for debugging the bump-allocation optimizer: any allocation
    /// that didn't make it into a region is flagged with its escape
    /// verdict and a representative blamed use.
    Viz {
        /// Path to the source file.
        file: PathBuf,
        /// Output HTML path.  If omitted, writes to <file>.ir.html alongside the source.
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// Suppress the `[aot] ...` progress output.
        #[arg(long)]
        quiet: bool,
    },
}

/// Dispatch the `ir` subcommands: `build`, `dump`, `viz`.
pub fn run(command: IrCommands) -> miette::Result<i32> {
    match command {
        IrCommands::Build {
            ns,
            output,
            src_paths,
            verbose,
        } => {
            let namespaces = if ns.is_empty() {
                vec!["clojure.core".to_string()]
            } else {
                ns
            };
            let stats = run_prebuild(&namespaces, &output, &src_paths, verbose)
                .map_err(|e| miette::miette!("{e}"))?;
            eprintln!(
                "Wrote {} functions ({} unsupported) to {}",
                stats.lowered,
                stats.unsupported,
                stats.output.display()
            );
            Ok(0)
        }
        IrCommands::Dump { input } => {
            let bytes =
                std::fs::read(&input).map_err(|e| miette::miette!("{}: {}", input.display(), e))?;
            let bundle = cljrs_ir::deserialize_bundle(&bytes)
                .map_err(|e| miette::miette!("failed to deserialize {}: {e}", input.display()))?;
            println!("{}", bundle);
            Ok(0)
        }
        IrCommands::Viz {
            file,
            out,
            src_paths,
            quiet,
        } => run_viz(file, out, src_paths, quiet),
    }
}

// ── ir viz ────────────────────────────────────────────────────────────────────

/// Lower a source file through the AOT pipeline (up to region optimization)
/// and write a self-contained HTML visualizer to disk.
fn run_viz(
    file: PathBuf,
    out: Option<PathBuf>,
    src_paths: Vec<PathBuf>,
    quiet: bool,
) -> miette::Result<i32> {
    let (source, ir) = cljrs_compiler::aot::lower_file_to_ir(&file, &src_paths, quiet)
        .map_err(|e| miette::miette!("{e}"))?;
    let title = format!("IR — {}", file.display());
    let html = viz::render_html(
        &ir,
        Some(&source),
        &viz::RenderOptions { title: Some(title) },
    );
    let out_path = out.unwrap_or_else(|| {
        let mut p = file.clone();
        let new_name = format!(
            "{}.ir.html",
            file.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("output")
        );
        p.set_file_name(new_name);
        p
    });
    std::fs::write(&out_path, html)
        .map_err(|e| miette::miette!("writing {}: {e}", out_path.display()))?;
    if !quiet {
        eprintln!("[ir viz] wrote {}", out_path.display());
    }
    Ok(0)
}

// ── ir build ──────────────────────────────────────────────────────────────────

/// Outcome of a [`run_prebuild`] call.
struct PrebuildStats {
    /// Number of function arities successfully lowered to IR.
    lowered: usize,
    /// Number of function arities the lowerer could not handle.
    unsupported: usize,
    /// Where the serialized bundle was written.
    output: PathBuf,
}

/// Boot a standard environment, lower every function in `namespaces` to IR,
/// and write the serialized bundle to `output`.
///
/// Non-`clojure.core` namespaces are `require`d from `src_paths` before
/// lowering. Returns an error string on any unrecoverable failure (IR
/// lowering disabled, a namespace that fails to load, or an I/O error).
fn run_prebuild(
    namespaces: &[String],
    output: &PathBuf,
    src_paths: &[PathBuf],
    verbose: bool,
) -> Result<PrebuildStats, String> {
    // 1. Boot the environment.
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .source_paths(src_paths.to_vec())
        .build()
        .map_err(|e| format!("{e}"))?;

    // 2. IR lowering must actually be live — `CLJRS_NO_IR` pins the runtime
    //    at tree-walk, and there would be nothing to lower or dump.
    if !runtime.tier_state().ir_enabled() {
        return Err("IR lowering is disabled (CLJRS_NO_IR is set)".to_string());
    }

    let globals = runtime.globals().clone();
    let mut env = Env::new(globals.clone(), "user");

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
        eprintln!("Lowering complete: {lowered} functions lowered, {unsupported} unsupported.");
    }

    // 5. Serialize and write to output file.
    let bytes =
        cljrs_ir::serialize_bundle(&bundle).map_err(|e| format!("serialization failed: {e}"))?;
    std::fs::write(output, &bytes)
        .map_err(|e| format!("failed to write {}: {e}", output.display()))?;

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

    let span = cljrs_types::span::Span::new(Arc::new("<prebuild>".to_string()), 0, 0, 1, 1);
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

    cljrs_runtime::tiered::eval(&require_form, env)
        .map_err(|e| format!("failed to load namespace {ns_name}: {e:?}"))?;

    if !globals.is_loaded(ns_name) {
        return Err(format!(
            "namespace {ns_name} was not marked as loaded after require"
        ));
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
    #[cfg(feature = "no-gc")]
    let _scratch = cljrs_gc::alloc_ctx::ScratchGuard::new();
    let var_entries: Vec<(Arc<str>, Value)> = {
        let ns_map = globals.namespaces.read().unwrap();
        let ns = ns_map
            .get(ns_name)
            .ok_or_else(|| format!("namespace {ns_name} not found"))?;
        let interns = ns.get().interns.lock().unwrap();
        interns
            .iter()
            .map(|(name, var)| {
                let val = var.get().deref().unwrap_or(Value::Nil);
                (name.clone(), val)
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

    for arity in &f.arities {
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
        match cljrs_runtime::tiered::lower::lower_arity(
            f.name.as_deref(),
            &arity.params,
            arity.rest_param.as_ref(),
            &arity.destructure_params,
            arity.destructure_rest.as_ref(),
            &arity.body,
            &ns_arc,
            env,
            f.is_async,
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
                unsupported += 1;
            }
        }
    }

    (lowered, unsupported)
}
