//! Project trust: which project roots the user has allowed to run native code.
//!
//! A `cljrs.edn` can name a Rust crate whose compiled library is loaded into
//! the process. The file asking for that cannot be what authorises it, so the
//! authorisation lives here instead: a per-user list of project roots, kept
//! outside every project, that only an explicit user action writes to
//! (`cljrs trust`, or a successful `cljrs build-native`).
//!
//! Two sources are consulted, and either is enough:
//!
//! * the **trust store**, `~/.cljrs/trusted-projects` — one canonical
//!   directory per line, `#` comments and blank lines ignored;
//! * the **`CLJRS_TRUSTED_PROJECTS`** environment variable — a
//!   platform path list (`:`-separated on unix), for CI and other
//!   non-interactive callers that have no store to write to.
//!
//! Trust is keyed on the canonical path of the project root and nothing else:
//! editing `cljrs.edn` does not revoke it, and only that exact directory is
//! covered, not its descendants or ancestors.
//!
//! Besides gating native code, a trusted root is exempt from the
//! file-ownership check in [`crate::config::load_config`].

use std::io;
use std::path::{Path, PathBuf};

/// Environment variable naming extra trusted project roots (a path list).
pub const TRUSTED_PROJECTS_ENV: &str = "CLJRS_TRUSTED_PROJECTS";

/// The user's home directory, from `HOME` (or `USERPROFILE` on Windows).
pub(crate) fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// Where the trust store lives: `~/.cljrs/trusted-projects`, beside the
/// dependency cache. `None` when the home directory is unknown, in which case
/// nothing can be recorded and only [`TRUSTED_PROJECTS_ENV`] grants trust.
pub fn store_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".cljrs").join("trusted-projects"))
}

/// The on-disk list of trusted project roots.
#[derive(Debug, Clone)]
pub struct TrustStore {
    path: PathBuf,
    roots: Vec<PathBuf>,
}

impl TrustStore {
    /// Open the user's store at [`store_path`]. A store that does not exist
    /// yet is empty, not an error.
    pub fn open() -> io::Result<TrustStore> {
        let path = store_path().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "cannot locate the trust store: the home directory is not set",
            )
        })?;
        TrustStore::open_at(path)
    }

    /// Open a store at an explicit location.
    pub fn open_at(path: impl Into<PathBuf>) -> io::Result<TrustStore> {
        let path = path.into();
        let roots = match std::fs::read_to_string(&path) {
            Ok(text) => parse_store(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(TrustStore { path, roots })
    }

    /// The file this store reads and writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every trusted root, in the order it was added.
    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Whether `root` is trusted. `root` is canonicalised first, so a symlink
    /// or a relative path names the directory it resolves to.
    pub fn contains(&self, root: &Path) -> bool {
        match root.canonicalize() {
            Ok(canonical) => self.roots.contains(&canonical),
            Err(_) => false,
        }
    }

    /// Trust `root`, which must be an existing directory. Returns the
    /// canonical path recorded and whether it was newly added. Call
    /// [`TrustStore::save`] to persist.
    pub fn add(&mut self, root: &Path) -> io::Result<(PathBuf, bool)> {
        let canonical = root.canonicalize()?;
        if !canonical.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{} is not a directory", canonical.display()),
            ));
        }
        // The store is line-oriented UTF-8 text; a path it cannot hold
        // faithfully must be refused rather than recorded as something else.
        match canonical.to_str() {
            Some(s) if !s.contains(['\n', '\r']) && !s.starts_with('#') && s.trim() == s => {}
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!(
                        "{} cannot be recorded in the trust store (path is not plain UTF-8 text)",
                        canonical.display()
                    ),
                ));
            }
        }
        if self.roots.contains(&canonical) {
            return Ok((canonical, false));
        }
        self.roots.push(canonical.clone());
        Ok((canonical, true))
    }

    /// Stop trusting `root`. Returns whether an entry was removed. A root
    /// that no longer exists on disk is matched by the path as given, so a
    /// deleted project can still be cleaned out of the store.
    pub fn remove(&mut self, root: &Path) -> bool {
        let target = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let before = self.roots.len();
        self.roots.retain(|r| r != &target);
        self.roots.len() != before
    }

    /// Write the store back, creating its directory if needed. The file is
    /// replaced atomically so a concurrent reader never sees half a list.
    pub fn save(&self) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut text = String::from(
            "# Project roots allowed to load native code. Managed by `cljrs trust` / `cljrs untrust`.\n",
        );
        for root in &self.roots {
            // `add` only admits UTF-8 paths; entries read from disk were text.
            text.push_str(&root.to_string_lossy());
            text.push('\n');
        }
        let tmp = self
            .path
            .with_extension(format!("tmp{}", std::process::id()));
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.path).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
    }
}

