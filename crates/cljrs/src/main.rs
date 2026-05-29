use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use miette::IntoDiagnostic as _;

use cljrs_eval::{Env, EvalError, GlobalEnv, eval};
use cljrs_gc::GcConfig;
use cljrs_stdlib::{self as cljrs_stdlib};
use cljrs_value::Value;

/// Default thread stack size: 64 MiB.
const DEFAULT_STACK_SIZE: usize = 64 * 1024 * 1024;

/// clojurust — a Rust-hosted dialect of the Clojure programming language.
#[derive(Parser)]
#[command(name = "cljrs", version, about, long_about = None)]
struct Cli {
    /// Thread stack size in megabytes (default: 64).
    /// Increase if you hit stack overflows with deeply recursive code.
    #[arg(
        long,
        global = true,
        value_name = "MB",
        help = "Set thread stack size (default 64MB)"
    )]
    stack_size_mb: Option<usize>,

    #[arg(long, global = true, help = "Enable debug logging")]
    debug: bool,

    #[arg(long, global = true, help = "Enable trace logging (implies --debug)")]
    trace: bool,

    /// Require valid GPG or SSH signatures on every versioned commit before
    /// executing historical code.  Off by default.  Can also be enabled
    /// per-project via `:verify-commit-signatures true` in `cljrs.edn`.
    #[arg(long, global = true)]
    verify_commit_signatures: bool,

    /// Feature-level logging flags: -X debug:gc,jit or -X trace:reader
    ///
    /// Format: <level>:<feature1>,<feature2>,...
    /// Levels: debug, trace
    #[arg(short = 'X', global = true, value_name = "LEVEL:FEATURES")]
    x_flags: Vec<String>,

    /// Print GC statistics on exit. Pass a path to write them to a file;
    /// pass the flag without a value to write them to stdout. Only the
    /// `run`, `eval`, and `test` subcommands honour this flag.
    #[arg(
        long = "gc-stats",
        global = true,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "",
    )]
    gc_stats: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Interpret a .cljrs or .cljc source file.
    Run {
        /// Path to the source file.
        file: PathBuf,
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// GC soft memory limit in MB (triggers collection when exceeded).
        #[arg(long)]
        gc_soft_limit_mb: Option<usize>,
        /// GC hard memory limit in MB (forces collection when exceeded).
        #[arg(long)]
        gc_hard_limit_mb: Option<usize>,
        /// Arguments forwarded to -main (everything after `--`).
        #[arg(last = true, value_name = "ARGS")]
        args: Vec<String>,
    },
    /// Start an interactive REPL.
    Repl {
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// GC soft memory limit in MB (triggers collection when exceeded).
        #[arg(long)]
        gc_soft_limit_mb: Option<usize>,
        /// GC hard memory limit in MB (forces collection when exceeded).
        #[arg(long)]
        gc_hard_limit_mb: Option<usize>,
    },
    /// AOT-compile a source file to a native binary.
    Compile {
        /// Path to the source file, or directory when --test is used.
        file: PathBuf,
        /// Output binary path.
        #[arg(short, long)]
        out: PathBuf,
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// Compile a test harness that runs all tests in the given file/directory.
        #[arg(long)]
        test: bool,
        /// GC soft memory limit in MB (triggers collection when exceeded).
        #[arg(long)]
        gc_soft_limit_mb: Option<usize>,
        /// GC hard memory limit in MB (forces collection when exceeded).
        #[arg(long)]
        gc_hard_limit_mb: Option<usize>,
    },
    /// Evaluate a single Clojure expression and print the result.
    Eval {
        /// The expression to evaluate.
        expr: String,
    },
    /// Render the optimized IR for a source file to a self-contained HTML
    /// page (source ↔ IR with region color-coding and escape annotations).
    ///
    /// Useful for debugging the bump-allocation optimizer: any allocation
    /// that didn't make it into a region is flagged with its escape
    /// verdict and a representative blamed use.
    IrViz {
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
    /// Run clojure.test tests for one or more namespaces.
    ///
    /// If no namespaces are given, discovers and runs all test namespaces
    /// found in the source paths.
    Test {
        /// Namespaces to test (e.g. my.app.core-test).
        /// If omitted, all namespaces in --src-path are discovered.
        namespaces: Vec<String>,
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
        /// Print each passing assertion (helps identify which test hangs).
        #[arg(long, short)]
        verbose: bool,
        /// GC soft memory limit in MB (triggers collection when exceeded).
        #[arg(long)]
        gc_soft_limit_mb: Option<usize>,
        /// GC hard memory limit in MB (forces collection when exceeded).
        #[arg(long)]
        gc_hard_limit_mb: Option<usize>,
    },
    /// Manage project dependencies declared in cljrs.edn.
    ///
    /// Git dependencies are cached in ~/.cljrs/cache/git/.
    /// No network access occurs unless you run `cljrs deps fetch`.
    Deps {
        #[command(subcommand)]
        command: DepsCommands,
    },
    /// Build the project's native Rust crate as a shared library.
    ///
    /// Reads the `:rust` key from `cljrs.edn`, runs `cargo build` in the
    /// declared crate directory, and prints the path of the resulting
    /// `.so` / `.dylib` / `.dll`.  The library is loaded automatically by
    /// `cljrs run` and `cljrs repl` to register native functions before any
    /// Clojure code is evaluated.
    ///
    /// The user crate must declare `crate-type = ["cdylib"]` (or
    /// `["cdylib", "rlib"]`) and export a `#[no_mangle] pub extern "C" fn
    /// cljrs_init(registry: *mut cljrs_interop::Registry)` entry point.
    BuildNative {
        /// Build in release mode instead of debug.
        #[arg(long)]
        release: bool,
    },
}

