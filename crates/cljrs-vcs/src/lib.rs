//! Pure-Rust git helpers for versioned symbol resolution.
//!
//! All git operations are performed in-process with [`gix`] (gitoxide); no
//! `git` binary is required. Commit-signature verification is likewise native
//! (see [`signature`]): PGP signatures are checked with rPGP and SSH signatures
//! with `ssh-key`, against a caller-supplied set of [`TrustedKeys`].
//!
//! Remote fetch/clone over the network is HTTPS-only and fully pure-Rust
//! (rustls). Local filesystem paths and `file://` URLs are also supported
//! (handled in-process). `ssh://`/scp-like remotes are fetched natively (no
//! `ssh` binary) when the optional `ssh` feature is enabled (see the `ssh`
//! module); without it they are rejected with a clear error. Other schemes
//! (`git://`, `http://`) are unsupported.

use std::path::{Path, PathBuf};

use thiserror::Error;

mod signature;
#[cfg(feature = "ssh")]
mod ssh;
pub use signature::{TrustedKeyError, TrustedKeys};

#[derive(Debug, Error)]
pub enum VcsError {
    #[error("invalid commit hash {0:?} (must be 7-40 hex characters)")]
    InvalidCommit(String),
    #[error("commit {0:?} not found in repository (run `cljrs deps fetch`)")]
    CommitNotFound(String),
    #[error("path {0:?} not found at commit {1:?}")]
    PathNotFound(String, String),
    #[error("git error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git output is not valid UTF-8")]
    Utf8,
    #[error("no git repository found at or above {0:?}")]
    NoRepo(PathBuf),
    #[error(
        "unsupported remote {0:?}: supported are https:// URLs, local paths, and (with the `ssh` feature) ssh:// remotes"
    )]
    UnsupportedRemote(String),
    #[error("git error: {0}")]
    Git(String),
    #[error("commit {commit:?} has no valid signature: {reason}")]
    SignatureVerificationFailed { commit: String, reason: String },
}

pub type VcsResult<T> = Result<T, VcsError>;

/// Returns `true` if `s` looks like a valid (abbreviated or full) commit hash:
/// 7–40 lowercase or uppercase hex characters.
pub fn is_valid_commit_hash(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Walk upward from `start` (a file or directory) to find the root of the
/// enclosing git repository, i.e. the working-tree root.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    // Normalise: if `start` is a file, begin from its parent directory.
    let dir: &Path = if start.is_file() {
        start.parent()?
    } else {
        start
    };

    let repo = gix::discover(dir).ok()?;
    repo.workdir().map(|p| p.to_path_buf())
}

/// Return the contents of `rel_path` (relative to the repo root) at `commit`.
///
/// Errors if the commit hash is malformed, the commit is not present locally,
/// or the path does not exist at that commit.
pub fn get_file_at_commit(repo_root: &Path, rel_path: &str, commit: &str) -> VcsResult<String> {
    if !is_valid_commit_hash(commit) {
        return Err(VcsError::InvalidCommit(commit.to_string()));
    }

    let repo = gix::open(repo_root).map_err(|e| VcsError::Git(e.to_string()))?;
    let object = repo
        .rev_parse_single(commit)
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?
        .object()
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?;
    let tree = object
        .try_into_commit()
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?
        .tree()
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?;

    let entry = tree
        .lookup_entry_by_path(Path::new(rel_path))
        .map_err(|e| VcsError::Git(e.to_string()))?
        .ok_or_else(|| VcsError::PathNotFound(rel_path.to_string(), commit.to_string()))?;

    let blob = entry.object().map_err(|e| VcsError::Git(e.to_string()))?;
    String::from_utf8(blob.data.clone()).map_err(|_| VcsError::Utf8)
}

/// Returns the path to the local git-dep cache root: `~/.cljrs/cache/git/`.
pub fn cache_root() -> PathBuf {
    // Prefer $HOME; fall back to the current directory if HOME is unset.
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cljrs").join("cache").join("git")
}

/// Return the local cache path for a given remote URL, without fetching.
///
/// This mirrors the slug derivation inside [`fetch_remote`] so callers can
/// check cache existence without triggering network access.
pub fn cache_path_for_url(url: &str) -> PathBuf {
    cache_root().join(url_slug(url))
}