/// Parse the store's text: one path per line, `#` comments and blanks skipped.
fn parse_store(text: &str) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let root = PathBuf::from(line);
        if !roots.contains(&root) {
            roots.push(root);
        }
    }
    roots
}

/// Whether `root` appears in a [`TRUSTED_PROJECTS_ENV`]-style path list.
/// Both sides are canonicalised; entries that do not resolve are ignored.
fn listed_in(path_list: &std::ffi::OsStr, canonical_root: &Path) -> bool {
    std::env::split_paths(path_list)
        .filter(|p| !p.as_os_str().is_empty())
        .filter_map(|p| p.canonicalize().ok())
        .any(|p| p == canonical_root)
}

/// Whether the user has trusted the project rooted at `root`, through either
/// [`TRUSTED_PROJECTS_ENV`] or the trust store.
///
/// Any failure — the root does not resolve, the store is unreadable — is
/// "not trusted": this gate fails closed.
pub fn is_trusted(root: &Path) -> bool {
    let Ok(canonical) = root.canonicalize() else {
        return false;
    };
    if let Some(list) = std::env::var_os(TRUSTED_PROJECTS_ENV)
        && listed_in(&list, &canonical)
    {
        return true;
    }
    TrustStore::open().is_ok_and(|store| store.roots().contains(&canonical))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_store_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = TrustStore::open_at(dir.path().join("nope")).unwrap();
        assert!(store.roots().is_empty());
    }

    #[test]
    fn add_save_reopen_remove_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir(&project).unwrap();
        let file = dir.path().join("state").join("trusted-projects");

        let mut store = TrustStore::open_at(&file).unwrap();
        let (canonical, added) = store.add(&project).unwrap();
        assert!(added);
        assert_eq!(canonical, project.canonicalize().unwrap());
        // Adding again is a no-op, not a duplicate.
        assert!(!store.add(&project).unwrap().1);
        store.save().unwrap();

        let mut reopened = TrustStore::open_at(&file).unwrap();
        assert_eq!(reopened.roots(), std::slice::from_ref(&canonical));
        assert!(reopened.contains(&project));
        assert!(reopened.remove(&project));
        assert!(!reopened.remove(&project));
        reopened.save().unwrap();
        assert!(TrustStore::open_at(&file).unwrap().roots().is_empty());
    }

    /// Trust covers exactly the directory named: not what is inside it, and
    /// not what contains it.
    #[test]
    fn trust_does_not_extend_to_relatives() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        let child = project.join("sub");
        std::fs::create_dir_all(&child).unwrap();
        let mut store = TrustStore::open_at(dir.path().join("store")).unwrap();
        store.add(&project).unwrap();
        assert!(store.contains(&project));
        assert!(!store.contains(&child));
        assert!(!store.contains(dir.path()));
    }

    #[test]
    fn a_root_that_is_not_a_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        std::fs::write(&file, "").unwrap();
        let mut store = TrustStore::open_at(dir.path().join("store")).unwrap();
        assert!(store.add(&file).is_err());
        assert!(store.add(&dir.path().join("missing")).is_err());
    }

    /// A project deleted from disk can still be removed from the store.
    #[test]
    fn a_vanished_root_can_be_removed() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("proj");
        std::fs::create_dir(&project).unwrap();
        let mut store = TrustStore::open_at(dir.path().join("store")).unwrap();
        let (canonical, _) = store.add(&project).unwrap();
        std::fs::remove_dir(&project).unwrap();
        assert!(store.remove(&canonical));
    }

    #[test]
    fn comments_blanks_and_duplicates_are_ignored() {
        let roots = parse_store("# header\n\n/a/b\n  /c d/e  \n/a/b\n");
        assert_eq!(roots, [PathBuf::from("/a/b"), PathBuf::from("/c d/e")]);
    }

    #[test]
    fn the_env_list_matches_canonical_entries_only() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        std::fs::create_dir(&a).unwrap();
        std::fs::create_dir(&b).unwrap();
        let canonical_a = a.canonicalize().unwrap();
        let list = std::env::join_paths([dir.path().join("missing"), a.join(".")]).unwrap();
        assert!(listed_in(&list, &canonical_a));
        assert!(!listed_in(&list, &b.canonicalize().unwrap()));
        assert!(!listed_in(std::ffi::OsStr::new(""), &canonical_a));
    }
}
