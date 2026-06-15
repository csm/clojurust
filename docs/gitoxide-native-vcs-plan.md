# Plan: Native git (gitoxide) + native commit-signature verification in `cljrs-vcs`

## Context

`cljrs-vcs` is a thin wrapper around the `git` CLI for versioned symbol
resolution. Every git operation shells out to the `git` binary, and commit
signature verification shells out to `git verify-commit` (which in turn invokes
`gpg` or `ssh-keygen`). This makes the toolchain depend on an external `git`
install at runtime, with no pure-Rust fallback, and inherits git's
keyring-based trust model implicitly.

This change replaces the git subprocess calls with the **gitoxide** (`gix`)
pure-Rust git implementation, and replaces signature verification with **native
PGP/SSH signature checking** against a **cljrs-managed set of trusted keys**.
The outcome: `cljrs` no longer requires the `git` binary (HTTPS remotes are
fully pure-Rust), and signature trust is explicit and self-contained rather than
delegated to the user's GPG keyring.

### Decisions (confirmed with user)
- **Trust model:** cljrs-managed trusted keys (declared in `cljrs.edn`), not the
  user's GPG keyring / SSH `allowed_signers`.
- **Transport:** HTTPS only at first, pure-Rust (rustls). `ssh://` remotes return
  a clear error (no `ssh` binary spawn). Pure-Rust SSH sync is deferred to a
  later phase (see Future work).
- **Scope:** also migrate `cljrs-dylib`'s `checkout_at` worktree checkout off the
  git binary.

## Current shelling-out sites

`crates/cljrs-vcs/src/lib.rs`:
| Fn | git call | gitoxide replacement |
|---|---|---|
| `find_repo_root` (47) | `rev-parse --show-toplevel` | `gix::discover` + `work_dir()` |
| `get_file_at_commit` (73) | `show <sha>:<path>` | rev-parse → peel to commit → tree → blob |
| `fetch_remote` (148/160/173) | `fetch`, `clone --bare`, `cat-file -e` | `gix::prepare_clone_bare` + fetch + object lookup |
| `verify_commit_signature` (197) | `verify-commit` | native PGP/SSH verification |

`crates/cljrs-dylib/src/lib.rs`:
| Fn | git call | replacement |
|---|---|---|
| `checkout_at` (203) | `clone` (local) + `checkout <sha>` | local `gix` clone + `gix-worktree-state` checkout of the pinned tree |

Callers (signatures must be preserved): `cljrs-env/src/versioned.rs` (`find_repo_root`,
`get_file_at_commit`), `cljrs-env/src/loader.rs` (`find_repo_root`),
`cljrs-env/src/env.rs:527` (`verify_commit_signature`), `cljrs/src/main.rs:1099/1140`
(`fetch_remote`, `cache_path_for_url`), `cljrs-dylib/src/lib.rs:150` (`fetch_remote`).

## Approach

