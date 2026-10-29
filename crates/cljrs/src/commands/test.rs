//! `cljrs test` — run `clojure.test` namespaces from the source paths.

use std::path::PathBuf;
use std::time::Instant;

use cljrs_runtime::tiered::Env;
use cljrs_value::Value;

use crate::session::{self, VersioningFlags};

#[derive(clap::Args)]
pub struct Args {
    /// Namespaces to test (e.g. my.app.core-test).
    /// If omitted, all namespaces in --src-path are discovered.
    pub namespaces: Vec<String>,
    /// Source directories to search when resolving `require`.
    #[arg(long = "src-path", value_name = "DIR")]
    pub src_paths: Vec<PathBuf>,
    /// Print each passing assertion (helps identify which test hangs).
    #[arg(long, short)]
    pub verbose: bool,
    /// GC soft memory limit in MB (each isolate heap collects when it exceeds this).
    #[arg(long)]
    pub gc_soft_limit_mb: Option<usize>,
    /// Process managed-memory hard limit in MB (enforced only with CLJRS_MEMORY_STRICT=1).
    #[arg(long)]
    pub gc_hard_limit_mb: Option<usize>,
}

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

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    #[cfg(feature = "no-gc")]
    let _reads = cljrs_gc::region::ReadScope::enter();
    let gc_config = session::build_gc_config(args.gc_soft_limit_mb, args.gc_hard_limit_mb)?;
    let globals = session::setup_globals(args.src_paths, gc_config, versioning);

    let namespaces = if args.namespaces.is_empty() {
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
        args.namespaces
    };

    let mut env = Env::new(globals, "user");

    // Ensure clojure.test is loaded.
    session::eval_in(&mut env, "(require 'clojure.test)", "<test>")?;

    if args.verbose {
        session::eval_in(
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
        cljrs_runtime::tiered::force_collect(&env);
        cljrs_runtime::tiered::force_collect(&env);
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
    if let Err(e) = session::eval_in(env, &format!("(require '{ns})"), "<test>") {
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
    match session::eval_in(env, &format!("(clojure.test/run-tests '{ns})"), "<test>") {
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
            && let Some(ns) = session::file_to_namespace(root, &path)
        {
            out.push(ns);
        }
    }
}
