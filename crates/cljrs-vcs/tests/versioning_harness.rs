//! Versioning test harness.
//!
//! Two git repositories are created in temporary directories:
//!
//! - **library repo** — a minimal Clojure library (`src/mylib.cljc`) with two
//!   commits.  The first commit is tagged `v1.0.0`; the second is HEAD.
//! - **app repo** — a consumer that references the library via `cljrs.edn`.
//!
//! The test cases cover every variant of versioned symbol resolution:
//!
//! | Case | What is tested |
//! |------|----------------|
//! | No version → HEAD | `get_file_at_commit` at HEAD sha returns v2 content |
//! | Tagged version → pinned commit | sha of tag `v1.0.0` returns v1 content |
//! | Signature positive | `verify_commit_signature` on a GPG-signed commit → Ok |
//! | Signature negative | `verify_commit_signature` on an unsigned commit → Err |

use std::path::{Path, PathBuf};
use std::process::Command;

use cljrs_vcs::{
    find_repo_root, get_file_at_commit, is_valid_commit_hash, verify_commit_signature, VcsError,
};

// ---------------------------------------------------------------------------
// Git subprocess helpers
// ---------------------------------------------------------------------------

fn git_cmd(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command failed to start")
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git_cmd(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Returns the full commit SHA for `rev` (e.g. "HEAD", "v1.0.0^{}").
fn git_sha(dir: &Path, rev: &str) -> String {
    let out = git_cmd(dir, &["rev-parse", rev]);
    assert!(
        out.status.success(),
        "git rev-parse {rev} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

// ---------------------------------------------------------------------------
// Library repo fixture
// ---------------------------------------------------------------------------

struct LibraryRepo {
    // Keeps the temp dir alive for the duration of the test.
    _dir: tempfile::TempDir,
    pub root: PathBuf,
    /// SHA of the v1 commit (tagged as v1.0.0).
    pub commit_v1: String,
    /// SHA of the v2 commit (HEAD).
    pub commit_v2: String,
}

/// Creates a library repo with two commits:
///
/// ```
/// v1 (tagged v1.0.0): greeting returns "hello-v1"
/// v2 (HEAD):          greeting returns "hello-v2"
/// ```
fn setup_library() -> LibraryRepo {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();

    git_ok(&root, &["init", "-b", "main"]);
    git_ok(&root, &["config", "user.email", "test@example.com"]);
    git_ok(&root, &["config", "user.name", "Test"]);
    // Override any global commit.gpgsign so test commits don't invoke a signing server.
    git_ok(&root, &["config", "commit.gpgsign", "false"]);

    // --- commit v1 ---
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/mylib.cljc"),
        "(ns mylib)\n(defn greeting [] \"hello-v1\")\n",
    )
    .unwrap();
    git_ok(&root, &["add", "src/mylib.cljc"]);
    git_ok(&root, &["commit", "-m", "v1: initial library"]);
    let commit_v1 = git_sha(&root, "HEAD");
    // Lightweight tag pointing at v1 commit.
    git_ok(&root, &["tag", "v1.0.0"]);

    // --- commit v2 (HEAD) ---
    std::fs::write(
        root.join("src/mylib.cljc"),
        "(ns mylib)\n(defn greeting [] \"hello-v2\")\n",
    )
    .unwrap();
    git_ok(&root, &["add", "src/mylib.cljc"]);
    git_ok(&root, &["commit", "-m", "v2: updated greeting"]);
    let commit_v2 = git_sha(&root, "HEAD");

    LibraryRepo {
        _dir: dir,
        root,
        commit_v1,
        commit_v2,
    }
}

// ---------------------------------------------------------------------------
// App repo fixture
// ---------------------------------------------------------------------------

struct AppRepo {
    _dir: tempfile::TempDir,
    pub root: PathBuf,
}

/// Creates an app repo whose `cljrs.edn` pins the library at its HEAD sha.
///
/// The app itself does not fetch or clone anything; the `cljrs.edn` exists
/// only to demonstrate the dependency config structure.
fn setup_app(lib: &LibraryRepo) -> AppRepo {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();

    git_ok(&root, &["init", "-b", "main"]);
    git_ok(&root, &["config", "user.email", "app@example.com"]);
    git_ok(&root, &["config", "user.name", "App"]);
    git_ok(&root, &["config", "commit.gpgsign", "false"]);

    // cljrs.edn pins the library at v2 (HEAD).
    let lib_url = lib.root.display().to_string();
    std::fs::write(
        root.join("cljrs.edn"),
        format!(
            "{{:paths [\"src\"]\n :deps {{mylib {{:git/url \"{lib_url}\" :git/sha \"{}\"}}}}}}\n",
            lib.commit_v2
        ),
    )
    .unwrap();

    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/app.cljc"),
        // mylib/greeting with no @ suffix resolves to HEAD (commit_v2).
        // mylib/greeting@<commit_v1> would resolve to the v1 commit.
        "(ns app (:require [mylib]))\n(defn -main [] (mylib/greeting))\n",
    )
    .unwrap();

    git_ok(&root, &["add", "."]);
    git_ok(&root, &["commit", "-m", "initial app"]);

    AppRepo { _dir: dir, root }
}

// ---------------------------------------------------------------------------
// GPG signing fixture
// ---------------------------------------------------------------------------

struct GpgSetup {
    // Keeps the temp GNUPGHOME dir alive.
    _homedir: tempfile::TempDir,
    pub fingerprint: String,
    /// Shell wrapper that calls `gpg --homedir <homedir>` so any git subprocess
    /// that invokes gpg will use our test keyring.
    pub wrapper_path: PathBuf,
}

/// Generates a temporary ed25519 GPG key in an isolated homedir and returns a
/// wrapper script that points gpg at that homedir.
///
/// Returns `None` when gpg is absent or key generation fails — callers should
/// skip the test gracefully in that case.
fn setup_gpg() -> Option<GpgSetup> {
    Command::new("gpg")
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())?;

    let homedir = tempfile::TempDir::new().ok()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(homedir.path(), std::fs::Permissions::from_mode(0o700)).ok()?;
    }

    let params_file = homedir.path().join("key-params");
    std::fs::write(
        &params_file,
        "%no-protection\n\
         Key-Type: EdDSA\n\
         Key-Curve: ed25519\n\
         Name-Real: Test Signer\n\
         Name-Email: test@example.com\n\
         Expire-Date: 0\n\
         %commit\n",
    )
    .ok()?;

    let out = Command::new("gpg")
        .env("GNUPGHOME", homedir.path())
        .args(["--batch", "--gen-key"])
        .arg(&params_file)
        .output()
        .ok()?;

    if !out.status.success() {
        eprintln!(
            "gpg key generation failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }

    // Extract the fingerprint from `gpg --list-secret-keys --with-colons`.
    let list_out = Command::new("gpg")
        .env("GNUPGHOME", homedir.path())
        .args(["--list-secret-keys", "--with-colons"])
        .output()
        .ok()?;
    let list_str = String::from_utf8(list_out.stdout).ok()?;
    let fingerprint = list_str
        .lines()
        .find(|l| l.starts_with("fpr:"))?
        .split(':')
        .nth(9)?
        .to_string();

    // A wrapper script that invokes gpg with our isolated homedir.
    // Git's `gpg.program` config will point at this script, so both
    // `git commit -S` and `git verify-commit` use the test keyring.
    let wrapper_path = homedir.path().join("gpg-wrapper.sh");
    std::fs::write(
        &wrapper_path,
        format!(
            "#!/bin/sh\nexec gpg --homedir '{}' \"$@\"\n",
            homedir.path().display()
        ),
    )
    .ok()?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper_path, std::fs::Permissions::from_mode(0o755)).ok()?;
    }

    Some(GpgSetup {
        _homedir: homedir,
        fingerprint,
        wrapper_path,
    })
}