#[derive(Subcommand)]
enum DepsCommands {
    /// Clone or update git dependencies from cljrs.edn.
    ///
    /// Without a name, fetches every git dependency declared in the
    /// nearest cljrs.edn.  With a name, fetches only that dependency.
    Fetch {
        /// Dependency name to fetch (fetches all if omitted).
        name: Option<String>,
    },
    /// Show which dependencies are cached and which are missing.
    Status,
}

/// Build GC config from CLI flags, or use defaults if not specified.
fn build_gc_config(soft_limit_mb: Option<usize>, hard_limit_mb: Option<usize>) -> Arc<GcConfig> {
    match (soft_limit_mb, hard_limit_mb) {
        (Some(soft), Some(hard)) => Arc::new(GcConfig::with_limits(
            soft * 1024 * 1024,
            hard * 1024 * 1024,
        )),
        (Some(soft), None) => Arc::new(GcConfig::with_hard_limit(soft * 1024 * 1024)),
        (None, Some(hard)) => Arc::new(GcConfig::with_hard_limit(hard * 1024 * 1024)),
        (None, None) => Arc::new(GcConfig::new()),
    }
}

fn main() -> miette::Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .build(),
        )
    }))
    .into_diagnostic()?;

    let cli = Cli::parse();
    let _ = tracing_subscriber::fmt()
        .with_max_level(if cli.trace {
            tracing::Level::TRACE
        } else if cli.debug {
            tracing::Level::DEBUG
        } else {
            tracing::Level::INFO
        })
        .try_init();

    // Parse -X feature logging flags
    for flag in &cli.x_flags {
        cljrs_logging::parse_x_flag(flag).map_err(|e| miette::miette!("invalid -X flag: {e}"))?;
    }

    let stack_size = cli
        .stack_size_mb
        .map(|mb| mb * 1024 * 1024)
        .unwrap_or(DEFAULT_STACK_SIZE);

    // Spawn the actual work on a thread with a larger stack to handle
    // deeply recursive Clojure code (lazy-seq chains, recursive macros, etc.).
    let builder = std::thread::Builder::new()
        .name("cljrs-main".into())
        .stack_size(stack_size);
    let handle = builder
        .spawn(move || {
            #[cfg(feature = "async")]
            {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("failed to build Tokio runtime");
                let local = tokio::task::LocalSet::new();
                rt.block_on(local.run_until(async move { run(cli) }))
            }
            #[cfg(not(feature = "async"))]
            run(cli)
        })
        .into_diagnostic()?;
    let result: miette::Result<i32> = handle.join().unwrap_or_else(|e| {
        eprintln!("cljrs: thread panicked: {e:?}");
        std::process::exit(1);
    });
    match result {
        Ok(0) => Ok(()),
        Ok(code) => std::process::exit(code),
        Err(e) => Err(e),
    }
}

fn run(cli: Cli) -> miette::Result<i32> {
    // Register the main thread as a GC mutator so the collector knows
    // how many threads to wait for during stop-the-world collection.
    let _mutator = cljrs_gc::register_mutator();

    let gc_stats_target = cli.gc_stats.clone();
    let verify_commit_signatures = cli.verify_commit_signatures;
    let supports_gc_stats = matches!(
        &cli.command,
        Commands::Run { .. } | Commands::Eval { .. } | Commands::Test { .. },
    );

    let result = run_command(cli.command, verify_commit_signatures);

    if supports_gc_stats
        && let Some(target) = gc_stats_target.as_deref()
        && let Err(e) = write_gc_stats(target)
    {
        eprintln!("cljrs: failed to write GC stats: {e}");
    }

    result
}

