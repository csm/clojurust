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
    /// The discovered config (or its directory) belongs to another user and
    /// its project root has not been trusted. See [`load_config`].
    #[error(
        "ignoring {}: {what} is owned by another user (uid {owner}); \
         run `cljrs trust {}` if you trust this project",
        .path.display(),
        .root.display()
    )]
    ForeignOwner {
        /// The `cljrs.edn` that was found.
        path: PathBuf,
        /// The directory holding it — the project root to trust.
        root: PathBuf,
        /// Which of the two failed the check: `"the file"` or `"its directory"`.
        what: &'static str,
        /// The uid that owns it.
        owner: u32,
    },
}

pub type DepsResult<T> = Result<T, DepsError>;

// ── Types ─────────────────────────────────────────────────────────────────────

/// A git-hosted dependency with a pinned commit SHA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitDep {
    pub url: Arc<str>,
    pub sha: Arc<str>,
    /// `:rust/init` — fully-qualified path to the dep's native init function
    /// (e.g. `"my_crate::cljrs_init_my_crate"`), when the dep ships Rust code.
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
        /// `:rust/init` — fully-qualified path to the dep's native init
        /// function, when the dep ships Rust code.
        rust_init: Option<Arc<str>>,
        /// `:rust/crate` — directory of the dep's Cargo.toml relative to
        /// `root` (defaults to `root` itself).
        rust_crate_dir: Option<Arc<str>>,
        /// `:rust/load :dylib` — build the dep's crate from the working tree
        /// as a cdylib and load it, instead of resolving its natives against
        /// the current binary. Unlike the git form there is no pinned commit,
        /// so a local dep serves `require` only, never versioned resolution.
        rust_load_dylib: bool,
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
/// The `:init` value is a Rust path like `"my_project::cljrs_init_my_project"`. The
/// first `::` segment is the crate name used in `Cargo.toml` and when
/// looking for the compiled shared library on disk.
///
/// Parsed from the `:rust` key in `cljrs.edn`:
///
/// ```edn
/// :rust {:crate "."                        ; path to directory with Cargo.toml
///        :init  "my_project::cljrs_init_my_project"}  ; optional hook-registration fn
/// ```
#[derive(Debug, Clone)]
pub struct RustConfig {
    /// Directory containing the user's `Cargo.toml`, resolved relative to the
    /// `cljrs.edn` file.  Defaults to the `cljrs.edn` directory (`"."`).
    pub crate_dir: PathBuf,
    /// Fully-qualified Rust path to the init function, e.g.
    /// `"my_project::cljrs_init_my_project"`.  When present, `cljrs compile` emits a
    /// call to this function in the generated `main.rs` before loading any
    /// Clojure source.  Omit if you rely solely on `#[cljrs::export]`
    /// inventory-based registration (future feature).
    pub init_fn: Option<Arc<str>>,
}

impl RustConfig {
    /// Derive the Rust crate name from the init function path.
    ///
    /// `"my_project::cljrs_init_my_project"` → `Some("my_project")`.
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
    /// The project root: the directory holding the `cljrs.edn` this config was
    /// loaded from.  Set by [`load_config`]; `None` for a config parsed from a
    /// string or built in code, which was handed over by the host program
    /// rather than discovered on disk.  Native-code trust is keyed on it.
    pub root: Option<PathBuf>,
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

/// Process-wide switch for [`load_config`]; see [`set_discovery_enabled`].
static DISCOVERY_ENABLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Turn `cljrs.edn` discovery on or off for this process.  While it is off,
/// [`load_config`] reports that there is no config, wherever it is called
/// from.  This is what `cljrs --no-config` sets.
pub fn set_discovery_enabled(enabled: bool) {
    DISCOVERY_ENABLED.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Whether [`load_config`] looks for a `cljrs.edn` at all.
pub fn discovery_enabled() -> bool {
    DISCOVERY_ENABLED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Walk up the directory tree from `start`, returning the path of the first
/// `cljrs.edn` file found, or `None` if there is none.
///
/// The walk does not leave the user's home directory: when it reaches `$HOME`
/// it checks there and stops, so a config planted above it (`/home/cljrs.edn`,
/// `/cljrs.edn`) is never picked up for a project inside it.  A `start`
/// outside the home directory is walked to the filesystem root.
///
/// This only locates the file.  [`load_config`] is what decides whether it
/// may be used.
pub fn find_config_file(start: &Path) -> Option<PathBuf> {
    let home = crate::trust::home_dir().and_then(|h| h.canonicalize().ok());
    find_config_file_below(start, home.as_deref())
}

/// [`find_config_file`] with the stopping directory given explicitly
/// (already canonical), so the rule is testable without touching `$HOME`.
fn find_config_file_below(start: &Path, home: Option<&Path>) -> Option<PathBuf> {
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
        if let Some(home) = home
            && dir.canonicalize().is_ok_and(|d| d == home)
        {
            return None;
        }
        dir = dir.parent()?;
    }
}

/// Refuse a config that another user could have planted.
///
/// On unix, both `cljrs.edn` and the directory holding it must be owned by
/// the effective user or by root.  Root is accepted because root can already
/// do anything this check guards against; a world-writable root-owned
/// directory such as `/tmp` is still covered, because the *file* there is
/// owned by whoever created it.  A project root the user has trusted
/// ([`crate::trust::is_trusted`]) is exempt.
///
/// On other platforms this always passes.
fn check_owner(config_path: &Path) -> DepsResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        let dir = config_path.parent().unwrap_or(config_path);
        // SAFETY: `geteuid` has no preconditions and cannot fail.
        let me = unsafe { libc::geteuid() };
        for (what, path) in [("the file", config_path), ("its directory", dir)] {
            let owner = std::fs::metadata(path)?.uid();
            if owner != me && owner != 0 {
                if crate::trust::is_trusted(dir) {
                    return Ok(());
                }
                return Err(DepsError::ForeignOwner {
                    path: config_path.to_path_buf(),
                    root: dir.to_path_buf(),
                    what,
                    owner,
                });
            }
        }
    }
    #[cfg(not(unix))]
    let _ = config_path;
    Ok(())
}

