//! Argument definitions, process setup, and subcommand dispatch.
//!
//! This module owns everything that is true of `cljrs` as a whole — the global
//! flags, the large-stack worker thread, the tracing subscriber, the
//! process-wide JIT and stats policy — and nothing that is specific to one
//! subcommand. Each subcommand's arguments and implementation live in its own
//! [`crate::commands`] module; [`Commands`] holds one variant per module and
//! `run_command` is a dispatcher with no logic of its own.

use clap::{Parser, Subcommand};
use miette::IntoDiagnostic as _;

use crate::commands;
use crate::session::{self, VersioningFlags};

/// Default thread stack size: 64 MiB.
const DEFAULT_STACK_SIZE: usize = 64 * 1024 * 1024;

/// clojurust — a Rust-hosted dialect of the Clojure programming language.
#[derive(Parser)]
#[command(name = "cljrs", version, about, long_about = None)]
pub struct Cli {
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

    /// Make pinned lookups of native (Rust-backed) functions an error when
    /// the package's recorded provenance does not match the requested
    /// commit (default: warn once per pin).  Can also be enabled
    /// per-project via `:enforce-native-versions true` in `cljrs.edn`.
    #[arg(long, global = true)]
    enforce_native_versions: bool,

    /// JIT compilation invocation threshold (default 1000; 0 = disable JIT).
    /// Also read from CLJRS_JIT_THRESHOLD env var.
    #[arg(
        long,
        global = true,
        value_name = "N",
        help = "JIT invocation threshold (0 to disable, default 1000)"
    )]
    jit_threshold: Option<u32>,

    /// Warm threshold: tree-walked calls before a function is lowered to IR
    /// in the background (default 50; 0 disables background lowering).
    /// Also read from CLJRS_IR_THRESHOLD env var.  Applies even when the
    /// JIT is disabled.
    #[arg(
        long,
        global = true,
        value_name = "N",
        help = "Background IR lowering threshold (0 to disable, default 50)"
    )]
    ir_threshold: Option<u32>,

    /// Feature-level logging flags: -X debug:gc,jit or -X trace:env
    ///
    /// Format: <level>:<feature1>,<feature2>,...
    /// Levels: debug, trace.  Features: gc, env, ir, jit.
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

    /// Print JIT specialization / inline-cache statistics on exit (Phase
    /// 10.6).  Pass a path to write them to a file; pass the flag without a
    /// value to write them to stdout.  Only the `run`, `eval`, and `test`
    /// subcommands honour this flag.
    #[arg(
        long = "jit-stats",
        global = true,
        value_name = "FILE",
        num_args = 0..=1,
        default_missing_value = "",
    )]
    jit_stats: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Interpret a .cljrs or .cljc source file.
    Run(commands::run::Args),
    /// Start an interactive REPL.
    Repl(commands::repl::Args),
    /// AOT-compile a source file or project to a native binary.
    ///
    /// When `cljrs.edn` is present in the working directory the compiler reads
    /// `:paths` (source directories), `:deps` (dependency source roots), and
    /// `:main` (entry-point namespace) from it automatically.  The `-main`
    /// function is discovered from `:paths` only (not from dependencies); an
    /// error is raised if more than one `-main` is found and neither `--main`
    /// nor `:main` resolves the ambiguity.
    Compile(commands::compile::Args),
    /// Evaluate a single Clojure expression and print the result.
    Eval(commands::eval::Args),
    /// Inspect, pre-lower, and visualize clojurust's intermediate representation.
    Ir {
        #[command(subcommand)]
        command: commands::ir::IrCommands,
    },
    /// Run clojure.test tests for one or more namespaces.
    ///
    /// If no namespaces are given, discovers and runs all test namespaces
    /// found in the source paths.
    Test(commands::test::Args),
    /// Manage project dependencies declared in cljrs.edn.
    ///
    /// Git dependencies are cached in ~/.cljrs/cache/git/.
    /// No network access occurs unless you run `cljrs deps fetch`.
    Deps {
        #[command(subcommand)]
        command: commands::deps::DepsCommands,
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
    /// cljrs_init_<crate>(registry: *mut cljrs_interop::Registry)` entry point.
    BuildNative(commands::build_native::Args),
    /// Run the clojurust language server (LSP) over stdio.
    ///
    /// Speaks the Language Server Protocol on stdin/stdout for editor
    /// integration, providing parse diagnostics and a document-symbol outline
    /// for `.cljrs` / `.cljc` files. Editors normally launch this for you.
    Lsp,
    /// Start an nREPL server for editor integration (CIDER, Calva, Conjure).
    ///
    /// Listens for bencode-encoded nREPL messages over TCP and writes the
    /// bound port to `.nrepl-port` in the current directory so clients can
    /// auto-connect. Runs until interrupted.
    Nrepl(commands::nrepl::Args),
}