fn run_command(command: Commands, verify_commit_signatures: bool) -> miette::Result<i32> {
    match command {
        Commands::Run {
            file,
            src_paths,
            gc_soft_limit_mb,
            gc_hard_limit_mb,
            args,
        } => {
            let src = std::fs::read_to_string(&file)
                .map_err(|e| miette::miette!("{}: {}", file.display(), e))?;
            let filename = file.display().to_string();
            let gc_config = build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb);
            let globals = setup_globals(src_paths, gc_config, verify_commit_signatures);
            run_source(&src, &filename, globals, &args)?;
            Ok(0)
        }
        Commands::Repl {
            src_paths,
            gc_soft_limit_mb,
            gc_hard_limit_mb,
        } => {
            let gc_config = build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb);
            let globals = setup_globals(src_paths, gc_config, verify_commit_signatures);
            run_repl(globals);
            Ok(0)
        }
        Commands::Compile {
            file,
            out,
            src_paths,
            test,
            gc_soft_limit_mb,
            gc_hard_limit_mb,
        } => {
            // GC config is for the compiled binary, not the compilation process
            let _gc_config = build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb);
            // Load cljrs.edn :rust config so native init gets wired into the
            // generated harness main.rs.
            let rust_config = std::env::current_dir()
                .ok()
                .and_then(|cwd| cljrs_deps::load_config(&cwd).ok().flatten())
                .and_then(|c| c.rust);
            if test {
                // For test mode, the file is a directory containing test files
                cljrs_compiler::aot::compile_test_harness(&file, &out, &src_paths)
                    .map_err(|e| miette::miette!("{e}"))?;
            } else {
                cljrs_compiler::aot::compile_file(&file, &out, &src_paths, rust_config.as_ref())
                    .map_err(|e| miette::miette!("{e}"))?;
            }
            Ok(0)
        }
        Commands::Eval { expr } => {
            let gc_config = Arc::new(GcConfig::new());
            let globals = setup_globals(Vec::new(), gc_config, verify_commit_signatures);
            let result = eval_source(&expr, "<eval>", globals)?;
            if result != Value::Nil {
                println!("{}", result);
            }
            Ok(0)
        }
        Commands::IrViz {
            file,
            out,
            src_paths,
            quiet,
        } => run_ir_viz(file, out, src_paths, quiet),
        Commands::Test {
            namespaces,
            src_paths,
            verbose,
            gc_soft_limit_mb,
            gc_hard_limit_mb,
        } => run_tests_command(
            namespaces,
            src_paths,
            verbose,
            gc_soft_limit_mb,
            gc_hard_limit_mb,
            verify_commit_signatures,
        ),
        Commands::Deps { command } => match command {
            DepsCommands::Fetch { name } => run_deps_fetch(name),
            DepsCommands::Status => run_deps_status(),
        },
        Commands::BuildNative { release } => run_build_native(release),
    }
}

/// Lower a source file through the AOT pipeline (up to region optimization)
/// and write a self-contained HTML visualizer to disk.
fn run_ir_viz(
    file: PathBuf,
    out: Option<PathBuf>,
    src_paths: Vec<PathBuf>,
    quiet: bool,
) -> miette::Result<i32> {
    let (source, ir) = cljrs_compiler::aot::lower_file_to_ir(&file, &src_paths, quiet)
        .map_err(|e| miette::miette!("{e}"))?;
    let title = format!("IR — {}", file.display());
    let html = cljrs_ir_viz::render_html(
        &ir,
        Some(&source),
        &cljrs_ir_viz::RenderOptions { title: Some(title) },
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
        eprintln!("[ir-viz] wrote {}", out_path.display());
    }
    Ok(0)
}

/// Write a snapshot of `cljrs_gc::GC_STATS` to `target`.
///
/// An empty target (the flag was passed without a value) writes to stdout;
/// any other value is treated as a filesystem path.
fn write_gc_stats(target: &str) -> std::io::Result<()> {
    let snapshot = cljrs_gc::GC_STATS.snapshot();
    if target.is_empty() {
        println!("{snapshot}");
        Ok(())
    } else {
        std::fs::write(target, format!("{snapshot}\n"))
    }
}

// ── Environment setup ─────────────────────────────────────────────────────────

