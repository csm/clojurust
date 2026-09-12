# cljrs-project

## Purpose

The project layer: everything needed to turn a `cljrs.edn` file into resolved
source paths and dependency checkouts — configuration parsing (`config`) and
the git operations that materialize what it declares (`vcs`).

## Status

Implemented. Formed in consolidation stage 5 by merging `cljrs-deps`
(→ `config`) and `cljrs-vcs` (→ `vcs`); both modules keep their previous public
API under the new module paths.

`vcs` is **native-only**: it is `#[cfg(not(target_arch = "wasm32"))]`, and
`gix`/`pgp`/`ssh-key` are declared under the same target predicate, so a
`wasm32` build of `cljrs-runtime` (and hence `cljrs-wasm`) compiles neither.
This is the same gating `cljrs-runtime` previously applied to the `cljrs-vcs`
dependency itself.

`vcs` is also **feature-gated**, in two tiers, so a consumer links only the
weight it actually uses:

- **`vcs`** — reading a local repository plus signature verification
  (`find_repo_root`, `get_file_at_commit`, `worktree_at_commit`,
  `checkout_tree`, `verify_commit_signature`). Pulls gitoxide, rPGP and
  `ssh-key`, but **no** network stack.
- **`vcs-net`** — `fetch_remote`, i.e. cloning/fetching a remote into the local
  cache. This is what selects gix's blocking http transport and with it
  reqwest/hyper/rustls/aws-lc-rs/tokio. Gitoxide routes *every* clone through
  that transport layer, so `vcs-net` gates local-path and `file://` fetches
  too, not only `https://`.

Without either, the crate is just the `cljrs.edn` config model and has no
dependencies outside the workspace. That is what lets `cljrs-runtime` embed
the config model — and, under its own default `deps` feature, local git reads —
without dragging a TLS stack into every embedding of the interpreter.

The workspace dependency entry sets `default-features = false`, so each member
opts in explicitly; only the `cljrs` binary takes `vcs-net`.

All git operations run in-process via [`gix`] (gitoxide) — no `git` binary is
required. Commit-signature verification is native: PGP signatures are checked
with rPGP (`pgp`) and SSH signatures with `ssh-key`, against a caller-supplied
`TrustedKeys` set (there is no fallback to the user's GPG keyring or SSH
`allowed_signers`).

Remote fetch/clone over the network is HTTPS-only and fully pure-Rust (rustls);
local filesystem paths and `file://` URLs are also supported. `ssh://`/scp-like
remotes are supported natively when the optional **`ssh` feature** is enabled;
without it they are rejected with a clear error. `fetch_remote` is called by
`cljrs deps fetch`; `cache_path_for_url` is used by `cljrs deps status` to check
cache presence without network access.

## Features

The crate's own `default` is `["vcs", "vcs-net"]`, but the workspace dependency
entry sets `default-features = false`, so within this repo every member states
what it needs.

| Feature | Default | Enabled by | Description |
|---------|---------|-----------|-------------|
| `vcs`     | on (crate default) | `cljrs-runtime/deps`, `cljrs` | The `vcs` module: local git reads (gitoxide) and commit-signature verification (rPGP, `ssh-key`). No network stack. |
| `vcs-net` | on (crate default) | `cljrs` | `fetch_remote`: clone/fetch a remote into the local cache. Selects gix's blocking http transport, and with it reqwest/hyper/rustls/aws-lc-rs/tokio. Implies `vcs`. |
| `ssh`     | off     | `cljrs` | Native pure-Rust SSH transport (`russh`) for `ssh://`/scp-like remotes. Host keys are verified against `~/.ssh/known_hosts`; authentication is via a running ssh-agent (`$SSH_AUTH_SOCK`). Implies `vcs-net`. |

[`gix`]: https://docs.rs/gix

## File layout

| File | Description |
|------|-------------|
| `src/lib.rs` | Crate root; declares `config` and the native-only, `vcs`-feature-gated `vcs` |
| `src/config.rs` | `cljrs.edn` types (`DepsConfig`, `RustConfig`, `Dependency`, `Alias`, `GitDep`, `TrustedSigner`), `find_config_file`, `load_config` |
| `src/config/parse.rs` | Walk the `cljrs-reader` Form tree from `cljrs.edn` source into `DepsConfig` |
| `src/vcs.rs` | `VcsError` and the `gix`-backed git operations |
| `src/vcs/signature.rs` | Native PGP/SSH commit-signature verification and the `TrustedKeys` set |
| `src/vcs/ssh.rs` | Native SSH transport (`ssh` feature): `russh` + gitoxide's `git::Connection`, known_hosts host-key checks, ssh-agent auth |
| `tests/versioning_harness.rs` | Integration harness — two-repo fixture (library + app) plus a natively SSH-signed commit, covering all versioned-symbol resolution cases |
| `tests/worktree.rs` | `worktree_at_commit` / `checkout_tree` against a local fixture repo |

## Public API

### `cljrs_project::config`

