//! `cljrs.edn` project configuration: types, parser, and discovery.
//!
//! # Format
//!
//! ```edn
//! {:paths ["src" "resources"]
//!
//!  :deps
//!  {my.lib      {:git/url "https://github.com/user/my-lib" :git/sha "abc1234ef"}
//!   local.utils {:local/root "../local-utils"}}
//!
//!  :aliases
//!  {:dev  {:extra-paths ["dev"]}
//!   :test {:extra-paths ["test"]
//!          :extra-deps  {test-tools {:git/url "..." :git/sha "..."}}}}}
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use thiserror::Error;

mod parse;
pub use parse::parse_config;

// ── Error ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Error)]
pub enum DepsError {
    #[error("could not read cljrs.edn: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error in cljrs.edn: {0}")]
    Parse(String),
}

pub type DepsResult<T> = Result<T, DepsError>;

// ── Types ─────────────────────────────────────────────────────────────────────

/// A git-hosted dependency with a pinned commit SHA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDep {
    pub url: Arc<str>,
    pub sha: Arc<str>,
}

/// A dependency declared in `:deps` or `:extra-deps`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dependency {
    Git(GitDep),
    /// A local directory on disk, resolved relative to the `cljrs.edn` file.
    Local {
        root: PathBuf,
    },
}

/// A single alias entry from the `:aliases` map.
#[derive(Debug, Clone, Default)]
pub struct Alias {
    pub extra_paths: Vec<PathBuf>,
    pub extra_deps: Vec<(Arc<str>, Dependency)>,
}

/// Rust-crate configuration for mixed Rust/Clojure projects.
///
/// Parsed from the `:rust` key in `cljrs.edn`:
///
/// ```edn
/// :rust {:crate "."                        ; path to directory with Cargo.toml
///        :init  "my_project::cljrs_init"}  ; optional hook-registration fn
/// ```
#[derive(Debug, Clone)]
pub struct RustConfig {
    /// Directory containing the user's `Cargo.toml`, resolved relative to the
    /// `cljrs.edn` file.  Defaults to the `cljrs.edn` directory (`"."`).
    pub crate_dir: PathBuf,
    /// Fully-qualified Rust path to the init function, e.g.
    /// `"my_project::cljrs_init"`.  When present, `cljrs compile` emits a
    /// call to this function in the generated `main.rs` before loading any
    /// Clojure source.  Omit if you rely solely on `#[cljrs::export]`
    /// inventory-based registration (future feature).
    pub init_fn: Option<Arc<str>>,
}

/// The fully parsed contents of a `cljrs.edn` file.
#[derive(Debug, Clone, Default)]
pub struct DepsConfig {
    /// Source/resource paths relative to the `cljrs.edn` file.
    pub paths: Vec<PathBuf>,
    /// Project-level dependency declarations.
    pub deps: Vec<(Arc<str>, Dependency)>,
    /// Named aliases (e.g. `:dev`, `:test`).
    pub aliases: Vec<(Arc<str>, Alias)>,
    /// When true, every versioned-symbol or versioned-namespace resolution
    /// must pass `git verify-commit` before historical code is executed.
    /// Equivalent to the `--verify-commit-signatures` CLI flag.
    pub verify_commit_signatures: bool,
    /// Optional Rust-crate configuration for mixed Rust/Clojure projects.
    pub rust: Option<RustConfig>,
}

impl DepsConfig {
    /// Find a dependency by namespace-prefix name.
    pub fn find_dep(&self, name: &str) -> Option<&Dependency> {
        self.deps
            .iter()
            .find(|(n, _)| n.as_ref() == name)
            .map(|(_, d)| d)
    }
}

// ── Discovery ─────────────────────────────────────────────────────────────────

/// Walk up the directory tree from `start`, returning the path of the first
/// `cljrs.edn` file found, or `None` if no config exists in any ancestor.
pub fn find_config_file(start: &Path) -> Option<PathBuf> {
    let mut dir: &Path = if start.is_file() {
        start.parent()?
    } else {
        start
    };
    loop {
        let candidate = dir.join("cljrs.edn");
        if candidate.exists() {
            return Some(candidate);
        }
        dir = dir.parent()?;
    }
}

/// Load and parse the `cljrs.edn` closest to `start`, returning `None` if
/// no config file is found.
pub fn load_config(start: &Path) -> DepsResult<Option<DepsConfig>> {
    match find_config_file(start) {
        None => Ok(None),
        Some(path) => {
            let src = std::fs::read_to_string(&path)?;
            let config = parse_config(&src, &path).map_err(DepsError::Parse)?;
            Ok(Some(config))
        }
    }
}