/// Create a fully initialised `GlobalEnv` with stdlib, user source paths, GC
/// config, and any `cljrs.edn` found in the current working directory.
///
/// Paths declared in `:paths` of `cljrs.edn` are appended to `src_paths` (CLI
/// flags take precedence).  The parsed `DepsConfig` is stored in
/// `GlobalEnv.deps_config` so that versioned symbol resolution and the
/// `deps fetch`/`deps status` commands share the same config object.
fn setup_globals(
    src_paths: Vec<PathBuf>,
    gc_config: Arc<GcConfig>,
    verify_commit_signatures: bool,
) -> Arc<GlobalEnv> {
    let globals = cljrs_stdlib::standard_env_with_paths_and_config(src_paths, gc_config);
    if verify_commit_signatures {
        globals
            .verify_commit_signatures
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if let Ok(cwd) = std::env::current_dir() {
        apply_deps_config(&globals, &cwd);
    }
    #[cfg(feature = "async")]
    {
        cljrs_async::init(&globals);
        cljrs_io::init(&globals);
    }
    globals
}

/// Load the nearest `cljrs.edn` and wire its data into `globals`.
///
/// Silently does nothing when no config file is found; prints a warning to
/// stderr when the file exists but cannot be parsed.
fn apply_deps_config(globals: &Arc<GlobalEnv>, cwd: &Path) {
    match cljrs_deps::load_config(cwd) {
        Ok(Some(config)) => {
            // Append edn :paths to the source-path list (CLI paths come first).
            {
                let mut paths = globals.source_paths.write().unwrap();
                for p in &config.paths {
                    if !paths.contains(p) {
                        paths.push(p.clone());
                    }
                }
            }
            if config.verify_commit_signatures {
                globals
                    .verify_commit_signatures
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            // Load the native shared library (if :rust is configured) so that
            // native functions are registered before any Clojure code runs.
            if let Some(rust_config) = &config.rust {
                load_native_lib(rust_config, globals);
            }
            *globals.deps_config.write().unwrap() = Some(Arc::new(config));
        }
        Ok(None) => {}
        Err(e) => eprintln!("cljrs: warning: could not load cljrs.edn: {e}"),
    }
}

// ── Source evaluation ─────────────────────────────────────────────────────────

/// Evaluate all forms in `src`, printing nothing. Returns the last value.
fn eval_source(src: &str, filename: &str, globals: Arc<GlobalEnv>) -> miette::Result<Value> {
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)
}

/// Run a source file: evaluate all top-level forms, then call `-main` if defined.
fn run_source(
    src: &str,
    filename: &str,
    globals: Arc<GlobalEnv>,
    args: &[String],
) -> miette::Result<()> {
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)?;
    call_main_if_defined(&mut env, args)?;
    Ok(())
}

/// Call `-main` in the current namespace if it is defined, passing `args` as
/// individual string arguments. Silently skips if `-main` is not defined.
fn call_main_if_defined(env: &mut Env, args: &[String]) -> miette::Result<()> {
    // resolve returns nil for undefined symbols; swallow lookup errors defensively.
    let resolved = eval_in(env, "(resolve '-main)", "<main-check>").unwrap_or(Value::Nil);
    if resolved == Value::Nil {
        return Ok(());
    }
    let escaped: Vec<String> = args.iter().map(|s| escape_clojure_string(s)).collect();
    let call = format!("(-main {})", escaped.join(" "));
    eval_in(env, &call, "<main>")?;
    Ok(())
}

/// Produce a Clojure string literal (double-quoted, with escapes) for `s`.
fn escape_clojure_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Evaluate `src` in an existing `Env`. Returns the last value.
fn eval_in(env: &mut Env, src: &str, filename: &str) -> miette::Result<Value> {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), filename.to_string());
    let forms = parser.parse_all().map_err(miette::Report::from)?;

    let mut result = Value::Nil;
    for form in forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        result = eval(&form, env).map_err(format_eval_error)?;
    }
    Ok(result)
}

fn format_eval_error(e: EvalError) -> miette::Report {
    match e {
        EvalError::Thrown(val) => miette::miette!("Unhandled exception: {}", val),
        EvalError::UnboundSymbol(s) => miette::miette!("Unable to resolve symbol: {}", s),
        EvalError::Arity {
            name,
            expected,
            got,
        } => miette::miette!("Wrong number of args ({got}) passed to {name}; expected {expected}"),
        EvalError::NotCallable(s) => miette::miette!("Not a function: {}", s),
        EvalError::Runtime(msg) => miette::miette!("{}", msg),
        EvalError::Read(e) => miette::Report::from(e),
        EvalError::Recur(_) => miette::miette!("recur outside of loop/fn"),
        EvalError::CommitSignatureVerificationFailed { commit, reason } => {
            miette::miette!("commit {commit:?} failed signature verification: {reason}")
        }
    }
}

// ── Native library support ────────────────────────────────────────────────────

