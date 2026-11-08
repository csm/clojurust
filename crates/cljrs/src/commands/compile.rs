//! `cljrs compile` — AOT-compile a source file or project to a native binary
//! or a WebAssembly module.

use std::path::{Path, PathBuf};

use clap::ValueEnum;

use crate::extensions;
use crate::session::{self, VersioningFlags};

/// Code-generation target for `cljrs compile`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
pub enum CompileTarget {
    /// Standalone binary via Cranelift.
    Native,
    /// WebAssembly module via the AOT wasm backend.
    Wasm,
}

#[derive(clap::Args)]
pub struct Args {
    /// Path to the source file.  Optional when `cljrs.edn` is present and
    /// the entry-point namespace can be determined from `--main`, `:main`
    /// in `cljrs.edn`, or auto-detection of a unique `-main` function.
    pub file: Option<PathBuf>,
    /// Output path (a native binary, or a `.wasm` module with `--target wasm`).
    #[arg(short, long)]
    pub out: PathBuf,
    /// Code-generation target: `native` (default) produces a standalone
    /// binary via Cranelift; `wasm` produces a WebAssembly module via the
    /// AOT wasm backend (the entry namespace's functions; the `"rt"` imports
    /// are satisfied by the runtime built for `wasm32-unknown-unknown`).
    #[arg(long, value_enum, default_value_t = CompileTarget::Native, value_name = "TARGET")]
    pub target: CompileTarget,
    /// Source directories to search when resolving `require`.
    #[arg(long = "src-path", value_name = "DIR")]
    pub src_paths: Vec<PathBuf>,
    /// Namespace containing the `-main` entry point (e.g. `my.app.core`).
    /// Overrides `:main` in `cljrs.edn` and auto-detection.
    #[arg(long = "main", value_name = "NS")]
    pub main_ns: Option<String>,
    /// Compile a test harness that runs all tests in the given file/directory.
    #[arg(long)]
    pub test: bool,
    /// Fail the build if the binary would embed readable Clojure source
    /// text (interpreted preambles, bundled namespaces).  Rejected with
    /// `--test` and `--target wasm`, which do not run the audit.
    #[arg(long = "require-fully-compiled")]
    pub require_fully_compiled: bool,
    /// GC soft memory limit in MB (each isolate heap collects when it exceeds this).
    #[arg(long)]
    pub gc_soft_limit_mb: Option<usize>,
    /// Process managed-memory hard limit in MB (enforced only with CLJRS_MEMORY_STRICT=1).
    #[arg(long)]
    pub gc_hard_limit_mb: Option<usize>,
}

pub fn run(args: Args, versioning: VersioningFlags) -> miette::Result<i32> {
    let Args {
        file,
        out,
        target,
        src_paths,
        main_ns,
        test,
        require_fully_compiled,
        gc_soft_limit_mb,
        gc_hard_limit_mb,
    } = args;

    let _gc_config = session::build_gc_config(gc_soft_limit_mb, gc_hard_limit_mb)?;

    // Load cljrs.edn; silently absent is fine.
    let deps_config = std::env::current_dir()
        .ok()
        .and_then(|cwd| cljrs_project::config::load_config(&cwd).ok().flatten());

    let rust_config = deps_config.as_ref().and_then(|c| c.rust.clone());

    // Build combined source paths: CLI flags first, then cljrs.edn :paths,
    // then each dependency's source roots.
    let mut all_src_paths = src_paths.clone();
    if let Some(ref config) = deps_config {
        for p in &config.paths {
            if !all_src_paths.contains(p) {
                all_src_paths.push(p.clone());
            }
        }
        for p in session::collect_dep_src_paths(config) {
            if !all_src_paths.contains(&p) {
                all_src_paths.push(p);
            }
        }
    }

    if test && target == CompileTarget::Wasm {
        return Err(miette::miette!(
            "--test is not supported with --target wasm yet"
        ));
    }
    let opacity =
        resolve_opacity_policy(require_fully_compiled, test).map_err(|e| miette::miette!("{e}"))?;

    if test {
        let test_dir = file.as_deref().unwrap_or_else(|| std::path::Path::new("."));
        cljrs_compiler::aot::compile_test_harness(test_dir, &out, &all_src_paths)
            .map_err(|e| miette::miette!("{e}"))?;
    } else {
        let entry_file = match file {
            Some(f) => f,
            None => {
                // Project mode: determine the entry namespace and locate its file.
                let project_paths: Vec<PathBuf> = deps_config
                    .as_ref()
                    .map(|c| c.paths.clone())
                    .unwrap_or_default();

                let effective_ns = if let Some(ns) = main_ns {
                    // --main CLI flag takes top priority.
                    ns
                } else if let Some(ns) = deps_config.as_ref().and_then(|c| c.main_ns.as_deref()) {
                    // :main in cljrs.edn is second priority.
                    ns.to_string()
                } else {
                    // Auto-detect from project :paths only (not dep paths).
                    if project_paths.is_empty() {
                        return Err(miette::miette!(
                            "no source paths: add :paths to cljrs.edn or use --src-path"
                        ));
                    }
                    let mains = find_main_namespaces(&project_paths);
                    match mains.len() {
                        0 => {
                            return Err(miette::miette!(
                                "no -main function found in source paths; \
                                     specify --main or add :main to cljrs.edn"
                            ));
                        }
                        1 => mains.into_iter().next().unwrap().0,
                        _ => {
                            let list = mains
                                .iter()
                                .map(|(ns, _)| ns.as_str())
                                .collect::<Vec<_>>()
                                .join(", ");
                            return Err(miette::miette!(
                                "multiple -main functions found ({list}); \
                                         specify --main or add :main to cljrs.edn"
                            ));
                        }
                    }
                };

                ns_to_file(&effective_ns, &all_src_paths).ok_or_else(|| {
                    miette::miette!("could not find source file for namespace {effective_ns}")
                })?
            }
        };

        // The compiler does not pick extensions; this build's feature
        // set does, exactly as it does for an interpreted run.
        let compile_session = cljrs_compiler::extensions::CompileSession::new(
            all_src_paths.clone(),
            extensions::default_set(),
        )
        .rust_config(rust_config.clone())
        .verify_commit_signatures(versioning.verify_commit_signatures)
        .opacity(opacity);

        if target == CompileTarget::Wasm {
            cljrs_compiler::aot::compile_file_to_wasm(&entry_file, &out, &compile_session)
                .map_err(|e| miette::miette!("{e}"))?;
        } else {
            cljrs_compiler::aot::compile_file(&entry_file, &out, &compile_session)
                .map_err(|e| miette::miette!("{e}"))?;
        }
    }
    Ok(0)
}

