//! End-to-end test for plain `require` of a **pure-Clojure git dependency**
//! declared in `cljrs.edn` (issue #222, Root Cause #2).
//!
//! Before the fix, a namespace provided only by a `:deps` entry could not be
//! `require`d — its source was never added to the source path, and the fetch
//! cache was a bare repo with no working tree.  This test stands up a real
//! (local) git repo holding a dependency library, declares it in a consuming
//! project's `cljrs.edn`, runs `cljrs deps fetch`, and then `cljrs run`s a file
//! that `require`s the dependency's namespace and calls into it.
//!
//! Uses a local-path git remote, so no network access is needed; it does
//! require the `git` binary (already a test-time assumption elsewhere in the
//! workspace).

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

/// Build a tiny git repo whose `src/greetlib.cljrs` defines `greetlib/hello`,
/// with a `cljrs.edn` declaring `:paths ["src"]`.  Returns `(dir, sha)`.
fn make_dep_repo() -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();

    git_ok(root, &["init", "-q", "-b", "main"]);
    git_ok(root, &["config", "commit.gpgsign", "false"]);

    std::fs::write(root.join("cljrs.edn"), "{:paths [\"src\"]}\n").unwrap();
    std::fs::write(
        root.join("src/greetlib.cljrs"),
        r#"(ns greetlib)
(defn hello [who] (str "hello, " who "!"))
"#,
    )
    .unwrap();

    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-q", "-m", "v1"]);
    let sha = git_sha(root, "HEAD");
    (dir, sha)
}

#[test]
fn pure_clojure_git_dep_loaded_by_plain_require() {
    if Command::new("git").arg("--version").output().is_err() {
        eprintln!("skipping: git binary not available");
        return;
    }

    let (dep_repo, sha) = make_dep_repo();

    // Isolated HOME so the dep cache (~/.cljrs/cache) is hermetic.
    let home = tempfile::tempdir().unwrap();

    // Consuming project: cljrs.edn declares the git dep by local path; main
    // requires the dep's namespace and uses it.
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::write(
        project.path().join("cljrs.edn"),
        format!(
            "{{:deps {{greetlib {{:git/url {:?} :git/sha {:?}}}}}\n :paths [\"src\"]}}\n",
            dep_repo.path().to_string_lossy(),
            sha,
        ),
    )
    .unwrap();
    std::fs::write(
        project.path().join("src/main.cljrs"),
        r#"(ns demo)
(require '[greetlib :as g])
(defn -main [& _] (println (g/hello "world")))
"#,
    )
    .unwrap();

    // 1. Fetch the dep into the (hermetic) cache.
    let fetch = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .current_dir(project.path())
        .env("HOME", home.path())
        .args(["deps", "fetch"])
        .output()
        .expect("run cljrs deps fetch");
    assert!(
        fetch.status.success(),
        "deps fetch failed:\n{}",
        String::from_utf8_lossy(&fetch.stderr)
    );

    // 2. Run a program that requires the dep's namespace.
    let run = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .current_dir(project.path())
        .env("HOME", home.path())
        .args(["run", "src/main.cljrs"])
        .output()
        .expect("run cljrs");
    let stdout = String::from_utf8_lossy(&run.stdout);
    assert!(
        run.status.success(),
        "run failed:\nstdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        stdout.contains("hello, world!"),
        "expected dep namespace to load and run; stdout was: {stdout}"
    );
}