// ===========================================================================
// Tests
// ===========================================================================

/// `find_repo_root` succeeds when called from a subdirectory of the library.
#[test]
fn test_find_repo_root_from_subdir() {
    let lib = setup_library();
    let subdir = lib.root.join("src");
    let found = find_repo_root(&subdir).expect("should find repo root from src/");
    assert_eq!(found, lib.root);
}

// ---------------------------------------------------------------------------
// Case 1: No version → HEAD
// ---------------------------------------------------------------------------

/// When no `@commit` suffix is given the runtime uses the HEAD sha.
/// Verify that `get_file_at_commit` at HEAD returns the latest content.
#[test]
fn test_no_version_resolves_to_head() {
    let lib = setup_library();
    let _app = setup_app(&lib);

    let content = get_file_at_commit(&lib.root, "src/mylib.cljc", &lib.commit_v2)
        .expect("should read file at HEAD sha");

    assert!(
        content.contains("hello-v2"),
        "HEAD should contain v2 content; got:\n{content}"
    );
}

/// HEAD sha is a valid commit hash as reported by `is_valid_commit_hash`.
#[test]
fn test_head_sha_is_valid_hash() {
    let lib = setup_library();
    assert!(is_valid_commit_hash(&lib.commit_v2));
    assert_eq!(lib.commit_v2.len(), 40); // full sha from git rev-parse
}

