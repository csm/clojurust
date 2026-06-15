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
    TrustedKeys, VcsError, find_repo_root, get_file_at_commit, is_valid_commit_hash,
    verify_commit_signature,
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
// Native SSH signing fixture
// ---------------------------------------------------------------------------

// A static Ed25519 keypair (generated once with `ssh-key`) used to produce a
// natively SSH-signed commit. No external `gpg`/`ssh-keygen` is involved.
const TEST_SSH_PRIVATE: &str = "\
-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACBmFUP3SDH5k28ErT2na8g4asrcsI4STLcmDImAF0WjDwAAAIiFW+7uhVvu
7gAAAAtzc2gtZWQyNTUxOQAAACBmFUP3SDH5k28ErT2na8g4asrcsI4STLcmDImAF0WjDw
AAAEAgsZE1vrnYoatnjRDx6BGE9PeOViG9mgDVkCbPj8unnmYVQ/dIMfmTbwStPadryDhq
ytywjhJMtyYMiYAXRaMPAAAAAAECAwQF
-----END OPENSSH PRIVATE KEY-----
";
const TEST_SSH_PUBLIC: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGYVQ/dIMfmTbwStPadryDhqytywjhJMtyYMiYAXRaMP cljrs-test";

/// Sign `payload` with the static SSH key under git's "git" namespace, returning
/// the armored SSHSIG block.
fn ssh_sign(payload: &[u8]) -> String {
    let key = ssh_key::PrivateKey::from_openssh(TEST_SSH_PRIVATE).expect("parse private key");
    let sig = ssh_key::SshSig::sign(&key, "git", ssh_key::HashAlg::Sha512, payload).expect("sign");
    sig.to_pem(ssh_key::LineEnding::LF)
        .expect("pem")
        .to_string()
}

/// Build a raw commit object embedding `armored_sig` in the `gpgsig` header.
/// `payload` is the no-signature commit text.
fn embed_gpgsig(payload: &[u8], armored_sig: &str) -> Vec<u8> {
    let split = payload
        .windows(2)
        .position(|w| w == b"\n\n")
        .expect("payload has a header/message separator");
    let headers = &payload[..=split];
    let message = &payload[split + 1..];

    let mut out = Vec::new();
    out.extend_from_slice(headers);
    out.extend_from_slice(b"gpgsig ");
    for (i, line) in armored_sig.lines().enumerate() {
        if i > 0 {
            out.push(b'\n');
            out.push(b' ');
        }
        out.extend_from_slice(line.as_bytes());
    }
    out.push(b'\n');
    out.extend_from_slice(message);
    out
}

/// Write a raw object of type `kind` into `dir`'s object store via
/// `git hash-object -w` (used only to store the fixture; signing/verifying is
/// native). Returns the object's SHA.
fn git_write_object(dir: &Path, kind: &str, bytes: &[u8]) -> String {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["hash-object", "-w", "-t", kind, "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn git hash-object");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(bytes)
        .expect("write object to git");
    let out = child.wait_with_output().expect("git hash-object");
    assert!(
        out.status.success(),
        "git hash-object failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().to_string()
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
    let result = get_file_at_commit(
        &lib.root,
        "src/mylib.cljc",
        "deadbeefdeadbeefdeadbeef0000000000000001",
    );
    assert!(
        result.is_err(),
        "nonexistent commit SHA should error; got Ok"
    );
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

    let result = verify_commit_signature(&lib.root, &lib.commit_v2, &TrustedKeys::new());

    assert!(result.is_err(), "unsigned commit should fail verification");
    match result {
        Err(VcsError::SignatureVerificationFailed { commit, reason }) => {
            assert_eq!(commit, lib.commit_v2);
            assert!(reason.contains("not signed"), "reason: {reason}");
        }
        Err(other) => panic!("expected SignatureVerificationFailed, got {other}"),
        Ok(()) => panic!("expected Err, got Ok"),
    }
}

/// Positive: `verify_commit_signature` succeeds on a commit natively signed with
/// an SSH key, when that key is present in the trusted set. The signed commit
/// object is constructed and signed in-process (no `gpg`/`ssh-keygen`); git is
/// used only to store the object in the repository.
#[test]
fn test_signature_verification_positive() {
    let dir = tempfile::TempDir::new().unwrap();
    let root = dir.path().to_path_buf();
    git_ok(&root, &["init", "-b", "main"]);

    // Store an empty tree so the commit object references a real object.
    let tree = git_write_object(&root, "tree", b"");

    let payload = format!(
        "tree {tree}\n\
         author Test <test@example.com> 0 +0000\n\
         committer Test <test@example.com> 0 +0000\n\
         \n\
         signed commit\n"
    );
    let sig = ssh_sign(payload.as_bytes());
    let raw = embed_gpgsig(payload.as_bytes(), &sig);
    let signed_sha = git_write_object(&root, "commit", &raw);

    let mut trusted = TrustedKeys::new();
    trusted.add_ssh_openssh(TEST_SSH_PUBLIC).unwrap();

    let result = verify_commit_signature(&root, &signed_sha, &trusted);
    assert!(
        result.is_ok(),
        "SSH-signed commit should verify; got: {:?}",
        result.err()
    );

    // The same commit must fail when the signing key is not trusted.
    let untrusted = verify_commit_signature(&root, &signed_sha, &TrustedKeys::new());
    assert!(
        matches!(untrusted, Err(VcsError::SignatureVerificationFailed { .. })),
        "empty trust set must reject; got {untrusted:?}"
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
