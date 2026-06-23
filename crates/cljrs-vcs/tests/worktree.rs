//! Integration test for `worktree_at_commit`: materializing a files-only
//! working checkout of a pinned commit from the local bare cache, with no
//! network access.

use std::path::Path;
use std::process::Command;

fn git_ok(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command failed to start");
    assert!(
        out.status.success(),
        "git {args:?} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_sha(dir: &Path, rev: &str) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", rev])
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

#[test]
fn worktree_materializes_pinned_source_from_cache() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: git binary not available");
        return;
    }

    // Source repo with a file whose contents change between commits.
    let repo = tempfile::tempdir().unwrap();
    let root = repo.path();
    git_ok(root, &["init", "-q", "-b", "main"]);
    git_ok(root, &["config", "commit.gpgsign", "false"]);
    std::fs::write(root.join("hello.txt"), "v1\n").unwrap();
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-q", "-m", "v1"]);
    let sha_v1 = git_sha(root, "HEAD");
    std::fs::write(root.join("hello.txt"), "v2\n").unwrap();
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-q", "-m", "v2"]);

    // Hermetic cache home.
    let home = tempfile::tempdir().unwrap();
    // SAFETY: single test in this binary; no concurrent env readers.
    unsafe {
        std::env::set_var("HOME", home.path());
    }

    let url = root.to_string_lossy().to_string();

    // Without a populated cache, the worktree cannot be materialized.
    assert!(
        cljrs_vcs::worktree_at_commit(&url, &sha_v1).is_err(),
        "worktree must fail before the bare cache is populated"
    );

    // Populate the bare cache, then materialize the pinned (v1) worktree.
    cljrs_vcs::fetch_remote(&url, &sha_v1).expect("fetch into cache");
    let wt = cljrs_vcs::worktree_at_commit(&url, &sha_v1).expect("materialize worktree");

    let contents = std::fs::read_to_string(wt.join("hello.txt")).expect("checked-out file");
    assert_eq!(
        contents, "v1\n",
        "worktree must hold the pinned commit's tree"
    );

    // The checkout is files-only (no `.git`).
    assert!(!wt.join(".git").exists(), "worktree should have no .git");

    // Idempotent: a second call returns the same cached path.
    let wt2 = cljrs_vcs::worktree_at_commit(&url, &sha_v1).expect("cached worktree");
    assert_eq!(wt, wt2);
}
