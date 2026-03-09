use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;

use clap::{Parser, Subcommand};
use miette::IntoDiagnostic as _;

use cljx_eval::{Env, EvalError, eval};
use cljx_stdlib::{standard_env, standard_env_with_paths};
use cljx_value::Value;

/// clojurust — a Rust-hosted dialect of the Clojure programming language.
#[derive(Parser)]
#[command(name = "cljx", version, about, long_about = None)]
struct Cli {
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
    },
    /// Start an interactive REPL.
    Repl {
        /// Source directories to search when resolving `require`.
        #[arg(long = "src-path", value_name = "DIR")]
        src_paths: Vec<PathBuf>,
    },
    /// AOT-compile a source file to a native binary.
    Compile {
        /// Path to the source file.
        file: PathBuf,
        /// Output binary path.
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Evaluate a single Clojure expression and print the result.
    Eval {
        /// The expression to evaluate.
        expr: String,
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
    },
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
    run(cli)
}

fn run(cli: Cli) -> miette::Result<()> {
    match cli.command {
        Commands::Run { file, src_paths } => {
            let src = std::fs::read_to_string(&file)
                .map_err(|e| miette::miette!("{}: {}", file.display(), e))?;
            let filename = file.display().to_string();
            run_source(&src, &filename, src_paths)?;
        }
        Commands::Repl { src_paths } => {
            run_repl(src_paths);
        }
        Commands::Compile { file, out } => {
            eprintln!(
                "[not yet implemented] cljx compile {} -o {}",
                file.display(),
                out.display()
            );
        }
        Commands::Eval { expr } => {
            let result = eval_source(&expr, "<eval>")?;
            // Printing/comparing may realize lazy seqs that panic; catch it.
            let prev_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(|_| {})); // suppress panic message
            let print_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                if result != Value::Nil {
                    println!("{}", result);
                }
            }));
            std::panic::set_hook(prev_hook);
            if let Err(e) = print_result {
                let msg = if let Some(s) = e.downcast_ref::<String>() {
                    s.clone()
                } else if let Some(s) = e.downcast_ref::<&str>() {
                    s.to_string()
                } else {
                    "error printing result".to_string()
                };
                return Err(miette::miette!("{}", msg));
            }
        }
        Commands::Test {
            namespaces,
            src_paths,
            verbose,
        } => {
            run_tests_command(namespaces, src_paths, verbose)?;
        }
    }
    Ok(())
}

// ── Source evaluation ─────────────────────────────────────────────────────────

/// Evaluate all forms in `src`, printing nothing. Returns the last value.
fn eval_source(src: &str, filename: &str) -> miette::Result<Value> {
    let globals = standard_env();
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)
}

/// Run a source file: evaluate all top-level forms, print nothing on success.
fn run_source(src: &str, filename: &str, src_paths: Vec<PathBuf>) -> miette::Result<()> {
    let globals = standard_env_with_paths(src_paths);
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)?;
    Ok(())
}

/// Evaluate `src` in an existing `Env`. Returns the last value.
fn eval_in(env: &mut Env, src: &str, filename: &str) -> miette::Result<Value> {
    let mut parser = cljx_reader::Parser::new(src.to_string(), filename.to_string());
    let forms = parser.parse_all().map_err(miette::Report::from)?;

    let mut result = Value::Nil;
    for form in forms {
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
    }
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
) -> miette::Result<()> {
    let namespaces = if namespaces.is_empty() {
        let discovered = discover_namespaces(&src_paths);
        if discovered.is_empty() {
            eprintln!("cljx test: no test namespaces found in source paths");
            std::process::exit(2);
        }
        eprintln!("Discovered {} test namespace(s).\n", discovered.len());
        discovered
    } else {
        namespaces
    };

    let globals = standard_env_with_paths(src_paths);
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
        results.push(result);
    }

    let elapsed = start.elapsed();

    // Print summary.
    print_summary(&results, elapsed);

    let total_fail: i64 = results.iter().map(|r| r.fail + r.error).sum();
    let total_load_errors: usize = results.iter().filter(|r| r.load_error.is_some()).count();

    if total_fail > 0 || total_load_errors > 0 {
        std::process::exit(1);
    }
    Ok(())
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

fn run_repl(src_paths: Vec<PathBuf>) {
    println!("clojurust REPL (type :quit to exit)");
    println!();

    let globals = standard_env_with_paths(src_paths);
    let mut env = Env::new(globals, "user");

    let stdin = io::stdin();
    let mut input_buf = String::new();
    let mut depth: i32 = 0;

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