/// Return the expected on-disk path for the shared library produced by
/// `cargo build` inside `crate_dir`.
///
/// Respects cargo's workspace semantics: when `crate_dir` is a workspace
/// member, cargo writes artifacts to `<workspace_root>/target/`, not
/// `<crate_dir>/target/`. We ask cargo where its target directory is via
/// `cargo metadata`. If that fails (no cargo on PATH, malformed manifest,
/// etc.), we fall back to `<crate_dir>/target/` so the standalone-crate
/// case still works.
fn native_lib_path(crate_dir: &Path, crate_name: &str, release: bool) -> PathBuf {
    let profile = if release { "release" } else { "debug" };
    let lib_file = if cfg!(target_os = "windows") {
        format!("{crate_name}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{crate_name}.dylib")
    } else {
        format!("lib{crate_name}.so")
    };
    let target_dir = cargo_target_dir(crate_dir).unwrap_or_else(|| crate_dir.join("target"));
    target_dir.join(profile).join(lib_file)
}

/// Ask `cargo metadata` for the target directory that cargo will actually use
/// when building inside `crate_dir`. Returns `None` on any failure; the caller
/// is expected to fall back to `<crate_dir>/target`.
fn cargo_target_dir(crate_dir: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(crate_dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = std::str::from_utf8(&output.stdout).ok()?;
    // Cargo's metadata JSON puts `target_directory` at the top level. We do
    // a small targeted extract rather than pulling in a JSON dependency.
    let key = r#""target_directory":""#;
    let start = stdout.find(key)? + key.len();
    let rest = &stdout[start..];
    let mut end = None;
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'"' => {
                end = Some(i);
                break;
            }
            _ => i += 1,
        }
    }
    let raw = &rest[..end?];
    Some(PathBuf::from(json_unescape(raw)))
}

/// Decode the small subset of JSON string escapes that can appear in a
/// `target_directory` path emitted by `cargo metadata`.
fn json_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('/') => out.push('/'),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Load the shared library declared by `rust_config` and call its `cljrs_init`
/// entry point to register native functions into `globals`.
///
/// A missing library emits a warning and returns — callers of unregistered
/// functions will get a runtime error rather than a startup crash, which is
/// friendlier during development.
fn load_native_lib(rust_config: &cljrs_deps::RustConfig, globals: &Arc<GlobalEnv>) {
    let Some(init_fn) = rust_config.init_fn.as_deref() else {
        return;
    };
    let Some(crate_name) = rust_config.crate_name() else {
        return;
    };
    // Symbol name is the last segment of the Rust path, e.g. "cljrs_init".
    let sym_name = init_fn.rsplit("::").next().unwrap_or(init_fn);

    let lib_path = native_lib_path(&rust_config.crate_dir, crate_name, false);
    if !lib_path.exists() {
        eprintln!(
            "cljrs: native library not found at {} — run `cljrs build-native` first",
            lib_path.display()
        );
        return;
    }

    // SAFETY: we own the process and are responsible for ensuring the library
    // stays loaded (via mem::forget below) for the entire lifetime of globals.
    unsafe {
        let lib = match libloading::Library::new(&lib_path) {
            Ok(l) => l,
            Err(e) => {
                eprintln!("cljrs: could not load {}: {e}", lib_path.display());
                return;
            }
        };

        // The exported symbol has C linkage and takes a raw pointer so it is
        // callable across the FFI boundary without ABI assumptions.
        let sym_bytes: Vec<u8> = format!("{sym_name}\0").into_bytes();
        let init: libloading::Symbol<unsafe extern "C" fn(*mut cljrs_interop::Registry)> =
            match lib.get(&sym_bytes) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!(
                        "cljrs: could not find symbol {sym_name} in {}: {e}",
                        lib_path.display()
                    );
                    return;
                }
            };

        let mut registry = cljrs_interop::Registry::new(globals.clone());
        init(&mut registry as *mut _);

        // Prevent the library from being unloaded — its code must remain
        // reachable as long as any registered NativeFn closures exist.
        std::mem::forget(lib);
    }
    eprintln!(
        "[build-native] loaded {} ({})",
        lib_path.display(),
        sym_name
    );
}

