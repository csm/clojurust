//! Git subprocess helpers for versioned symbol resolution.
//!
//! All git operations are performed by shelling out to the `git` binary.
//! No network access happens here; callers are responsible for ensuring
//! that required commits are present locally before calling these functions.

use std::path::{Path, PathBuf};

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VcsError {
    #[error("invalid commit hash {0:?} (must be 7-40 hex characters)")]
    InvalidCommit(String),
    #[error("commit {0:?} not found in repository (run `cljrs deps fetch`)")]
    CommitNotFound(String),
    #[error("path {0:?} not found at commit {1:?}")]
    PathNotFound(String, String),
    #[error("git subprocess error: {0}")]
    Io(#[from] std::io::Error),
    #[error("git output is not valid UTF-8")]
    Utf8,
    #[error("no git repository found at or above {0:?}")]
    NoRepo(PathBuf),
}

pub type VcsResult<T> = Result<T, VcsError>;

/// Returns `true` if `s` looks like a valid (abbreviated or full) commit hash:
/// 7–40 lowercase or uppercase hex characters.
pub fn is_valid_commit_hash(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Walk upward from `start` (a file or directory) to find the root of the
/// enclosing git repository, i.e. the directory that contains `.git`.
pub fn find_repo_root(start: &Path) -> Option<PathBuf> {
    // Normalise: if `start` is a file, begin from its parent directory.
    let dir: &Path = if start.is_file() {
        start.parent()?
    } else {
        start
    };

    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("rev-parse")
        .arg("--show-toplevel")
        .output()
        .ok()?;

    if output.status.success() {
        let s = String::from_utf8(output.stdout).ok()?;
        Some(PathBuf::from(s.trim()))
    } else {
        None
    }
}

/// Return the contents of `rel_path` (relative to the repo root) at `commit`.
///
/// Errors if the commit hash is malformed, the commit is not present locally,
/// or the path does not exist at that commit.
pub fn get_file_at_commit(repo_root: &Path, rel_path: &str, commit: &str) -> VcsResult<String> {
    if !is_valid_commit_hash(commit) {
        return Err(VcsError::InvalidCommit(commit.to_string()));
    }

    let spec = format!("{commit}:{rel_path}");
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("show")
        .arg(&spec)
        .output()?;

    if output.status.success() {
        String::from_utf8(output.stdout).map_err(|_| VcsError::Utf8)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("does not exist") || stderr.contains("exists on disk") {
            Err(VcsError::PathNotFound(
                rel_path.to_string(),
                commit.to_string(),
            ))
        } else {
            Err(VcsError::CommitNotFound(commit.to_string()))
        }
    }
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
    let slug: String = url
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    cache_root().join(slug)
}

/// Clone or fetch a remote git repository into the local cache.
///
/// `url`  — remote URL (https or ssh)
/// `sha`  — the commit SHA that must be reachable after the operation
///
/// Returns the path to the bare repository in the cache.
pub fn fetch_remote(url: &str, sha: &str) -> VcsResult<PathBuf> {
    if !is_valid_commit_hash(sha) {
        return Err(VcsError::InvalidCommit(sha.to_string()));
    }

    // Stable cache directory derived from the URL (replace non-alphanum with _).
    let slug: String = url
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let repo_dir = cache_root().join(&slug);

    if repo_dir.exists() {
        // Already cloned — fetch to make sure we have the requested commit.
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .arg("fetch")
            .arg("--quiet")
            .arg("origin")
            .status()?;
        if !status.success() {
            return Err(VcsError::Io(std::io::Error::other("git fetch failed")));
        }
    } else {
        std::fs::create_dir_all(&repo_dir).map_err(VcsError::Io)?;
        let status = std::process::Command::new("git")
            .arg("clone")
            .arg("--bare")
            .arg("--quiet")
            .arg(url)
            .arg(&repo_dir)
            .status()?;
        if !status.success() {
            return Err(VcsError::Io(std::io::Error::other("git clone failed")));
        }
    }

    // Verify that the requested commit is now present.
    let check = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo_dir)
        .arg("cat-file")
        .arg("-e")
        .arg(sha)
        .status()?;
    if !check.success() {
        return Err(VcsError::CommitNotFound(sha.to_string()));
    }

    Ok(repo_dir)
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
}