/// Stable cache directory slug derived from a URL (non-alphanumerics → `_`).
fn url_slug(url: &str) -> String {
    url.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Materialize a files-only working checkout of `sha` for `url` from the local
/// bare cache, returning the worktree root.
///
/// **Network-free**: the bare cache must already contain `sha` (run
/// [`fetch_remote`] / `cljrs deps fetch` first).  The checkout is the tree of
/// `sha` with no `.git`, written under
/// `~/.cljrs/cache/git/worktrees/<slug>@<sha>/`.  Idempotent and cached per
/// `(url, sha)` via a `.cljrs-worktree-complete` sentinel (the worktree has no
/// `.git`, so we cannot probe for one).
pub fn worktree_at_commit(url: &str, sha: &str) -> VcsResult<PathBuf> {
    if !is_valid_commit_hash(sha) {
        return Err(VcsError::InvalidCommit(sha.to_string()));
    }
    let bare = cache_path_for_url(url);
    if !bare.exists() {
        return Err(VcsError::NoRepo(bare));
    }
    let dest = cache_root()
        .join("worktrees")
        .join(format!("{}@{sha}", url_slug(url)));
    let sentinel = dest.join(".cljrs-worktree-complete");
    if sentinel.exists() {
        return Ok(dest);
    }
    std::fs::create_dir_all(&dest).map_err(VcsError::Io)?;
    checkout_tree(&bare, sha, &dest)?;
    std::fs::write(&sentinel, sha.as_bytes()).map_err(VcsError::Io)?;
    Ok(dest)
}

/// Check out the tree of `commit` from the repository at `repo` (bare or not)
/// into `dest` as a files-only working tree (no `.git`).
///
/// Used to materialize dependency sources from the local cache without a
/// network round-trip.  `dest` must already exist.
pub fn checkout_tree(repo: &Path, commit: &str, dest: &Path) -> VcsResult<()> {
    let repository = gix::open(repo).map_err(|e| VcsError::Git(e.to_string()))?;
    let tree = repository
        .rev_parse_single(commit)
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?
        .object()
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?
        .peel_to_tree()
        .map_err(|e| VcsError::Git(e.to_string()))?;
    let mut index = repository
        .index_from_tree(&tree.id)
        .map_err(|e| VcsError::Git(e.to_string()))?;
    let opts = repository
        .checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)
        .map_err(|e| VcsError::Git(e.to_string()))?;
    let odb = repository
        .objects
        .clone()
        .into_arc()
        .map_err(|e| VcsError::Git(e.to_string()))?;
    let should_interrupt = std::sync::atomic::AtomicBool::new(false);
    gix::worktree::state::checkout(
        &mut index,
        dest,
        odb,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &should_interrupt,
        opts,
    )
    .map_err(|e| VcsError::Git(e.to_string()))?;
    Ok(())
}

/// Clone or fetch a git repository into the local cache.
///
/// `url`  — an `https://` URL, a local filesystem path, a `file://` URL, or
///          (with the `ssh` feature) an `ssh://`/scp-like remote
/// `sha`  — the commit SHA that must be reachable after the operation
///
/// Returns the path to the bare repository in the cache.
pub fn fetch_remote(url: &str, sha: &str) -> VcsResult<PathBuf> {
    if !is_valid_commit_hash(sha) {
        return Err(VcsError::InvalidCommit(sha.to_string()));
    }
    let kind = classify_remote(url);
    if matches!(kind, RemoteKind::Unsupported) {
        return Err(VcsError::UnsupportedRemote(url.to_string()));
    }
    // ssh requires the optional `ssh` feature; without it, reject clearly.
    #[cfg(not(feature = "ssh"))]
    if matches!(kind, RemoteKind::Ssh) {
        return Err(VcsError::UnsupportedRemote(url.to_string()));
    }

    let repo_dir = cache_root().join(url_slug(url));

    match kind {
        #[cfg(feature = "ssh")]
        RemoteKind::Ssh => ssh::fetch_into_cache(url, &repo_dir)?,
        // https / local / file (and, without the `ssh` feature, nothing else
        // reaches here).
        _ => {
            if repo_dir.exists() {
                // Already cloned — fetch to make sure we have the requested commit.
                fetch_existing(&repo_dir)?;
            } else {
                std::fs::create_dir_all(&repo_dir).map_err(VcsError::Io)?;
                clone_bare(url, &repo_dir)?;
            }
        }
    }

    // Verify that the requested commit is now present locally.
    let repo = gix::open(&repo_dir).map_err(|e| VcsError::Git(e.to_string()))?;
    if repo.rev_parse_single(sha).is_err() {
        return Err(VcsError::CommitNotFound(sha.to_string()));
    }

    Ok(repo_dir)
}

/// How a remote URL is transported.
enum RemoteKind {
    /// `https://` (pure-Rust network) or a local filesystem path / `file://`.
    Supported,
    /// `ssh://` or scp-like `git@host:path`. Fetched natively only with the
    /// `ssh` feature; otherwise rejected.
    Ssh,
    /// `git://`, `http://`, or any other unsupported scheme.
    Unsupported,
}