/// Build the native Rust crate declared in `cljrs.edn` as a shared library.
fn run_build_native(release: bool) -> miette::Result<i32> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_deps::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    let rust_config = config
        .rust
        .as_ref()
        .ok_or_else(|| miette::miette!("no :rust key found in cljrs.edn"))?;

    let crate_name = rust_config
        .crate_name()
        .ok_or_else(|| miette::miette!(":rust has no :init function; cannot derive crate name"))?;

    eprintln!(
        "[build-native] building {} in {}",
        crate_name,
        rust_config.crate_dir.display()
    );

    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build");
    if release {
        cmd.arg("--release");
    }
    cmd.current_dir(&rust_config.crate_dir);

    let status = cmd.status().into_diagnostic()?;
    if !status.success() {
        return Err(miette::miette!("cargo build failed"));
    }

    let lib_path = native_lib_path(&rust_config.crate_dir, crate_name, release);
    eprintln!("[build-native] built {}", lib_path.display());
    println!("{}", lib_path.display());

    Ok(0)
}

// ── Deps subcommand ───────────────────────────────────────────────────────────

/// Fetch one or all git dependencies declared in the nearest `cljrs.edn`.
fn run_deps_fetch(name: Option<String>) -> miette::Result<i32> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_deps::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    if config.deps.is_empty() {
        println!("No dependencies declared in cljrs.edn.");
        return Ok(0);
    }

    // Collect (dep_name, dependency) pairs to process.
    let to_fetch: Vec<(&str, &cljrs_deps::Dependency)> = if let Some(ref n) = name {
        match config.find_dep(n) {
            Some(dep) => vec![(n.as_str(), dep)],
            None => {
                return Err(miette::miette!("dependency {:?} not found in cljrs.edn", n));
            }
        }
    } else {
        config.deps.iter().map(|(n, d)| (n.as_ref(), d)).collect()
    };

    let mut all_ok = true;
    for (dep_name, dep) in to_fetch {
        match dep {
            cljrs_deps::Dependency::Git(git_dep) => {
                eprintln!("fetching {dep_name} ({})...", git_dep.url);
                match cljrs_vcs::fetch_remote(&git_dep.url, &git_dep.sha) {
                    Ok(path) => eprintln!("  ok → {}", path.display()),
                    Err(e) => {
                        eprintln!("  error: {e}");
                        all_ok = false;
                    }
                }
            }
            cljrs_deps::Dependency::Local { root } => {
                if root.exists() {
                    eprintln!("{dep_name}: local dep at {} — ok", root.display());
                } else {
                    eprintln!(
                        "{dep_name}: local dep at {} — directory not found",
                        root.display()
                    );
                    all_ok = false;
                }
            }
        }
    }

    Ok(if all_ok { 0 } else { 1 })
}

/// Print the cache status of every dependency declared in the nearest `cljrs.edn`.
fn run_deps_status() -> miette::Result<i32> {
    let cwd = std::env::current_dir().into_diagnostic()?;
    let config = cljrs_deps::load_config(&cwd)
        .into_diagnostic()?
        .ok_or_else(|| miette::miette!("no cljrs.edn found in or above the current directory"))?;

    if config.deps.is_empty() {
        println!("No dependencies declared in cljrs.edn.");
        return Ok(0);
    }

    let mut all_ok = true;
    for (dep_name, dep) in &config.deps {
        match dep {
            cljrs_deps::Dependency::Git(git_dep) => {
                let cache_path = cljrs_vcs::cache_path_for_url(&git_dep.url);
                let sha_present = cache_path.exists()
                    && std::process::Command::new("git")
                        .arg("-C")
                        .arg(&cache_path)
                        .arg("cat-file")
                        .arg("-e")
                        .arg(git_dep.sha.as_ref())
                        .status()
                        .map(|s| s.success())
                        .unwrap_or(false);
                if sha_present {
                    println!(
                        "{dep_name}: cached (sha: {}, url: {})",
                        git_dep.sha, git_dep.url
                    );
                } else {
                    println!(
                        "{dep_name}: NOT cached — run `cljrs deps fetch` (sha: {}, url: {})",
                        git_dep.sha, git_dep.url
                    );
                    all_ok = false;
                }
            }
            cljrs_deps::Dependency::Local { root } => {
                if root.exists() {
                    println!("{dep_name}: local dep at {} — ok", root.display());
                } else {
                    println!("{dep_name}: local dep at {} — NOT FOUND", root.display());
                    all_ok = false;
                }
            }
        }
    }

    Ok(if all_ok { 0 } else { 1 })
}

// ── Test runner ───────────────────────────────────────────────────────────────

/// Result of running tests for a single namespace.
struct NsTestResult {
    ns: String,
    pass: i64,
    fail: i64,
    error: i64,
    test_count: i64,
    /// None if tests ran; Some(msg) if the ns failed to load.
    load_error: Option<String>,
}

