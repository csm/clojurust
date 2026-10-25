//! The runtime session a subcommand evaluates in: how it is built, and how
//! source is run inside it.
//!
//! Everything here is shared by more than one subcommand. `run`, `repl`,
//! `eval`, `test`, and `nrepl` all build their environment with
//! [`setup_globals`] and evaluate through [`eval_in`] / [`eval_form`], so the
//! `cljrs.edn` wiring, the JIT policy, and the async driver are identical no
//! matter which one the user typed.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_gc::GcConfig;
use cljrs_runtime::tiered::{Env, EvalError, GlobalEnv, eval};
use cljrs_value::Value;

use crate::native;

/// Build GC config from CLI flags, or use defaults if not specified.
///
/// The limits apply to each isolate heap as a collection trigger.  The soft
/// limit is only that per-heap trigger; it does not reach the process-wide
/// memory governor (`cljrs_gc::governor`), whose soft limit would otherwise
/// sit at Yellow whenever several isolates each approach it.  A hard limit
/// also sets the governor's budget (soft limit 75% of it).  That budget is
/// enforced only in strict mode (`CLJRS_MEMORY_STRICT=1`, which a hard limit
/// keeps).  Without a hard limit the governor keeps its `CLJRS_MEMORY_*` or
/// platform defaults.  A zero hard limit or a soft limit above the hard limit is
/// rejected.
pub fn build_gc_config(
    soft_limit_mb: Option<usize>,
    hard_limit_mb: Option<usize>,
) -> miette::Result<Arc<GcConfig>> {
    let (config, memory) = gc_limits(soft_limit_mb, hard_limit_mb)?;
    if let Some(mut memory) = memory {
        // The flag came from the environment; the CLI limits replace only
        // the budget.
        memory.strict = cljrs_gc::governor().config().strict;
        cljrs_gc::governor()
            .configure(memory)
            .map_err(|e| miette::miette!("invalid GC limits: {e}"))?;
    }
    Ok(Arc::new(config))
}

/// Validate CLI limits into a per-heap config and, when a hard limit was
/// given, a process governor config.  Pure: does not touch the global governor.
fn gc_limits(
    soft_limit_mb: Option<usize>,
    hard_limit_mb: Option<usize>,
) -> miette::Result<(GcConfig, Option<cljrs_gc::MemoryConfig>)> {
    let mb = |n: usize| n.saturating_mul(1024 * 1024);
    let invalid = |e: cljrs_gc::MemoryConfigError| miette::miette!("invalid GC limits: {e}");
    let config = match (soft_limit_mb, hard_limit_mb) {
        (Some(soft), Some(hard)) => {
            GcConfig::try_with_limits(mb(soft), mb(hard)).map_err(invalid)?
        }
        (Some(soft), None) => GcConfig::with_soft_limit(mb(soft)),
        (None, Some(hard)) => {
            let config = GcConfig::with_hard_limit(mb(hard));
            config.validate().map_err(invalid)?;
            config
        }
        (None, None) => return Ok((GcConfig::new(), None)),
    };
    let memory = hard_limit_mb
        .map(|hard| cljrs_gc::MemoryConfig::from_optional_limits(None, Some(mb(hard))))
        .transpose()
        .map_err(invalid)?;
    Ok((config, memory))
}

/// CLI-level versioned-symbol policy flags, threaded into `setup_globals`.
#[derive(Clone, Copy, Default)]
pub struct VersioningFlags {
    /// `--verify-commit-signatures` / `:verify-commit-signatures`.
    pub verify_commit_signatures: bool,
    /// `--enforce-native-versions` / `:enforce-native-versions`.
    pub enforce_native_versions: bool,
}