// ---------------------------------------------------------------------------
// Case 2: Tagged version → pinned commit
// ---------------------------------------------------------------------------

/// When a symbol carries `@<sha>` pointing at the v1.0.0 tag commit, the
/// resolver fetches that historical file rather than the current HEAD.
#[test]
fn test_tagged_version_resolves_to_pinned_commit() {
    let lib = setup_library();

    // Resolve the lightweight tag to its commit sha.
    // ^{} dereferences annotated tags; it is a no-op for lightweight tags.
    let tag_sha = git_sha(&lib.root, "v1.0.0^{}");
    assert_eq!(
        tag_sha, lib.commit_v1,
        "tag v1.0.0 must point at the first commit"
    );
    assert!(is_valid_commit_hash(&tag_sha));

    let content = get_file_at_commit(&lib.root, "src/mylib.cljc", &tag_sha)
        .expect("should read file at v1.0.0 tag sha");

    assert!(
        content.contains("hello-v1"),
        "v1.0.0 commit should contain v1 content; got:\n{content}"
    );
}

/// The v1 and v2 contents are distinct, confirming the pinning actually
/// retrieves a different snapshot.
#[test]
fn test_pinned_and_head_contents_differ() {
    let lib = setup_library();

    let v1 = get_file_at_commit(&lib.root, "src/mylib.cljc", &lib.commit_v1).unwrap();
    let v2 = get_file_at_commit(&lib.root, "src/mylib.cljc", &lib.commit_v2).unwrap();

    assert_ne!(v1, v2, "v1 and v2 snapshots must differ");
    assert!(v1.contains("hello-v1"));
    assert!(v2.contains("hello-v2"));
}

/// A malformed (too-short) commit hash is rejected before any git subprocess.
#[test]
fn test_invalid_commit_hash_rejected() {
    let lib = setup_library();
    let result = get_file_at_commit(&lib.root, "src/mylib.cljc", "abc123"); // 6 chars
    assert!(
        matches!(result, Err(VcsError::InvalidCommit(_))),
        "expected InvalidCommit; got {result:?}"
    );
}

/// A valid hash that does not exist in the repo returns an error.
///
/// When the path exists on disk git reports "exists on disk, but not in <sha>",
/// which `get_file_at_commit` classifies as `PathNotFound`.  When the path does
/// not exist on disk git reports "Not a valid object name", producing
/// `CommitNotFound`.  Either variant is acceptable; we just require an error.
#[test]
fn test_commit_not_found() {
    let lib = setup_library();
    // A plausible-looking but nonexistent SHA.
    let result =
        get_file_at_commit(&lib.root, "src/mylib.cljc", "deadbeefdeadbeefdeadbeef0000000000000001");
    assert!(result.is_err(), "nonexistent commit SHA should error; got Ok");
    assert!(
        matches!(
            result,
            Err(VcsError::CommitNotFound(_)) | Err(VcsError::PathNotFound(_, _))
        ),
        "expected CommitNotFound or PathNotFound; got {result:?}"
    );
}

/// A nonexistent file path at a real commit returns an error.
#[test]
fn test_path_not_found_at_commit() {
    let lib = setup_library();
    let result = get_file_at_commit(&lib.root, "src/no_such_file.cljc", &lib.commit_v2);
    assert!(result.is_err(), "missing path should error; got {result:?}");
}

// ---------------------------------------------------------------------------
// Case 3 & 4: Signature verification — positive and negative
// ---------------------------------------------------------------------------

