# cljrs-vcs

## Purpose

Thin wrapper around the `git` CLI for versioned symbol resolution: locating
repository roots, fetching file content at a specific commit, validating commit
hashes, and managing the local dependency cache at `~/.cljrs/cache/git/`.

## Status

Phase 2 (implemented).  All git operations shell out to the `git` binary; no
libgit2 dependency.  Remote fetching (`fetch_remote`) is wired up but the CLI
command that calls it (`cljrs deps fetch`) is Phase 8 (todo).

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | All public functions and `VcsError` type |

## Public API

```rust
/// True if `s` is a valid abbreviated or full git commit hash (7–40 hex chars).
pub fn is_valid_commit_hash(s: &str) -> bool

/// Walk up from `start` to find the git repo root (dir containing `.git`).
pub fn find_repo_root(start: &Path) -> Option<PathBuf>

/// Return file contents at `rel_path` (relative to repo root) at `commit`.
pub fn get_file_at_commit(repo_root: &Path, rel_path: &str, commit: &str) -> VcsResult<String>

/// Path to the local git-dep cache: `~/.cljrs/cache/git/`.
pub fn cache_root() -> PathBuf

/// Clone or fetch `url`, ensuring `sha` is present locally.
/// Returns the path to the bare repo in the cache.
pub fn fetch_remote(url: &str, sha: &str) -> VcsResult<PathBuf>

pub enum VcsError {
    InvalidCommit(String),
    CommitNotFound(String),
    PathNotFound(String, String),
    Io(std::io::Error),
    Utf8,
    NoRepo(PathBuf),
}
```
