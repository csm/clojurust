//! Tier 1 (JIT) versioned symbol resolution, end to end.
//!
//! Boots the full stdlib environment with the JIT enabled at a tiny
//! threshold, defines functions whose bodies reference pinned symbols
//! (`mylib/the-answer@<sha>`), hammers them until the background worker
//! publishes native code, and asserts:
//!
//! - the JIT-native result equals the interpreter result (tier consistency)
//!   and sees the pinned value, not HEAD;
//! - the per-call-site inline cache's permanently rooted value survives a
//!   forced GC;
//! - the HEAD binding stays untouched throughout.

use std::path::{Path, PathBuf};
use std::process::Command;

use cljrs_eval::{Env, eval};
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

fn eval_str(env: &mut Env, src: &str) -> Value {
    let mut parser = cljrs_reader::Parser::new(src.to_string(), "<test>".to_string());
    let forms = parser.parse_all().expect("parse");
    let mut last = Value::Nil;
    for form in &forms {
        let _alloc_frame = cljrs_gc::push_alloc_frame();
        last = eval(form, env).unwrap_or_else(|e| panic!("eval of {src:?} failed: {e:?}"));
    }
    last
}

/// The `ir_arity_id` of the sole arity of the fn bound to `user/<name>`.
fn arity_id_of(env: &Env, name: &str) -> u64 {
    let val = env
        .globals
        .lookup_in_ns("user", name)
        .unwrap_or_else(|| panic!("user/{name} not bound"));
    match val {
        Value::Fn(f) => f.get().arities[0].ir_arity_id,
        other => panic!("user/{name} is not a fn: {other:?}"),
    }
}

/// Call `(fn-name)` repeatedly until the JIT publishes native code for it
/// (or panic after ~15s).  Returns the last interpreted result.
fn hammer_until_native(env: &mut Env, fn_name: &str, arity_id: u64) -> Value {
    let call = format!("({fn_name})");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let mut last = Value::Nil;
    while cljrs_eval::jit_state::get_native_fn(arity_id).is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "JIT never published native code for {fn_name} (arity {arity_id})"
        );
        last = eval_str(env, &call);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    last
}

// ── The test ──────────────────────────────────────────────────────────────────

/// Single test (not one per scenario): `cljrs_jit::init` installs process
/// globals, so scenarios share the booted environment.
#[test]
fn jit_native_code_resolves_pinned_symbols() {
    let repo = make_lib_repo();

    // Tiny threshold so the worker kicks in after a handful of calls; must be
    // set before init.  init() also forces eager IR lowering.
    cljrs_eval::jit_state::set_jit_threshold(3);
    cljrs_jit::init();

    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_stdlib::standard_env_with_paths(vec![repo.src_dir.clone()]);

    // Wait for the background compiler-namespace load (see
    // compiler_clojure_tests.rs for why).
    while !globals
        .compiler_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut env = Env::new(globals.clone(), "user");
    cljrs_env::callback::push_eval_context(&env);

    eval_str(&mut env, "(require 'mylib)");
    assert_eq!(eval_str(&mut env, "mylib/the-answer"), Value::Long(2));

    // A fn whose body is a pinned value reference, and one that calls a
    // pinned fn from the versioned namespace (exercises rt_call through the
    // versioned-IC-loaded callee).
    eval_str(
        &mut env,
        &format!("(defn hot-val [] mylib/the-answer@{})", repo.commit_v1),
    );
    eval_str(
        &mut env,
        &format!("(defn hot-call [] (mylib/describe@{}))", repo.commit_v1),
    );

    // Scenario 1: pinned value reference.
    let val_id = arity_id_of(&env, "hot-val");
    let interpreted = hammer_until_native(&mut env, "hot-val", val_id);
    assert_eq!(interpreted, Value::Long(1), "interpreted tiers see v1");
    let native = eval_str(&mut env, "(hot-val)");
    assert_eq!(native, Value::Long(1), "JIT-native code sees v1");

    // Scenario 2: pinned fn call.
    let call_id = arity_id_of(&env, "hot-call");
    let interpreted = hammer_until_native(&mut env, "hot-call", call_id);
    assert_eq!(interpreted, Value::string("v1"));
    assert_eq!(eval_str(&mut env, "(hot-call)"), Value::string("v1"));

    // The IC-cached values are permanently rooted: a forced collection must
    // not invalidate what native code reads from its cache slots.
    cljrs_env::gc_roots::force_collect(&env);
    assert_eq!(eval_str(&mut env, "(hot-val)"), Value::Long(1));
    assert_eq!(eval_str(&mut env, "(hot-call)"), Value::string("v1"));

    // HEAD is untouched throughout.
    assert_eq!(eval_str(&mut env, "mylib/the-answer"), Value::Long(2));
    assert_eq!(eval_str(&mut env, "(mylib/describe)"), Value::string("v2"));

    cljrs_env::callback::pop_eval_context();
}