/// Whether runtimes built by this process get a JIT tier attached.
static JIT_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Decide the JIT policy from CLI flags and env vars.
///
/// JIT is enabled by default; disable with `CLJRS_NO_JIT=1` or `--jit-threshold 0`.
/// The backend itself is attached per runtime, in [`setup_globals`].
pub fn configure_jit(threshold: Option<u32>) {
    if std::env::var("CLJRS_NO_JIT").is_ok() {
        return;
    }
    if let Some(0) = threshold {
        return; // Explicitly disabled via --jit-threshold 0.
    }
    if let Some(t) = threshold {
        cljrs_runtime::tiered::jit_state::set_jit_threshold(t);
    }
    JIT_ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Create a fully initialised `GlobalEnv` with stdlib, user source paths, GC
/// config, and any `cljrs.edn` found in the current working directory.
///
/// Paths declared in `:paths` of `cljrs.edn` are appended to `src_paths` (CLI
/// flags take precedence).  The parsed `DepsConfig` is stored in
/// `GlobalEnv.deps_config` so that versioned symbol resolution and the
/// `deps fetch`/`deps status` commands share the same config object.
pub fn setup_globals(
    src_paths: Vec<PathBuf>,
    gc_config: Arc<GcConfig>,
    versioning: VersioningFlags,
) -> Arc<GlobalEnv> {
    let runtime = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::Tiered)
        .source_paths(src_paths)
        .gc_config(gc_config)
        .build()
        .unwrap_or_else(|e| {
            eprintln!("failed to start the runtime: {e}");
            std::process::exit(1);
        });
    // Attach the JIT tier to this runtime (unless disabled on the command
    // line or by `CLJRS_NO_JIT`); nothing has been lowered yet, so no promotion
    // can be missed.
    if JIT_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
        cljrs_compiler::jit::install(&runtime);
    }
    cljrs_stdlib::install(&runtime);
    let globals = runtime.into_globals();
    if versioning.verify_commit_signatures {
        globals
            .verify_commit_signatures
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if versioning.enforce_native_versions {
        globals.set_enforce_native_versions(true);
    }
    // Opt-in pinned native packages (:rust/load :dylib in cljrs.edn).
    native::pinned::install(&globals);
    if let Ok(cwd) = std::env::current_dir() {
        apply_deps_config(&globals, &cwd);
    }
    // Initialise async inside the LocalSet so `cljrs_async::init` can spawn its
    // background GC-service task (it calls `spawn_local`, which requires a
    // LocalSet context). The task persists on the LocalSet and is serviced by
    // each subsequent per-form drive in `eval_form`.
    #[cfg(feature = "async")]
    ASYNC_DRIVER.with(|d| {
        let guard = d.borrow();
        let init = |g: &Arc<GlobalEnv>| {
            cljrs_async::init(g);
            cljrs_io::init(g);
            #[cfg(feature = "net")]
            cljrs_net::init(g);
            #[cfg(feature = "charset")]
            cljrs_charset::init(g);
        };
        match guard.as_ref() {
            Some(drv) => drv.local.block_on(&drv.rt, async { init(&globals) }),
            None => init(&globals),
        }
    });
    #[cfg(feature = "base64")]
    cljrs_base64::init(&globals);
    globals
}

/// Load the nearest `cljrs.edn` and wire its data into `globals`.
///
/// Silently does nothing when no config file is found; prints a warning to
/// stderr when the file exists but cannot be parsed.
fn apply_deps_config(globals: &Arc<GlobalEnv>, cwd: &Path) {
    match cljrs_project::config::load_config(cwd) {
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
            // Append each dependency's own source roots so a plain `require` of
            // a dep's namespace resolves (git deps are materialized from the
            // local cache — run `cljrs deps fetch` first; no network here).
            add_dep_source_paths(globals, &config);
            if config.verify_commit_signatures {
                globals
                    .verify_commit_signatures
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            globals.load_trusted_signers(&config);
            if config.enforce_native_versions {
                globals.set_enforce_native_versions(true);
            }
            // Load the native shared library (if :rust is configured) so that
            // native functions are registered before any Clojure code runs.
            if let Some(rust_config) = &config.rust {
                native::load_project_lib(rust_config, globals);
            }
            *globals.deps_config.write().unwrap() = Some(Arc::new(config));
        }
        Ok(None) => {}
        Err(e) => eprintln!("cljrs: warning: could not load cljrs.edn: {e}"),
    }
}

/// Append each declared dependency's own source roots to `globals.source_paths`
/// so a plain `(require '[dep.ns :as …])` resolves namespaces provided by a
/// dependency rather than only by the running binary or the local project.
///
/// - **Local deps** (`:local/root`) contribute paths from the directory on disk.
/// - **Git deps** are materialized from the local bare cache at their pinned
///   `:git/sha` (no network — `cljrs deps fetch` must have populated it).  A
///   missing cache warns and is skipped rather than aborting the run.
///
/// Each dep's roots come from its own `cljrs.edn` `:paths` (resolved relative to
/// the dep root), defaulting to `src/`.  Only directories that actually exist
/// are added; pure-native deps (no Clojure source) contribute nothing here and
/// are brought in by the native-`require` loader instead.
fn add_dep_source_paths(globals: &Arc<GlobalEnv>, config: &cljrs_project::config::DepsConfig) {
    for (name, dep) in &config.deps {
        let root = match dep {
            cljrs_project::config::Dependency::Local { root, .. } => {
                if root.is_dir() {
                    root.clone()
                } else {
                    eprintln!(
                        "cljrs: warning: local dep {name} not found at {}",
                        root.display()
                    );
                    continue;
                }
            }
            cljrs_project::config::Dependency::Git(git) => {
                match cljrs_project::vcs::worktree_at_commit(&git.url, &git.sha) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "cljrs: warning: git dep {name} ({}) is not available ({e}); \
                             run `cljrs deps fetch`",
                            git.url
                        );
                        continue;
                    }
                }
            }
        };
        let mut paths = globals.source_paths.write().unwrap();
        for p in dep_source_paths(&root) {
            if p.is_dir() && !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
}