fn run_tests_command(
    namespaces: Vec<String>,
    src_paths: Vec<PathBuf>,
    verbose: bool,
    gc_soft_limit_mb: Option<usize>,
    gc_hard_limit_mb: Option<usize>,
    verify_commit_signatures: bool,
) -> miette::Result<i32> {
    let gc_config = build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb);
    let globals = setup_globals(src_paths, gc_config, verify_commit_signatures);

    let namespaces = if namespaces.is_empty() {
        // Read the final source paths (which may include cljrs.edn :paths).
        let effective_paths = globals.source_paths.read().unwrap().clone();
        let discovered = discover_namespaces(&effective_paths);
        if discovered.is_empty() {
            eprintln!("cljrs test: no test namespaces found in source paths");
            return Ok(2);
        }
        eprintln!("Discovered {} test namespace(s).\n", discovered.len());
        discovered
    } else {
        namespaces
    };

    let mut env = Env::new(globals, "user");

    // Ensure clojure.test is loaded.
    eval_in(&mut env, "(require 'clojure.test)", "<test>")?;

    if verbose {
        eval_in(
            &mut env,
            "(alter-var-root (var clojure.test/*verbose*) (constantly true))",
            "<test>",
        )?;
    }

    let start = Instant::now();
    let mut results: Vec<NsTestResult> = Vec::new();

    for ns in &namespaces {
        let result = run_single_ns_tests(&mut env, ns);
        // Remove the namespace after testing so its closures and form-trees can
        // be reclaimed by GC.  Without this all 233 namespaces accumulate
        // simultaneously and peak RSS can exceed 15 GB.
        // Two force_collect calls are required: GC_INITIAL_LIVES=2 means an
        // unreachable object survives one cycle in the grace period before being
        // freed on the second cycle.
        env.globals.namespaces.write().unwrap().remove(ns.as_str());
        env.globals.loaded.lock().unwrap().remove(ns.as_str());
        cljrs_eval::force_collect(&env);
        cljrs_eval::force_collect(&env);
        results.push(result);
    }

    let elapsed = start.elapsed();

    // Print summary.
    print_summary(&results, elapsed);

    let total_fail: i64 = results.iter().map(|r| r.fail + r.error).sum();
    let total_load_errors: usize = results.iter().filter(|r| r.load_error.is_some()).count();

    if total_fail > 0 || total_load_errors > 0 {
        Ok(1)
    } else {
        Ok(0)
    }
}

fn run_single_ns_tests(env: &mut Env, ns: &str) -> NsTestResult {
    // Try to load the namespace.
    if let Err(e) = eval_in(env, &format!("(require '{ns})"), "<test>") {
        return NsTestResult {
            ns: ns.to_string(),
            pass: 0,
            fail: 0,
            error: 0,
            test_count: 0,
            load_error: Some(format!("{e}")),
        };
    }

    // Run the tests.
    match eval_in(env, &format!("(clojure.test/run-tests '{ns})"), "<test>") {
        Ok(counters) => {
            let (pass, fail, error, test_count) = extract_counters(&counters);
            NsTestResult {
                ns: ns.to_string(),
                pass,
                fail,
                error,
                test_count,
                load_error: None,
            }
        }
        Err(e) => NsTestResult {
            ns: ns.to_string(),
            pass: 0,
            fail: 0,
            error: 0,
            test_count: 0,
            load_error: Some(format!("run-tests failed: {e}")),
        },
    }
}

fn extract_counters(val: &Value) -> (i64, i64, i64, i64) {
    let Value::Map(m) = val else {
        return (0, 0, 0, 0);
    };
    let mut pass = 0i64;
    let mut fail = 0i64;
    let mut error = 0i64;
    let mut test_count = 0i64;
    m.for_each(|k, v| {
        if let (Value::Keyword(kw), Value::Long(count)) = (k, v) {
            match kw.get().name.as_ref() {
                "pass" => pass = *count,
                "fail" => fail = *count,
                "error" => error = *count,
                "test" => test_count = *count,
                _ => {}
            }
        }
    });
    (pass, fail, error, test_count)
}