/// Load and parse the `cljrs.edn` closest to `start`, returning `None` if
/// no config file is found or discovery is switched off
/// ([`set_discovery_enabled`]).
///
/// The nearest config is the only candidate: one that fails the ownership
/// check is an error ([`DepsError::ForeignOwner`]), not a reason to keep
/// walking up to a different project's config.
pub fn load_config(start: &Path) -> DepsResult<Option<DepsConfig>> {
    if !discovery_enabled() {
        return Ok(None);
    }
    match find_config_file(start) {
        None => Ok(None),
        Some(path) => {
            check_owner(&path)?;
            let src = std::fs::read_to_string(&path)?;
            let mut config = parse_config(&src, &path).map_err(DepsError::Parse)?;
            config.root = path.parent().map(Path::to_path_buf);
            Ok(Some(config))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_nearest_config_in_an_ancestor() {
        let dir = tempfile::tempdir().unwrap();
        let deep = dir.path().join("a").join("b");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(dir.path().join("cljrs.edn"), "{}").unwrap();
        std::fs::write(dir.path().join("a").join("cljrs.edn"), "{}").unwrap();
        assert_eq!(
            find_config_file_below(&deep, None),
            Some(dir.path().join("a").join("cljrs.edn"))
        );
    }

    /// A config above the home directory is out of reach from inside it; one
    /// in the home directory itself is still found.
    #[test]
    fn the_walk_stops_at_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let work = home.join("work");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(dir.path().join("cljrs.edn"), "{}").unwrap();
        let canonical_home = home.canonicalize().unwrap();

        assert_eq!(find_config_file_below(&work, Some(&canonical_home)), None);
        assert_eq!(find_config_file_below(&home, Some(&canonical_home)), None);
        // Without the boundary the planted config is what would be found.
        assert_eq!(
            find_config_file_below(&work, None),
            Some(dir.path().join("cljrs.edn"))
        );

        std::fs::write(home.join("cljrs.edn"), "{}").unwrap();
        assert_eq!(
            find_config_file_below(&work, Some(&canonical_home)),
            Some(home.join("cljrs.edn"))
        );
    }

    /// Outside the home directory there is no boundary to stop at.
    #[test]
    fn a_start_outside_home_walks_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let elsewhere = dir.path().join("srv").join("app");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::write(dir.path().join("cljrs.edn"), "{}").unwrap();
        assert_eq!(
            find_config_file_below(&elsewhere, Some(&home.canonicalize().unwrap())),
            Some(dir.path().join("cljrs.edn"))
        );
    }

    #[test]
    fn load_config_records_the_project_root() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("sub");
        std::fs::create_dir(&sub).unwrap();
        std::fs::write(dir.path().join("cljrs.edn"), "{:paths [\"src\"]}").unwrap();
        let config = load_config(&sub).unwrap().unwrap();
        assert_eq!(config.root.as_deref(), Some(dir.path()));
    }

    /// Files this process just created are its own, so they pass.
    #[test]
    fn own_files_pass_the_owner_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cljrs.edn");
        std::fs::write(&path, "{}").unwrap();
        assert!(check_owner(&path).is_ok());
    }

    /// A file owned by neither the caller nor root is refused, and the error
    /// names the command that lifts the refusal.  Needs root to hand a file
    /// to another uid, so it is skipped otherwise.
    #[cfg(unix)]
    #[test]
    fn a_foreign_owned_config_is_refused() {
        // SAFETY: `geteuid` has no preconditions.
        if unsafe { libc::geteuid() } != 0 {
            eprintln!("skipped: needs root to chown");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cljrs.edn");
        std::fs::write(&path, "{}").unwrap();
        std::os::unix::fs::chown(&path, Some(65534), None).unwrap();
        let err = check_owner(&path).unwrap_err();
        assert!(matches!(
            err,
            DepsError::ForeignOwner {
                what: "the file",
                owner: 65534,
                ..
            }
        ));
        assert!(err.to_string().contains("cljrs trust"), "{err}");
    }
}