/// Collect source paths contributed by every declared dependency in `config`.
///
/// Git deps are resolved from the local cache (run `cljrs deps fetch` first);
/// missing deps emit a warning and are skipped.
pub fn collect_dep_src_paths(config: &cljrs_project::config::DepsConfig) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for (name, dep) in &config.deps {
        let root = match dep {
            cljrs_project::config::Dependency::Local { root, .. } => {
                if root.is_dir() {
                    root.clone()
                } else {
                    eprintln!(
                        "cljrs: warning: local dep {name} not found at {}",
                        root.display()
                    );
                    continue;
                }
            }
            cljrs_project::config::Dependency::Git(git) => {
                match cljrs_project::vcs::worktree_at_commit(&git.url, &git.sha) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!(
                            "cljrs: warning: git dep {name} ({}) is not available ({e}); \
                             run `cljrs deps fetch`",
                            git.url
                        );
                        continue;
                    }
                }
            }
        };
        for p in dep_source_paths(&root) {
            if p.is_dir() && !paths.contains(&p) {
                paths.push(p);
            }
        }
    }
    paths
}

/// The source roots a dependency at `root` contributes: its `cljrs.edn`
/// `:paths` (already resolved to absolute paths relative to the dep root) if a
/// parseable config is present, else the conventional `<root>/src`.
fn dep_source_paths(root: &Path) -> Vec<PathBuf> {
    let cfg_path = root.join("cljrs.edn");
    if cfg_path.exists()
        && let Ok(src) = std::fs::read_to_string(&cfg_path)
        && let Ok(parsed) = cljrs_project::config::parse_config(&src, &cfg_path)
        && !parsed.paths.is_empty()
    {
        return parsed.paths;
    }
    vec![root.join("src")]
}

/// Evaluate all forms in `src`, printing nothing. Returns the last value.
/// Convert a file path relative to the source root into a Clojure namespace name.
/// e.g. `test/clojure/core_test/juxt.cljc` relative to `test/` → `clojure.core-test.juxt`
pub fn file_to_namespace(root: &PathBuf, file: &Path) -> Option<String> {
    let rel = file.strip_prefix(root).ok()?;
    let stem = rel.with_extension(""); // remove .cljc / .cljrs
    let ns = stem
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, ".")
        .replace('_', "-");
    Some(ns)
}

pub fn eval_source(src: &str, filename: &str, globals: Arc<GlobalEnv>) -> miette::Result<Value> {
    let mut env = Env::new(globals, "user");
    eval_in(&mut env, src, filename)
}

/// Run a source file: evaluate all top-level forms, then call `-main` if defined.
pub fn run_source(
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
    let result = eval_in(env, &call, "<main>")?;
    // An `^:async` `-main` returns a `Future` immediately, with its body
    // queued as a task on the shared `LocalSet`. Await that future so the body
    // (and anything it spawns) runs to completion before the process exits;
    // for a synchronous `-main` this is a no-op pass-through.
    await_main_result(result)?;
    Ok(())
}

/// Drive `value` to a settled value on the shared async `LocalSet`. If `value`
/// is a `Future`/`Promise` (e.g. the result of an `^:async` `-main`), this
/// yields until it resolves; any other value is returned unchanged.
#[cfg(feature = "async")]
fn await_main_result(value: Value) -> miette::Result<()> {
    ASYNC_DRIVER.with(|d| {
        let guard = d.borrow();
        match guard.as_ref() {
            Some(drv) => {
                drv.local
                    .block_on(&drv.rt, cljrs_async::eval_async::await_value(value))
                    .map_err(format_eval_error)?;
                Ok(())
            }
            // No driver installed: nothing to await against, so leave as-is.
            None => Ok(()),
        }
    })
}

#[cfg(not(feature = "async"))]
fn await_main_result(_value: Value) -> miette::Result<()> {
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
pub fn eval_in(env: &mut Env, src: &str, filename: &str) -> miette::Result<Value> {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), filename.to_string());
    let forms = parser.parse_all().map_err(miette::Report::from)?;

    let mut result = Value::Nil;
    for form in forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        result = eval_form(&form, env).map_err(format_eval_error)?;
    }
    Ok(result)
}

/// The Tokio runtime + `LocalSet` that drive async evaluation, installed once
/// by [`with_async_driver`] and reused for the lifetime of the process.
#[cfg(feature = "async")]
struct AsyncDriver {
    rt: tokio::runtime::Runtime,
    local: tokio::task::LocalSet,
}

