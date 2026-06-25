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
    /// `:rust/init` — fully-qualified path to the dep's native init function
    /// (e.g. `"my_crate::cljrs_init"`), when the dep ships Rust code.
    pub rust_init: Option<Arc<str>>,
    /// `:rust/crate` — directory of the dep's Cargo.toml relative to its
    /// repository root (defaults to the root).
    pub rust_crate_dir: Option<Arc<str>>,
    /// `:rust/load :dylib` — opt in to **pinned native code**: when a
    /// versioned symbol resolves into this dep's namespace and falls back to
    /// a native function, build the dep's crate at the pinned commit as a
    /// cdylib and load it instead of using the current binary's
    /// implementation.
    pub rust_load_dylib: bool,
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

/// A trusted commit signer declared in `:trusted-signers`.
///
/// Used (with `:verify-commit-signatures true`) to decide which keys are
/// allowed to sign versioned dependency commits. Each entry is either an
/// inline public key or a path to a key file resolved relative to the
/// `cljrs.edn` directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustedSigner {
    /// An inline public key: an armored PGP block or an OpenSSH public key line.
    Inline(String),
    /// A path to a public-key file (PGP `.asc` or OpenSSH `.pub`).
    File(PathBuf),
}

/// A single alias entry from the `:aliases` map.
#[derive(Debug, Clone, Default)]
pub struct Alias {
    pub extra_paths: Vec<PathBuf>,
    pub extra_deps: Vec<(Arc<str>, Dependency)>,
}

/// Rust-crate configuration for mixed Rust/Clojure projects.
///
/// The `:init` value is a Rust path like `"my_project::cljrs_init"`. The
/// first `::` segment is the crate name used in `Cargo.toml` and when
/// looking for the compiled shared library on disk.
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

impl RustConfig {
    /// Derive the Rust crate name from the init function path.
    ///
    /// `"my_project::cljrs_init"` → `Some("my_project")`.
    /// Returns `None` when `init_fn` is absent.
    pub fn crate_name(&self) -> Option<&str> {
        self.init_fn
            .as_deref()
            .map(|s| s.split("::").next().unwrap_or(s))
    }
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
    /// must carry a valid commit signature (verified natively against
    /// `trusted_signers`) before historical code is executed.  Equivalent to
    /// the `--verify-commit-signatures` CLI flag.
    pub verify_commit_signatures: bool,
    /// Public keys trusted to sign versioned dependency commits, from
    /// `:trusted-signers`.  Consulted only when `verify_commit_signatures` is
    /// on; an empty set means no commit can be verified.
    pub trusted_signers: Vec<TrustedSigner>,
    /// When true, a pinned lookup of a native (Rust-backed) function whose
    /// recorded provenance does not match the requested commit is an error
    /// instead of a warning.  Equivalent to the `--enforce-native-versions`
    /// CLI flag.
    pub enforce_native_versions: bool,
    /// Optional Rust-crate configuration for mixed Rust/Clojure projects.
    pub rust: Option<RustConfig>,
    /// The namespace containing `-main`, used as the AOT entry point.
    /// Set via `:main` in `cljrs.edn` or overridden by `--main` on the CLI.
    pub main_ns: Option<Arc<str>>,
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