/// Resolve the opacity policy a `compile` invocation runs under.
///
/// Both backends audit: `compile_file` for embedded source, and
/// `compile_file_to_wasm` for units the module would omit.  `--test` does not:
/// `compile_test_harness` bundles every test namespace as interpreted source
/// unconditionally, so a strict policy there could never be satisfied and the
/// combination is refused rather than left to fail confusingly.
fn resolve_opacity_policy(
    require_fully_compiled: bool,
    test: bool,
) -> Result<cljrs_compiler::aot::OpacityPolicy, String> {
    if !require_fully_compiled {
        return Ok(cljrs_compiler::aot::OpacityPolicy::Report);
    }
    if test {
        return Err("--require-fully-compiled is not supported with --test: \
                    the test harness is not audited for embedded source"
            .to_string());
    }
    Ok(cljrs_compiler::aot::OpacityPolicy::RequireFullyCompiled)
}

/// Scan `src_paths` for source files that define a top-level `-main` function.
///
/// Returns a list of `(namespace_name, file_path)` pairs, one per file that
/// contains `(defn -main ...)` or `(defn- -main ...)` at the top level.
/// Only looks at the reader-level form structure (no evaluation).
fn find_main_namespaces(src_paths: &[PathBuf]) -> Vec<(String, PathBuf)> {
    use cljrs_reader::form::FormKind;

    let mut results = Vec::new();
    for dir in src_paths {
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_source_files(dir, &mut files);
        for file in files {
            let Ok(src) = std::fs::read_to_string(&file) else {
                continue;
            };
            let mut parser = cljrs_reader::Parser::new(src, file.display().to_string());
            let Ok(forms) = parser.parse_all() else {
                continue;
            };
            let mut ns_name: Option<String> = None;
            let mut has_main = false;
            for form in &forms {
                if let FormKind::List(parts) = &form.kind
                    && let Some(head) = parts.first()
                    && let FormKind::Symbol(s) = &head.kind
                {
                    if s == "ns" {
                        if let Some(second) = parts.get(1)
                            && let FormKind::Symbol(n) = &second.kind
                        {
                            ns_name = Some(n.clone());
                        }
                    } else if (s == "defn" || s == "defn-")
                        && parts.len() >= 2
                        && let FormKind::Symbol(n) = &parts[1].kind
                        && n == "-main"
                    {
                        has_main = true;
                    }
                }
            }
            if has_main {
                let ns = ns_name.unwrap_or_else(|| {
                    // Fall back to file-based namespace name if no (ns ...) form.
                    session::file_to_namespace(dir, &file).unwrap_or_default()
                });
                results.push((ns, file));
            }
        }
    }
    results
}

/// Recursively collect `.cljrs` and `.cljc` files under `dir`.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_source_files(&path, out);
        } else if let Some(ext) = path.extension()
            && (ext == "cljrs" || ext == "cljc")
        {
            out.push(path);
        }
    }
}

/// Translate a Clojure namespace name into a source file path by searching
/// `src_paths` for `<ns/with/slashes>.cljrs` or `.cljc`.
fn ns_to_file(ns: &str, src_paths: &[PathBuf]) -> Option<PathBuf> {
    let rel: String = ns.replace('.', "/").replace('-', "_");
    for dir in src_paths {
        for ext in &["cljrs", "cljc"] {
            let path = dir.join(format!("{rel}.{ext}"));
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::resolve_opacity_policy;
    use cljrs_compiler::aot::OpacityPolicy;

    /// Without the flag the policy is the reporting default.
    #[test]
    fn absent_flag_always_reports() {
        for test in [false, true] {
            assert_eq!(
                resolve_opacity_policy(false, test),
                Ok(OpacityPolicy::Report),
                "test={test}"
            );
        }
    }

    #[test]
    fn flag_selects_the_strict_policy_on_the_audited_path() {
        assert_eq!(
            resolve_opacity_policy(true, false),
            Ok(OpacityPolicy::RequireFullyCompiled)
        );
    }

    /// `--test` rejects rather than silently ignoring the flag.
    #[test]
    fn flag_is_rejected_under_test() {
        let err = resolve_opacity_policy(true, true).expect_err("--test must be rejected");
        assert!(
            err.contains("--require-fully-compiled") && err.contains("--test"),
            "error names both flags: {err}"
        );
    }
}