/// Negative: `verify_commit_signature` must fail for an unsigned commit and
/// return `VcsError::SignatureVerificationFailed` with the correct sha.
#[test]
fn test_signature_verification_negative() {
    let lib = setup_library();

    let result = verify_commit_signature(&lib.root, &lib.commit_v2);

    assert!(
        result.is_err(),
        "unsigned commit should fail verification"
    );
    match result {
        Err(VcsError::SignatureVerificationFailed { commit, .. }) => {
            assert_eq!(commit, lib.commit_v2);
        }
        Err(other) => panic!("expected SignatureVerificationFailed, got {other}"),
        Ok(()) => panic!("expected Err, got Ok"),
    }
}

/// Positive: `verify_commit_signature` succeeds on a commit signed with a
/// GPG key that git can verify through its `gpg.program` configuration.
///
/// This test is skipped automatically when gpg is unavailable or key
/// generation fails (e.g. restricted CI environments).
#[test]
fn test_signature_verification_positive() {
    let gpg = match setup_gpg() {
        Some(g) => g,
        None => {
            eprintln!("SKIP test_signature_verification_positive: gpg setup unavailable");
            return;
        }
    };

    // A fresh repo so we can configure gpg.program locally without touching
    // the library or app fixtures.
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();

    git_ok(&root, &["init", "-b", "main"]);
    git_ok(&root, &["config", "user.email", "test@example.com"]);
    git_ok(&root, &["config", "user.name", "Test Signer"]);
    git_ok(&root, &["config", "commit.gpgsign", "false"]);
    git_ok(&root, &["config", "user.signingkey", &gpg.fingerprint]);
    // Point gpg.program at the wrapper so both `git commit -S` and
    // `git verify-commit` use the isolated test keyring.
    git_ok(
        &root,
        &["config", "gpg.program", gpg.wrapper_path.to_str().unwrap()],
    );

    std::fs::write(root.join("signed.txt"), "signed content\n").unwrap();
    git_ok(&root, &["add", "signed.txt"]);

    // Create a signed commit.  -S forces signing regardless of gpg.program config.
    let sign_out = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["-c", "commit.gpgsign=true", "commit", "-S", "-m", "signed commit"])
        .env("GIT_AUTHOR_NAME", "Test Signer")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test Signer")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git commit -S failed to start");

    if !sign_out.status.success() {
        eprintln!(
            "SKIP test_signature_verification_positive: signed commit failed: {}",
            String::from_utf8_lossy(&sign_out.stderr)
        );
        return;
    }

    let signed_sha = git_sha(&root, "HEAD");

    // verify_commit_signature calls `git verify-commit` which picks up
    // gpg.program from the repo's local config and uses our test keyring.
    let result = verify_commit_signature(&root, &signed_sha);
    assert!(
        result.is_ok(),
        "GPG-signed commit should verify successfully; got: {:?}",
        result.err()
    );
}

// ---------------------------------------------------------------------------
// App ↔ Library integration
// ---------------------------------------------------------------------------

/// The app's `cljrs.edn` correctly references the library repo path and pins
/// it to the HEAD sha.
#[test]
fn test_app_deps_edn_references_library() {
    let lib = setup_library();
    let app = setup_app(&lib);

    let edn = std::fs::read_to_string(app.root.join("cljrs.edn"))
        .expect("cljrs.edn should exist in app repo");

    assert!(edn.contains("mylib"), "deps must include mylib");
    assert!(
        edn.contains(&lib.commit_v2),
        "deps must pin to the library HEAD sha"
    );
    assert!(
        edn.contains(lib.root.to_str().unwrap()),
        "deps must reference the library repo path"
    );
}

/// The app can independently load the library at v1 (pinned) and at HEAD,
/// simulating `(require '[mylib@<sha>])` vs `(require '[mylib])`.
#[test]
fn test_app_loads_library_at_pinned_and_head_commits() {
    let lib = setup_library();
    let _app = setup_app(&lib);

    // Simulate (require '[mylib@<commit_v1>]) — pinned to the tagged commit.
    let pinned =
        get_file_at_commit(&lib.root, "src/mylib.cljc", &lib.commit_v1).expect("v1 must load");
    assert!(pinned.contains("hello-v1"));

    // Simulate (require '[mylib]) — no version, resolves to HEAD.
    let head =
        get_file_at_commit(&lib.root, "src/mylib.cljc", &lib.commit_v2).expect("HEAD must load");
    assert!(head.contains("hello-v2"));

    assert_ne!(pinned, head, "pinned and HEAD snapshots must differ");
}
