//! End-to-end versioned symbol resolution through the tree-walking
//! interpreter, against a real git repository fixture.
//!
//! Guards the core semantics of the shared resolver in
//! `cljrs_env::versioned`:
//!
//! - a pinned symbol (`ns/name@sha`) resolves to the value at that commit;
//! - resolving a pinned symbol must NOT clobber the HEAD binding (the
//!   historical `def` interns into the immutable `ns@sha` namespace, never
//!   into the live one);
//! - same-namespace helpers referenced by a pinned function resolve at the
//!   pinned commit (structural inheritance via the versioned namespace);
//! - resolved versioned values survive a forced GC.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cljrs_env::env::{Env, GlobalEnv};
use cljrs_value::Value;

// ── Git fixture helpers ───────────────────────────────────────────────────────

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

fn git_sha(dir: &Path, rev: &str) -> String {
    let out = git_cmd(dir, &["rev-parse", rev]);
    assert!(out.status.success(), "git rev-parse {rev} failed");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

struct LibRepo {
    _dir: tempfile::TempDir,
    src_dir: PathBuf,
    commit_v1: String,
}

/// Library repo with two commits of `src/mylib.cljrs`:
/// v1: `the-answer` = 1; v2 (HEAD): `the-answer` = 2.
/// `describe` returns `"v" + the-answer` via an unqualified same-ns ref.
fn make_lib_repo() -> LibRepo {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().to_path_buf();
    let src_dir = root.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();

    git_ok(&root, &["init", "-q", "-b", "main"]);
    // Override any global commit.gpgsign so test commits don't invoke a
    // signing server.
    git_ok(&root, &["config", "commit.gpgsign", "false"]);

    let lib_file = src_dir.join("mylib.cljrs");
    std::fs::write(
        &lib_file,
        "(def the-answer 1)\n(defn describe [] (str \"v\" the-answer))\n",
    )
    .unwrap();
    git_ok(&root, &["add", "."]);
    git_ok(&root, &["commit", "-q", "-m", "v1"]);
    let commit_v1 = git_sha(&root, "HEAD");

    std::fs::write(
        &lib_file,
        "(def the-answer 2)\n(defn describe [] (str \"v\" the-answer))\n",
    )
    .unwrap();
    git_ok(&root, &["add", "."]);
    git_ok(&root, &["commit", "-q", "-m", "v2"]);

    LibRepo {
        _dir: dir,
        src_dir,
        commit_v1,
    }
}

// ── Eval helpers ──────────────────────────────────────────────────────────────

fn make_env(src_dir: &Path) -> (Arc<GlobalEnv>, Env) {
    let globals =
        cljrs_interp::standard_env_with_paths(None, None, None, vec![src_dir.to_path_buf()]);
    let env = Env::new(globals.clone(), "user");
    (globals, env)
}

fn eval_str(env: &mut Env, src: &str) -> Value {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut last = Value::Nil;
    for form in &forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        last = cljrs_interp::eval::eval(form, env)
            .unwrap_or_else(|e| panic!("eval of {src:?} failed: {e:?}"));
    }
    last
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// Resolving a pinned symbol returns the historical value and leaves the
/// HEAD binding untouched (regression: the historical `def` used to intern
/// into the live namespace, clobbering HEAD).
#[test]
fn pinned_symbol_does_not_clobber_head() {
    let repo = make_lib_repo();
    let (_globals, mut env) = make_env(&repo.src_dir);

    eval_str(&mut env, "(require 'mylib)");
    assert_eq!(eval_str(&mut env, "mylib/the-answer"), Value::Long(2));

    let pinned = eval_str(&mut env, &format!("mylib/the-answer@{}", repo.commit_v1));
    assert_eq!(pinned, Value::Long(1), "pinned symbol must see v1");

    // HEAD must still be v2 after the versioned resolution.
    assert_eq!(
        eval_str(&mut env, "mylib/the-answer"),
        Value::Long(2),
        "resolving a pinned symbol must not clobber the HEAD binding"
    );
}

/// A pinned function's unqualified same-namespace references resolve at the
/// pinned commit (the whole namespace is loaded as `mylib@sha`).
#[test]
fn pinned_fn_sees_same_ns_helpers_at_commit() {
    let repo = make_lib_repo();
    let (_globals, mut env) = make_env(&repo.src_dir);

    eval_str(&mut env, "(require 'mylib)");
    let head = eval_str(&mut env, "(mylib/describe)");
    assert_eq!(head, Value::string("v2"));

    let pinned = eval_str(&mut env, &format!("(mylib/describe@{})", repo.commit_v1));
    assert_eq!(pinned, Value::string("v1"));
}

/// A versioned require pins the whole namespace under an alias.
#[test]
fn versioned_require_pins_namespace() {
    let repo = make_lib_repo();
    let (_globals, mut env) = make_env(&repo.src_dir);

    eval_str(
        &mut env,
        &format!("(require '[mylib@{} :as v1])", repo.commit_v1),
    );
    assert_eq!(eval_str(&mut env, "v1/the-answer"), Value::Long(1));
    assert_eq!(eval_str(&mut env, "(v1/describe)"), Value::string("v1"));
}

/// Versioned values survive a forced collection: vars in the versioned
/// namespace are traced via the namespace table, and native-fallback values
/// are traced via the version cache.
#[test]
fn versioned_values_survive_gc() {
    let repo = make_lib_repo();
    let (globals, mut env) = make_env(&repo.src_dir);

    eval_str(&mut env, "(require 'mylib)");
    let pinned_expr = format!("(mylib/describe@{})", repo.commit_v1);
    assert_eq!(eval_str(&mut env, &pinned_expr), Value::string("v1"));

    // Native fallback path: a pure-Rust namespace with no Clojure source.
    let nf = cljrs_value::NativeFn {
        name: Arc::from("native-fn"),
        arity: cljrs_value::Arity::Fixed(0),
        func: Arc::new(|_args| Ok(Value::Long(7))),
    };
    globals.get_or_create_ns("purelib");
    globals.intern(
        "purelib",
        Arc::from("native-fn"),
        Value::NativeFunction(cljrs_gc::GcPtr::new(nf)),
    );
    assert_eq!(
        eval_str(&mut env, "(purelib/native-fn@abcdef1234567)"),
        Value::Long(7)
    );

    cljrs_env::gc_roots::force_collect(&env);

    // Both the versioned-namespace value and the cache-only native fallback
    // must still be alive and callable after collection.
    assert_eq!(eval_str(&mut env, &pinned_expr), Value::string("v1"));
    assert_eq!(
        eval_str(&mut env, "(purelib/native-fn@abcdef1234567)"),
        Value::Long(7)
    );
}