#[cfg(feature = "async")]
thread_local! {
    static ASYNC_DRIVER: std::cell::RefCell<Option<AsyncDriver>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f` with an async driver installed on this thread.
///
/// Builds the single-threaded runtime + `LocalSet` that every async task
/// (core.async producers, `^:async` calls, `cljrs-io` readers) runs on, and
/// stashes it so [`eval_form`] can drive each top-level form on it.  We
/// deliberately do *not* wrap `f` in a single `block_on`/`run_until`:
/// top-level evaluation is synchronous (it reads stdin, dispatches commands),
/// and each form is driven individually via `LocalSet::block_on`, which would
/// panic if nested inside an outer `block_on`.
#[cfg(feature = "async")]
pub fn with_async_driver<T>(f: impl FnOnce() -> T) -> T {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("failed to build Tokio runtime");
    let local = tokio::task::LocalSet::new();
    ASYNC_DRIVER.with(|d| *d.borrow_mut() = Some(AsyncDriver { rt, local }));
    let result = f();
    ASYNC_DRIVER.with(|d| *d.borrow_mut() = None);
    result
}

/// Evaluate a single top-level form.
///
/// With the `async` feature, the form is driven on the shared `LocalSet` via
/// [`tokio::task::LocalSet::block_on`], so spawned tasks — core.async channel
/// producers, `^:async` calls, the `cljrs-io` readers/writers — make progress
/// and a top-level `await` resolves. Tasks that outlive a form (e.g. a producer
/// feeding a channel `def`d in one REPL line and consumed in the next) stay
/// queued on the `LocalSet` and continue on the next form's drive. Without the
/// feature it is a plain synchronous `eval`.
#[cfg(feature = "async")]
#[allow(clippy::result_large_err)]
pub fn eval_form(form: &cljrs_reader::Form, env: &mut Env) -> Result<Value, EvalError> {
    ASYNC_DRIVER.with(|d| {
        let guard = d.borrow();
        match guard.as_ref() {
            Some(drv) => drv
                .local
                .block_on(&drv.rt, cljrs_async::eval_async::eval_async(form, env)),
            // No driver installed (shouldn't happen once `main` runs): fall back
            // to a synchronous evaluation rather than panicking.
            None => eval(form, env),
        }
    })
}

#[cfg(not(feature = "async"))]
#[allow(clippy::result_large_err)]
pub fn eval_form(form: &cljrs_reader::Form, env: &mut Env) -> Result<Value, EvalError> {
    eval(form, env)
}

pub fn format_eval_error(e: EvalError) -> miette::Report {
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
        EvalError::GasExhausted => miette::miette!("gas exhausted"),
        EvalError::MemoryLimitExceeded(fault) => miette::miette!("{fault}"),
        EvalError::ForbiddenEffect(operation) => {
            miette::miette!("effect forbidden in transaction function: {operation}")
        }
        EvalError::Read(e) => miette::Report::from(e),
        EvalError::Recur(_) => miette::miette!("recur outside of loop/fn"),
        EvalError::CommitSignatureVerificationFailed { commit, reason } => {
            miette::miette!("commit {commit:?} failed signature verification: {reason}")
        }
    }
}

#[cfg(test)]
mod gc_config_tests {
    // These call `gc_limits`, not `build_gc_config`, so they never reconfigure
    // the process-global governor that other tests in this binary may read.
    use super::gc_limits;

    const MB: usize = 1024 * 1024;

    #[test]
    fn soft_only_sets_the_soft_limit() {
        let (config, memory) = gc_limits(Some(64), None).unwrap();
        assert_eq!(config.soft_limit(), 64 * MB);
        assert!(config.hard_limit() >= 64 * MB);
        assert!(
            memory.is_none(),
            "the soft limit does not configure the governor"
        );
    }

    #[test]
    fn both_limits_are_kept() {
        let (config, memory) = gc_limits(Some(64), Some(128)).unwrap();
        assert_eq!(config.soft_limit(), 64 * MB);
        assert_eq!(config.hard_limit(), 128 * MB);
        let memory = memory.expect("a hard limit configures the governor");
        assert_eq!(memory.hard_limit, 128 * MB);
        assert_eq!(
            memory.soft_limit,
            96 * MB,
            "governor soft limit is 75% of hard"
        );
    }

    #[test]
    fn soft_above_hard_is_rejected() {
        assert!(gc_limits(Some(256), Some(128)).is_err());
    }

    #[test]
    fn zero_hard_limit_is_rejected() {
        assert!(gc_limits(None, Some(0)).is_err());
        assert!(gc_limits(Some(0), Some(0)).is_err());
    }
}