/// Classify a remote URL. `https://` (network) plus local paths and `file://`
/// are always supported (gitoxide handles them in-process). `ssh://` and
/// scp-like `git@host:path` map to [`RemoteKind::Ssh`]. Other network schemes
/// (`http://`, `git://`, …) are unsupported.
fn classify_remote(url: &str) -> RemoteKind {
    if url.starts_with("https://") || url.starts_with("file://") {
        return RemoteKind::Supported;
    }
    if url.starts_with("ssh://") {
        return RemoteKind::Ssh;
    }
    if url.contains("://") {
        // An explicit non-https/file/ssh scheme (git://, http://, …).
        return RemoteKind::Unsupported;
    }
    // No scheme: an scp-like `user@host:path` is SSH; otherwise a local path.
    match url.split_once(':') {
        Some((host, _)) if !host.is_empty() && !host.contains('/') => RemoteKind::Ssh,
        _ => RemoteKind::Supported,
    }
}

/// Clone `url` as a bare repository into `repo_dir`.
fn clone_bare(url: &str, repo_dir: &Path) -> VcsResult<()> {
    let mut prepare =
        gix::prepare_clone_bare(url, repo_dir).map_err(|e| VcsError::Git(e.to_string()))?;
    let (_repo, _outcome) = prepare
        .fetch_only(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .map_err(|e| VcsError::Git(e.to_string()))?;
    Ok(())
}

/// Fetch updates for an already-cloned bare repository at `repo_dir`.
fn fetch_existing(repo_dir: &Path) -> VcsResult<()> {
    let repo = gix::open(repo_dir).map_err(|e| VcsError::Git(e.to_string()))?;
    let remote = repo
        .find_default_remote(gix::remote::Direction::Fetch)
        .ok_or_else(|| VcsError::Git("repository has no default remote".to_string()))?
        .map_err(|e| VcsError::Git(e.to_string()))?;
    remote
        .connect(gix::remote::Direction::Fetch)
        .map_err(|e| VcsError::Git(e.to_string()))?
        .prepare_fetch(gix::progress::Discard, Default::default())
        .map_err(|e| VcsError::Git(e.to_string()))?
        .receive(gix::progress::Discard, &gix::interrupt::IS_INTERRUPTED)
        .map_err(|e| VcsError::Git(e.to_string()))?;
    Ok(())
}

/// Verify the PGP or SSH signature on `commit` inside `repo_root` against the
/// caller-supplied set of `trusted` keys.
///
/// Returns `Ok(())` when the commit carries a cryptographically valid signature
/// whose signing key is present in `trusted`. Returns
/// `Err(SignatureVerificationFailed)` for an unsigned commit, an invalid
/// signature, or a signature made by a key that is not trusted.
pub fn verify_commit_signature(
    repo_root: &Path,
    commit: &str,
    trusted: &TrustedKeys,
) -> VcsResult<()> {
    if !is_valid_commit_hash(commit) {
        return Err(VcsError::InvalidCommit(commit.to_string()));
    }
    let repo = gix::open(repo_root).map_err(|e| VcsError::Git(e.to_string()))?;
    let object = repo
        .rev_parse_single(commit)
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?
        .object()
        .map_err(|_| VcsError::CommitNotFound(commit.to_string()))?;
    let raw = object.data.clone();

    signature::verify_commit_object(&raw, trusted).map_err(|reason| {
        VcsError::SignatureVerificationFailed {
            commit: commit.to_string(),
            reason,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_hashes() {
        assert!(is_valid_commit_hash("abc1234"));
        assert!(is_valid_commit_hash(
            "abc1234ef5678901234567890123456789012345"
        ));
        assert!(!is_valid_commit_hash("abc123")); // too short
        assert!(!is_valid_commit_hash("xyz1234")); // non-hex
        assert!(!is_valid_commit_hash(""));
    }

    #[test]
    fn remote_classification() {
        use RemoteKind::*;
        assert!(matches!(
            classify_remote("https://github.com/u/r"),
            Supported
        ));
        assert!(matches!(classify_remote("file:///tmp/repo"), Supported));
        assert!(matches!(classify_remote("/tmp/local/repo"), Supported)); // absolute local path
        assert!(matches!(classify_remote("../relative/repo"), Supported)); // relative local path
        assert!(matches!(classify_remote("ssh://git@github.com/u/r"), Ssh));
        assert!(matches!(classify_remote("git@github.com:u/r.git"), Ssh)); // scp-like
        assert!(matches!(
            classify_remote("git://github.com/u/r"),
            Unsupported
        ));
        assert!(matches!(
            classify_remote("http://github.com/u/r"),
            Unsupported
        )); // insecure
    }
}