/// Install the global tracing subscriber.
///
/// The default verbosity comes from `--debug`/`--trace`, with the noisy
/// codegen crates pinned to `warn` and the runtime's own feature targets
/// (`gc`, `env`, `ir`, `jit`) pinned off — see
/// [`cljrs_runtime::logging::base_filter`].  `RUST_LOG` (in `tracing`
/// target=level syntax, e.g. `RUST_LOG=info,cranelift_codegen=debug`) replaces
/// those defaults entirely, so the IR dumps are still reachable when wanted;
/// one that does not parse is reported and ignored, exactly as it is for an AOT
/// binary (see [`cljrs_runtime::logging::apply_rust_log`]).
///
/// `-X debug:gc,jit` is layered on last, so it survives a `RUST_LOG` that says
/// nothing about those targets.
fn init_tracing(cli: &Cli) -> miette::Result<()> {
    let default_level = if cli.trace {
        tracing::Level::TRACE
    } else if cli.debug {
        tracing::Level::DEBUG
    } else {
        tracing::Level::INFO
    };

    let mut filter =
        cljrs_runtime::logging::apply_rust_log(cljrs_runtime::logging::base_filter(default_level));

    for flag in &cli.x_flags {
        filter = cljrs_runtime::logging::apply_x_flag(filter, flag)
            .map_err(|e| miette::miette!("invalid -X flag: {e}"))?;
    }

    cljrs_runtime::logging::init(filter);
    Ok(())
}

/// The `cljrs` entry point: parse arguments, set up the process, and run the
/// requested subcommand on a large-stack thread.
pub fn main() -> miette::Result<()> {
    miette::set_hook(Box::new(|_| {
        Box::new(
            miette::MietteHandlerOpts::new()
                .terminal_links(true)
                .build(),
        )
    }))
    .into_diagnostic()?;

    let cli = Cli::parse();
    init_tracing(&cli)?;

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
                session::with_async_driver(|| run(cli))
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

    // Decide whether runtimes this process builds get a JIT tier.
    session::configure_jit(cli.jit_threshold);

    // Configure background IR lowering (Phase 10.7); independent of the JIT.
    match cli.ir_threshold {
        // 0 disables background lowering entirely (functions stay at
        // tree-walk unless eagerly lowered or pre-built).
        Some(0) => cljrs_runtime::tiered::set_ir_threshold(u32::MAX),
        Some(t) => cljrs_runtime::tiered::set_ir_threshold(t),
        None => {}
    }

    let gc_stats_target = cli.gc_stats.clone();
    let jit_stats_target = cli.jit_stats.clone();
    let versioning = VersioningFlags {
        verify_commit_signatures: cli.verify_commit_signatures,
        enforce_native_versions: cli.enforce_native_versions,
    };
    let supports_gc_stats = matches!(
        &cli.command,
        Commands::Run(..) | Commands::Eval(..) | Commands::Test(..),
    );

    let result = run_command(cli.command, versioning);

    if supports_gc_stats
        && let Some(target) = gc_stats_target.as_deref()
        && let Err(e) = write_gc_stats(target)
    {
        eprintln!("cljrs: failed to write GC stats: {e}");
    }
    if supports_gc_stats
        && let Some(target) = jit_stats_target.as_deref()
        && let Err(e) = write_jit_stats(target)
    {
        eprintln!("cljrs: failed to write JIT stats: {e}");
    }

    result
}

/// Hand each subcommand to the module that owns it.
fn run_command(command: Commands, versioning: VersioningFlags) -> miette::Result<i32> {
    match command {
        Commands::Run(args) => commands::run::run(args, versioning),
        Commands::Repl(args) => commands::repl::run(args, versioning),
        Commands::Compile(args) => commands::compile::run(args, versioning),
        Commands::Eval(args) => commands::eval::run(args, versioning),
        Commands::Ir { command } => commands::ir::run(command),
        Commands::Test(args) => commands::test::run(args, versioning),
        Commands::Deps { command } => commands::deps::run(command),
        Commands::BuildNative(args) => commands::build_native::run(args),
        Commands::Lsp => commands::lsp::run(),
        Commands::Nrepl(args) => commands::nrepl::run(args, versioning),
    }
}

/// Write a snapshot of the JIT specialization / inline-cache counters
/// (`cljrs_compiler::rt_abi::jit_stats`) to `target`.
///
/// An empty target (the flag was passed without a value) writes to stdout;
/// any other value is treated as a filesystem path.
fn write_jit_stats(target: &str) -> std::io::Result<()> {
    let snapshot = cljrs_compiler::rt_abi::jit_stats::snapshot();
    if target.is_empty() {
        println!("{snapshot}");
        Ok(())
    } else {
        std::fs::write(target, snapshot)
    }
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