fn print_summary(results: &[NsTestResult], elapsed: std::time::Duration) {
    let total_tests: i64 = results.iter().map(|r| r.test_count).sum();
    let total_assertions: i64 = results.iter().map(|r| r.pass + r.fail + r.error).sum();
    let total_pass: i64 = results.iter().map(|r| r.pass).sum();
    let total_fail: i64 = results.iter().map(|r| r.fail).sum();
    let total_error: i64 = results.iter().map(|r| r.error).sum();
    let load_errors: Vec<&NsTestResult> =
        results.iter().filter(|r| r.load_error.is_some()).collect();
    let ns_with_failures: Vec<&NsTestResult> = results
        .iter()
        .filter(|r| r.load_error.is_none() && (r.fail > 0 || r.error > 0))
        .collect();

    println!();
    println!("══════════════════════════════════════════════════════════════");
    println!("Test Summary");
    println!("══════════════════════════════════════════════════════════════");
    println!(
        "Ran {} tests containing {} assertions across {} namespace(s) in {:.1}s.",
        total_tests,
        total_assertions,
        results.len(),
        elapsed.as_secs_f64()
    );
    println!(
        "{} passed, {} failed, {} errors.",
        total_pass, total_fail, total_error
    );

    if !load_errors.is_empty() {
        println!();
        println!(
            "── {} namespace(s) failed to load ──────────────────────────────",
            load_errors.len()
        );
        for r in &load_errors {
            println!("  {} — {}", r.ns, r.load_error.as_deref().unwrap_or("?"));
        }
    }

    if !ns_with_failures.is_empty() {
        println!();
        println!(
            "── {} namespace(s) with test failures ──────────────────────────",
            ns_with_failures.len()
        );
        for r in &ns_with_failures {
            println!("  {} — {} failures, {} errors", r.ns, r.fail, r.error);
        }
    }

    if load_errors.is_empty() && ns_with_failures.is_empty() {
        println!();
        println!("All tests passed.");
    }
    println!("══════════════════════════════════════════════════════════════");
}

/// Discover all namespace names from `.cljc` / `.cljrs` files in the given source paths.
fn discover_namespaces(src_paths: &[PathBuf]) -> Vec<String> {
    let mut namespaces = Vec::new();
    for dir in src_paths {
        if dir.is_dir() {
            discover_in_dir(dir, dir, &mut namespaces);
        }
    }
    namespaces.sort();
    namespaces
}

fn discover_in_dir(root: &PathBuf, dir: &PathBuf, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            discover_in_dir(root, &path, out);
        } else if let Some(ext) = path.extension()
            && (ext == "cljc" || ext == "cljrs")
            && let Some(ns) = file_to_namespace(root, &path)
        {
            out.push(ns);
        }
    }
}

/// Convert a file path relative to the source root into a Clojure namespace name.
/// e.g. `test/clojure/core_test/juxt.cljc` relative to `test/` → `clojure.core-test.juxt`
fn file_to_namespace(root: &PathBuf, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let stem = rel.with_extension(""); // remove .cljc / .cljrs
    let ns = stem
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, ".")
        .replace('_', "-");
    Some(ns)
}

// ── REPL ──────────────────────────────────────────────────────────────────────

fn run_repl(globals: Arc<GlobalEnv>) {
    println!("clojurust REPL (type :quit to exit)");
    println!();

    #[cfg(feature = "enable-rustyline")]
    let mut rl = rustyline::DefaultEditor::new().unwrap();

    let mut env = Env::new(globals, "user");

    let stdin = io::stdin();
    let mut input_buf = String::new();
    let mut depth: i32 = 0;

    #[cfg(feature = "enable-rustyline")]
    loop {
        let readline = rl.readline("=> ");
        match readline {
            Ok(line) => {
                rl.add_history_entry(line.as_str());
                if line.is_empty() {
                    continue;
                } else if line.starts_with(":quit") {
                    break;
                } else {
                    match eval_in(&mut env, &line, "<repl>") {
                        Ok(Value::Nil) => println!("nil"),
                        Ok(v) => println!("{}", v),
                        Err(e) => println!("error: {}", e),
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => break,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(err) => {
                eprintln!("error: {}", err);
                break;
            }
        }
    }

    #[cfg(not(feature = "enable-rustyline"))]
    loop {
        let prompt = if input_buf.is_empty() { "=> " } else { ".. " };
        print!("{}", prompt);
        io::stdout().flush().unwrap();

        let mut line = String::new();
        match stdin.lock().read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => {
                eprintln!("I/O error: {e}");
                break;
            }
        }

        let trimmed = line.trim_end();

        if input_buf.is_empty() && trimmed == ":quit" {
            break;
        }

        // Track paren depth to support multi-line input.
        for ch in trimmed.chars() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                _ => {}
            }
        }

        if !input_buf.is_empty() {
            input_buf.push('\n');
        }
        input_buf.push_str(trimmed);

        // Only evaluate when parens are balanced (or we have a bare atom).
        if depth <= 0 && !input_buf.trim().is_empty() {
            depth = 0;
            let src = std::mem::take(&mut input_buf);
            match eval_in(&mut env, &src, "<repl>") {
                Ok(Value::Nil) => {}
                Ok(v) => println!("{}", v),
                Err(e) => eprintln!("Error: {e}"),
            }
        }
    }

    println!("Bye.");
}