### 1. Dependencies (`Cargo.toml` workspace + `cljrs-vcs/Cargo.toml`)
Add to `[workspace.dependencies]` and reference with `{ workspace = true }`:
- `gix` with a minimal feature set: `blocking-http-transport-reqwest-rust-tls`
  (HTTPS fetch/clone over rustls, consistent with `cljrs-net`'s rustls usage),
  `revision` (rev-parse), and the object/worktree access that ships in the
  default features. Disable default features and enable only what is needed to
  keep the dependency lean; pin an exact version like the other deps.
- `gix-worktree-state` (or use `gix`'s checkout entry point) for the dylib
  worktree checkout.
- `pgp` (rPGP, pure-Rust OpenPGP) for PGP signature verification.
- `ssh-key` with the `ed25519`/`p256`/`rsa` + signature features for SSHSIG
  (the `-----BEGIN SSH SIGNATURE-----` format git uses for SSH-signed commits).

Rationale for rPGP + `ssh-key`: both are pure-Rust and align with the
workspace's existing pure-Rust crypto stance (rustls/ring, blake3). Avoid
`sequoia-openpgp` (pulls C/nettle by default).

### 2. Rewrite `crates/cljrs-vcs/src/lib.rs` git operations with `gix`
Keep all public function signatures and `VcsError` variants unchanged so callers
compile untouched. Preserve `is_valid_commit_hash` validation guards.

- **`find_repo_root`**: `gix::discover(start)` → `repo.workdir()` (map to
  `Option<PathBuf>`; `None` when discovery fails). Keep the file→parent
  normalization.
- **`get_file_at_commit`**: `gix::open(repo_root)` → `repo.rev_parse_single(commit)`
  → `.object()?.peel_to_commit()?.tree()?` → `lookup_entry_by_path(rel_path)`.
  Missing entry → `PathNotFound`; unknown rev → `CommitNotFound`; non-UTF-8 blob
  → `Utf8`. Map gix errors to the existing variants (add `#[from]`/`map_err`
  conversions in `VcsError`; do not change the public variant set if avoidable —
  fold gix errors into `Io`/`CommitNotFound`/`PathNotFound`).
- **`fetch_remote`**: keep the URL→slug cache path logic and the `repo_dir.exists()`
  branch. New repo → `gix::prepare_clone_bare(url, &repo_dir)` then drive the
  fetch; existing repo → open and fetch the default remote. After fetch, confirm
  the sha is present via `repo.find_object(oid)` / `repo.objects.exists(oid)` →
  `CommitNotFound` when absent. For `ssh://`/`git@` URLs return a clear `Io`/new
  error (HTTPS-only decision). Keep `cache_root`/`cache_path_for_url` as-is.

### 3. Native signature verification (`crates/cljrs-vcs/src/lib.rs` + new module)
Replace `verify_commit_signature` body. New helper module (e.g.
`src/signature.rs`) with:
- **Extract signature + signed payload from the commit object.** Read the commit
  via `gix` as raw bytes; use `gix_object::CommitRef` to read the `gpgsig` extra
  header (the armored signature) and reconstruct the *signed payload* = the
  commit object text with the `gpgsig` header removed (this is exactly what git
  signs). gitoxide exposes the extra headers; the payload reconstruction is the
  canonical "commit without gpgsig" serialization.
- **Dispatch by armor banner:** `-----BEGIN PGP SIGNATURE-----` → rPGP path;
  `-----BEGIN SSH SIGNATURE-----` → `ssh-key::SshSig` path. No signature →
  `SignatureVerificationFailed { reason: "commit is not signed" }`.
- **Verify against cljrs-managed trusted keys.** New API:
  `verify_commit_signature(repo_root, commit, trusted: &TrustedKeys) -> VcsResult<()>`
  where `TrustedKeys` holds parsed PGP public keys and SSH public keys. A valid
  signature whose key is **not** in `trusted` → `SignatureVerificationFailed`
  with reason "signing key not in trusted set". (This changes the function
  signature — see §5 for threading `TrustedKeys` through callers.)

### 4. Trusted-keys configuration (`crates/cljrs-deps`)
Add a field to `DepsConfig` (`crates/cljrs-deps/src/lib.rs:115`) e.g.
`trusted_signers: Vec<TrustedSigner>` and parse a new `cljrs.edn` key
(e.g. `:trusted-signers`) in `crates/cljrs-deps/src/parse.rs` (alongside the
existing `:verify-commit-signatures` handling at `parse.rs:56`). Each entry is
either an inline armored public key or a path to a key file (PGP `.asc` / SSH
`.pub`). Provide a constructor that loads these into the `cljrs-vcs::TrustedKeys`
type. Document the field in `crates/cljrs-deps/README.md`.

### 5. Thread `TrustedKeys` to the verifier
`cljrs-env/src/env.rs:517` (`check_commit_signature`) currently calls
`cljrs_vcs::verify_commit_signature(repo_root, commit)`. Store the loaded
`TrustedKeys` on the env/globals (next to `verify_commit_signatures: AtomicBool`)
when the config is parsed, and pass it into the verifier. Same for the AOT path
in `crates/cljrs-compiler/src/aot.rs:413`. Preserve the per-`(repo_root, commit)`
success cache.

### 6. Migrate `cljrs-dylib` `checkout_at` (`crates/cljrs-dylib/src/lib.rs:203`)
Replace the local `git clone` + `git checkout <sha>` with: open the bare repo
with `gix`, resolve the commit's tree, and check it out into `dest` using
`gix-worktree-state` (write the index + materialize files). The
`dest.join(".git").exists()` short-circuit becomes a check for an existing
populated checkout (e.g. a sentinel file or `dest` non-empty). No network — this
is a local materialization.

### 7. READMEs & docs
Update `crates/cljrs-vcs/README.md` (no longer "shells out to git"; document
gix + native signature verification + `TrustedKeys`), `crates/cljrs/README.md`,
`crates/cljrs-dylib/README.md`, `crates/cljrs-deps/README.md`, and the
`cljrs-vcs` rows in `VERSIONING.md`. Per CLAUDE.md, update each crate README in
the same commit as the code change.

## Critical files
- `crates/cljrs-vcs/Cargo.toml`, `crates/cljrs-vcs/src/lib.rs`, new `src/signature.rs`
- `Cargo.toml` (workspace deps)
- `crates/cljrs-deps/src/lib.rs`, `crates/cljrs-deps/src/parse.rs`
- `crates/cljrs-env/src/env.rs`, `crates/cljrs-compiler/src/aot.rs`
- `crates/cljrs-dylib/src/lib.rs`
- `crates/cljrs-vcs/tests/versioning_harness.rs` (signature tests, see below)
- READMEs listed in §7

## Test changes
`crates/cljrs-vcs/tests/versioning_harness.rs` currently builds GPG-signed
fixtures via the `gpg` binary (`setup_gpg`, lines 176–271). Replace with native
fixtures:
- Generate an Ed25519 key in-test with `ssh-key`, build the SSHSIG over a commit
  payload natively, and write the signed commit object (no `gpg`/`ssh-keygen`).
  Add the public key to a `TrustedKeys` and assert `verify_commit_signature`
  passes; assert an unsigned commit and an untrusted-key commit both fail.
- The non-signature git-operation tests still create fixtures with the `git`
  binary in test setup; that is acceptable (tests already depend on git for
  fixture creation). Optionally migrate fixture creation to `gix` later — out of
  scope here.

## Verification
1. `cargo build` and `cargo clippy --workspace` — clean.
2. `cargo test -p cljrs-vcs` — git-op tests pass via gix; native signature
   tests (positive/negative/untrusted) pass with **no `git`/`gpg` binary needed**
   for the signature path.
3. `cargo test -p cljrs-deps` — `:trusted-signers` parsing.
4. `cargo test -p cljrs-env -p cljrs-dylib` — versioned resolution + pinned
   native checkout still work.
5. Manual end-to-end: a `cljrs.edn` with an https git dep + `:verify-commit-signatures true`
   and a `:trusted-signers` entry → `cljrs run` fetches via gix and verifies
   natively; with the binary `git` removed from `PATH`, HTTPS fetch + verify
   still succeed; an `ssh://` dep returns the clear "https-only" error.

## Future work: pure-Rust SSH transport (later phase)
HTTPS-only is the starting point. A follow-up phase should investigate wiring a
pure-Rust SSH client — `russh` is the leading candidate — into gitoxide as a
custom transport so `ssh://`/`git@` remotes fetch without spawning the system
`ssh` binary. gitoxide's transport layer is pluggable (`gix-transport`'s
`client::Transport` trait), but it has no built-in pure-Rust SSH backend, so
this means implementing a `russh`-backed transport that speaks the git
pack-protocol over an SSH channel (invoking the remote `git-upload-pack`),
including host-key and key-auth handling. Non-trivial; scoped as its own phase.
Until then, `ssh://` remotes return the clear "https-only" error from §2.

## Risks / notes
- `gix` HTTPS fetch feature surface is the main dependency-weight risk; pin the
  version and enable the minimal feature set.
- Reconstructing the exact signed payload (commit minus `gpgsig`) must be
  byte-exact or verification fails — cover with the round-trip native test.
- Public API change: `verify_commit_signature` gains a `TrustedKeys` parameter;
  one caller (`cljrs-env/src/env.rs`) and the AOT path must be updated together.