```rust
/// Find the nearest `cljrs.edn` by walking up from `start`.
pub fn find_config_file(start: &Path) -> Option<PathBuf>

/// Load and parse the nearest `cljrs.edn`, returning None if absent.
pub fn load_config(start: &Path) -> DepsResult<Option<DepsConfig>>

/// Parse `cljrs.edn` source text directly (used in tests / CLI).
pub fn parse_config(src: &str, config_path: &Path) -> Result<DepsConfig, String>

pub struct DepsConfig {
    pub paths:                    Vec<PathBuf>,
    pub deps:                     Vec<(Arc<str>, Dependency)>,
    pub aliases:                  Vec<(Arc<str>, Alias)>,
    pub verify_commit_signatures: bool,
    pub trusted_signers:          Vec<TrustedSigner>,  // :trusted-signers — keys allowed
                                         // to sign versioned dependency commits
    pub enforce_native_versions:  bool,  // :enforce-native-versions — pinned-native
                                         // provenance mismatches error instead of warning
    pub rust:                     Option<RustConfig>,
    pub main_ns:                  Option<Arc<str>>,  // :main — AOT entry-point namespace
}

/// A trusted commit signer from `:trusted-signers`: an inline public key
/// (armored PGP or OpenSSH) or a path to a key file resolved relative to
/// `cljrs.edn`.
pub enum TrustedSigner {
    Inline(String),
    File(PathBuf),
}

/// Rust-crate configuration for mixed Rust/Clojure projects.
/// Parsed from the `:rust` key in `cljrs.edn`.
pub struct RustConfig {
    /// Directory containing the user's Cargo.toml (resolved from cljrs.edn dir).
    pub crate_dir: PathBuf,
    /// Fully-qualified init fn, e.g. "my_project::cljrs_init_my_project". Optional.
    pub init_fn:   Option<Arc<str>>,
}

pub enum Dependency {
    Git(GitDep),
    Local {
        root: PathBuf,
        rust_init:       Option<Arc<str>>,  // :rust/init  — native init fn path
        rust_crate_dir:  Option<Arc<str>>,  // :rust/crate — Cargo.toml subdir, relative to root
        rust_load_dylib: bool,              // :rust/load :dylib — build the working tree as a cdylib
    },
}

pub struct GitDep {
    pub url: Arc<str>,
    pub sha: Arc<str>,
    pub rust_init:       Option<Arc<str>>,  // :rust/init  — native init fn path
    pub rust_crate_dir:  Option<Arc<str>>,  // :rust/crate — Cargo.toml subdir
    pub rust_load_dylib: bool,              // :rust/load :dylib — pinned native code
}

pub struct Alias {
    pub extra_paths: Vec<PathBuf>,
    pub extra_deps:  Vec<(Arc<str>, Dependency)>,
}

pub enum DepsError { Io(std::io::Error), Parse(String) }
pub type DepsResult<T> = Result<T, DepsError>;
```

### `cljrs_project::vcs` (native targets, `vcs` feature)

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
/// Requires the `vcs-net` feature (gitoxide routes all clones, local ones
/// included, through its transport layer).
#[cfg(feature = "vcs-net")]
pub fn fetch_remote(url: &str, sha: &str) -> VcsResult<PathBuf>

/// Materialize a files-only working checkout of `sha` for `url` from the local
/// bare cache (network-free; `fetch_remote` must have populated it first).
/// Cached per (url, sha) under `~/.cljrs/cache/git/worktrees/`.  Used to put a
/// dependency's source on the source path for a plain `require`.
pub fn worktree_at_commit(url: &str, sha: &str) -> VcsResult<PathBuf>

/// Check out the tree of `commit` from `repo` (bare or not) into `dest` as a
/// files-only working tree (no `.git`).  `dest` must already exist.
pub fn checkout_tree(repo: &Path, commit: &str, dest: &Path) -> VcsResult<()>

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
pub type VcsResult<T> = Result<T, VcsError>;
```

## cljrs.edn format

```edn
{:paths ["src"]

 :deps
 {my.lib {:git/url "https://github.com/user/my-lib" :git/sha "abc1234ef"}
  ;; Native dep with opt-in pinned native code (the CLI's `native` module):
  my.native.lib {:git/url   "https://github.com/user/my-native-lib"
                 :git/sha   "abc1234ef"
                 :rust/init "my_native_lib::cljrs_init_my_native_lib"
                 :rust/load :dylib}}

 ;; Optional: embed a Rust crate in this project.
 ;; :crate is the path (relative to cljrs.edn) to the directory holding Cargo.toml.
 ;; :init  is the fully-qualified Rust path to a fn(registry: &mut Registry) called
 ;;        at startup before any Clojure source is loaded.
 ;; AOT entry-point namespace (used by `cljrs compile` when no file is given).
 :main my.app.core

 :rust {:crate "."
        :init  "my_project::cljrs_init_my_project"}

 :aliases
 {:dev  {:extra-paths ["dev"]}
  :test {:extra-paths ["test"]
         :extra-deps  {test-tools {:git/url "..." :git/sha "..."}}}}}
```
