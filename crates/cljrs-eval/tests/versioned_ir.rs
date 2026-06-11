//! Tier 2 (IR interpreter) versioned symbol resolution.
//!
//! Drives the ANF lowering + IR interpreter directly against a real git
//! fixture, asserting that:
//!
//! - a `LoadGlobal` whose name carries an `@<sha>` suffix resolves through
//!   the shared versioned resolver (`cljrs_env::versioned`) to the pinned
//!   value, leaving the HEAD binding untouched;
//! - code lowered *inside* a versioned namespace (`defining_ns = "mylib@sha"`)
//!   resolves both unqualified and base-qualified same-namespace references
//!   at the pinned commit, including lazily loading the versioned namespace
//!   on first use.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cljrs_env::env::GlobalEnv;
use cljrs_eval::{Env, ir_interp::interpret_ir};
use cljrs_ir::lower::lower_fn_body;
use cljrs_reader::{Form, Parser};
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

/// `src/mylib.cljrs` — v1: `the-answer` = 1; v2 (HEAD): `the-answer` = 2.
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
    std::fs::write(&lib_file, "(def the-answer 1)\n").unwrap();
    git_ok(&root, &["add", "."]);
    git_ok(&root, &["commit", "-q", "-m", "v1"]);
    let commit_v1 = git_sha(&root, "HEAD");

    std::fs::write(&lib_file, "(def the-answer 2)\n").unwrap();
    git_ok(&root, &["add", "."]);
    git_ok(&root, &["commit", "-q", "-m", "v2"]);

    LibRepo {
        _dir: dir,
        src_dir,
        commit_v1,
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn parse_body(src: &str) -> Vec<Form> {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_all().expect("parse")
}

fn make_globals(src_dir: &Path) -> Arc<GlobalEnv> {
    cljrs_interp::standard_env_with_paths(None, None, None, vec![src_dir.to_path_buf()])
}

fn eval_tree_walk(globals: &Arc<GlobalEnv>, src: &str) -> Value {
    let mut env = Env::new(globals.clone(), "user");
    let forms = parse_body(src);
    let mut last = Value::Nil;
    for form in &forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        last = cljrs_interp::eval::eval(form, &mut env)
            .unwrap_or_else(|e| panic!("eval of {src:?} failed: {e:?}"));
    }
    last
}

/// Lower `body_src` as a zero-arg fn body in namespace `ns`, then run it
/// through the IR interpreter.
fn run_ir(globals: &Arc<GlobalEnv>, ns: &str, body_src: &str) -> Value {
    let _mutator = cljrs_gc::register_mutator();
    let body = parse_body(body_src);
    let ir = lower_fn_body(Some("test"), ns, &[], &body, false).expect("lower");

    let mut env = Env::new(globals.clone(), ns);
    let ns_arc: Arc<str> = Arc::from(ns);
    cljrs_env::callback::push_eval_context(&env);
    let result = interpret_ir(&ir, vec![], globals, &ns_arc, &mut env);
    cljrs_env::callback::pop_eval_context();
    result.unwrap_or_else(|e| panic!("IR interpret of {body_src:?} failed: {e:?}"))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

/// An explicitly versioned reference in IR (`LoadGlobal` with `name@sha`)
/// resolves to the pinned value without clobbering HEAD.
#[test]
fn ir_resolves_pinned_symbol() {
    let repo = make_lib_repo();
    let globals = make_globals(&repo.src_dir);

    eval_tree_walk(&globals, "(require 'mylib)");
    assert_eq!(eval_tree_walk(&globals, "mylib/the-answer"), Value::Long(2));

    let pinned = run_ir(
        &globals,
        "user",
        &format!("mylib/the-answer@{}", repo.commit_v1),
    );
    assert_eq!(pinned, Value::Long(1), "IR must see the pinned value");

    // HEAD must be untouched, and the IR tier must agree with it.
    assert_eq!(eval_tree_walk(&globals, "mylib/the-answer"), Value::Long(2));
    assert_eq!(run_ir(&globals, "user", "mylib/the-answer"), Value::Long(2));
}

/// IR lowered inside a versioned namespace resolves unqualified same-ns
/// references at the pinned commit, lazily loading `mylib@sha` on first use.
#[test]
fn ir_in_versioned_ns_resolves_unqualified_at_commit() {
    let repo = make_lib_repo();
    let globals = make_globals(&repo.src_dir);
    eval_tree_walk(&globals, "(require 'mylib)");

    let versioned_ns = format!("mylib@{}", repo.commit_v1);
    // The versioned namespace has not been loaded yet — the IR LoadGlobal
    // must trigger the lazy load.
    assert!(!globals.is_loaded(&versioned_ns));

    let val = run_ir(&globals, &versioned_ns, "the-answer");
    assert_eq!(val, Value::Long(1));
    assert!(globals.is_loaded(&versioned_ns));
}

/// IR lowered inside a versioned namespace rewrites base-qualified
/// self-references (`mylib/x` in `mylib@sha`) to the versioned namespace.
#[test]
fn ir_in_versioned_ns_resolves_qualified_self_ref_at_commit() {
    let repo = make_lib_repo();
    let globals = make_globals(&repo.src_dir);
    eval_tree_walk(&globals, "(require 'mylib)");

    let versioned_ns = format!("mylib@{}", repo.commit_v1);
    let val = run_ir(&globals, &versioned_ns, "mylib/the-answer");
    assert_eq!(
        val,
        Value::Long(1),
        "qualified self-ref must see the pinned value"
    );

    // Cross-namespace references from versioned code still see HEAD.
    let head = run_ir(&globals, "user", "mylib/the-answer");
    assert_eq!(head, Value::Long(2));
}
