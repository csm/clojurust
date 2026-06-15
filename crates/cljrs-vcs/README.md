# cljrs-vcs

## Purpose

Pure-Rust git helpers for versioned symbol resolution: locating repository
roots, fetching file content at a specific commit, cloning/fetching remotes,
validating commit hashes, managing the local dependency cache at
`~/.cljrs/cache/git/`, and verifying commit signatures natively.

## Status

Phase 2 (implemented), extended in Phase 8. All git operations run in-process
via [`gix`] (gitoxide) — no `git` binary is required. Commit-signature
verification is native: PGP signatures are checked with rPGP (`pgp`) and SSH
signatures with `ssh-key`, against a caller-supplied [`TrustedKeys`] set (there
is no fallback to the user's GPG keyring or SSH `allowed_signers`).

Remote fetch/clone over the network is HTTPS-only and fully pure-Rust (rustls);
local filesystem paths and `file://` URLs are also supported. `ssh://`/scp-like
remotes are supported natively when the optional **`ssh` feature** is enabled
(see below); without it they are rejected with a clear error. `fetch_remote` is
called by `cljrs deps fetch`; `cache_path_for_url` is used by `cljrs deps
status` to check cache presence without network access.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `ssh`   | off     | Native pure-Rust SSH transport (`russh`) for `ssh://`/scp-like remotes. Host keys are verified against `~/.ssh/known_hosts`; authentication is via a running ssh-agent (`$SSH_AUTH_SOCK`). The `cljrs` binary enables this. |

[`gix`]: https://docs.rs/gix
[`TrustedKeys`]: src/signature.rs

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Public functions, `VcsError`, and the `gix`-backed git operations |
| `src/signature.rs` | Native PGP/SSH commit-signature verification and the `TrustedKeys` set |
| `src/ssh.rs` | Native SSH transport (`ssh` feature): `russh` + gitoxide's `git::Connection`, known_hosts host-key checks, ssh-agent auth |
| `tests/versioning_harness.rs` | Integration test harness — two-repo fixture (library + app) plus a natively SSH-signed commit, covering all versioned-symbol resolution cases |

## Public API

```rust
/// True if `s` is 7–40 lowercase or uppercase hex characters.
pub fn is_valid_commit_hash(s: &str) -> bool

/// Walk up from `start` to find the git working-tree root.
pub fn find_repo_root(start: &Path) -> Option<PathBuf>

/// Return file contents at `rel_path` (relative to repo root) at `commit`.
pub fn get_file_at_commit(repo_root: &Path, rel_path: &str, commit: &str) -> VcsResult<String>

/// Path to the local git-dep cache: `~/.cljrs/cache/git/`.
pub fn cache_root() -> PathBuf

/// Local cache path for a given remote URL (same slug derivation as `fetch_remote`).
/// Does not touch the network; use to check cache existence before fetching.
pub fn cache_path_for_url(url: &str) -> PathBuf

/// Clone or fetch `url` (https/local/file), ensuring `sha` is present locally.
/// Returns the path to the bare repo in the cache.
pub fn fetch_remote(url: &str, sha: &str) -> VcsResult<PathBuf>

/// Verify the PGP or SSH signature on `commit` against `trusted`.
/// Ok only when the signature is valid AND its key is in the trusted set.
pub fn verify_commit_signature(repo_root: &Path, commit: &str, trusted: &TrustedKeys) -> VcsResult<()>

/// A cljrs-managed set of public keys trusted to sign commits.
pub struct TrustedKeys { /* … */ }
impl TrustedKeys {
    pub fn new() -> Self
    pub fn is_empty(&self) -> bool
    /// Auto-detect PGP-armored vs OpenSSH public-key text.
    pub fn add_key_text(&mut self, text: &str) -> Result<(), TrustedKeyError>
    pub fn add_pgp_armored(&mut self, armored: &str) -> Result<(), TrustedKeyError>
    pub fn add_ssh_openssh(&mut self, openssh: &str) -> Result<(), TrustedKeyError>
}

pub enum TrustedKeyError { Pgp(String), Ssh(String), Unrecognized }

pub enum VcsError {
    InvalidCommit(String),
    CommitNotFound(String),
    PathNotFound(String, String),
    Io(std::io::Error),
    Utf8,
    NoRepo(PathBuf),
    UnsupportedRemote(String),
    Git(String),
    SignatureVerificationFailed { commit: String, reason: String },
}
```
