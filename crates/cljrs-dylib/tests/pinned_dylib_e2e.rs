//! End-to-end test for pinned native packages (`:rust/load :dylib`).
//!
//! Heavyweight: builds a wrapper cdylib with cargo (compiling
//! `cljrs-interop` and its dependency tree in release mode), so it only
//! runs when `CLJRS_DYLIB_E2E=1` is set:
//!
//! ```sh
//! CLJRS_DYLIB_E2E=1 cargo test -p cljrs-dylib --test pinned_dylib_e2e
//! ```
//!
//! Fixture: a git repository holding a tiny native crate (`pinlib`) whose
//! `cljrs_init` defines `pinlib/build-tag`.  Commit v1 returns 1; HEAD
//! returns 2.  The host also registers its own (HEAD) implementation
//! returning 99.  Resolving `pinlib/build-tag@<sha1>` must load the dylib
//! built from commit v1 and return 1, leaving the host's binding untouched.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use cljrs_value::Value;

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

/// Locate the clojurust workspace root (this crate is `<root>/crates/cljrs-dylib`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn pinlib_source(tag: i64) -> String {
    format!(
        r#"use cljrs_interop::{{Registry, wrap_fn0}};

pub fn cljrs_init(registry: &mut Registry) {{
    registry.define(
        "pinlib/build-tag",
        wrap_fn0("build-tag", || Ok::<i64, String>({tag})),
    );
}}
"#
    )
}

/// Build the two-commit native-crate fixture repo; returns `(dir, sha_v1)`.
fn make_pinlib_repo(ws_root: &Path) -> (tempfile::TempDir, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();

    git_ok(root, &["init", "-q", "-b", "main"]);
    // Override any global commit.gpgsign so test commits don't invoke a
    // signing server.
    git_ok(root, &["config", "commit.gpgsign", "false"]);

    // The fixture pins cljrs-interop by absolute path into this workspace so
    // the wrapper (which uses the same path) unifies on one crate instance.
    let cargo_toml = format!(
        r#"[package]
name = "pinlib"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
cljrs-interop = {{ path = "{}" }}
"#,
        ws_root.join("crates/cljrs-interop").display()
    );
    std::fs::write(root.join("Cargo.toml"), cargo_toml).unwrap();

    std::fs::write(root.join("src/lib.rs"), pinlib_source(1)).unwrap();
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-q", "-m", "v1"]);
    let sha_v1 = git_sha(root, "HEAD");

    std::fs::write(root.join("src/lib.rs"), pinlib_source(2)).unwrap();
    git_ok(root, &["add", "."]);
    git_ok(root, &["commit", "-q", "-m", "v2"]);

    (dir, sha_v1)
}

#[test]
fn pinned_native_dylib_end_to_end() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!("skipping pinned_native_dylib_end_to_end (set CLJRS_DYLIB_E2E=1 to run)");
        return;
    }

    let ws_root = workspace_root();
    let (repo, sha_v1) = make_pinlib_repo(&ws_root);

    // Hermetic dylib/git caches + a pinned workspace for the wrapper deps.
    let cache_home = tempfile::tempdir().unwrap();
    // SAFETY: single gated test in this binary; no concurrent env readers.
    unsafe {
        std::env::set_var("HOME", cache_home.path());
        std::env::set_var("CLJRS_WORKSPACE_ROOT", &ws_root);
    }

    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_interp::standard_env_minimal(None, None, None);

    // Host's own (HEAD) implementation — must remain untouched.
    let head_fn = cljrs_value::NativeFn {
        name: Arc::from("build-tag"),
        arity: cljrs_value::Arity::Fixed(0),
        func: Arc::new(|_args| Ok(Value::Long(99))),
    };
    globals.get_or_create_ns("pinlib");
    globals.intern(
        "pinlib",
        Arc::from("build-tag"),
        Value::NativeFunction(cljrs_gc::GcPtr::new(head_fn)),
    );

    // cljrs.edn equivalent: pinlib is a git dep with :rust/load :dylib.
    let config = cljrs_deps::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_deps::Dependency::Git(cljrs_deps::GitDep {
                url: Arc::from(repo.path().to_string_lossy().as_ref()),
                sha: Arc::from(sha_v1.as_str()),
                rust_init: Some(Arc::from("pinlib::cljrs_init")),
                rust_crate_dir: None,
                rust_load_dylib: true,
            }),
        )],
        ..Default::default()
    };
    *globals.deps_config.write().unwrap() = Some(Arc::new(config));

    cljrs_dylib::install(&globals);

    // Resolve the pinned symbol: must build + load the v1 dylib.
    let resolved = cljrs_env::versioned::resolve_versioned_value(
        &globals,
        "user",
        Some("pinlib"),
        "build-tag",
        &sha_v1,
    )
    .expect("pinned native resolution should succeed");
    let Value::NativeFunction(nf) = &resolved else {
        panic!("expected a native fn, got {resolved:?}");
    };
    let result = (nf.get().func)(&[]).expect("pinned fn call");
    assert_eq!(result, Value::Long(1), "pinned dylib must be built from v1");

    // The versioned namespace holds the pinned impl; HEAD is untouched.
    let versioned_ns = format!("pinlib@{sha_v1}");
    assert!(globals.is_loaded(&versioned_ns));
    let head = globals.lookup_in_ns("pinlib", "build-tag").unwrap();
    let Value::NativeFunction(head_nf) = &head else {
        panic!("HEAD binding missing");
    };
    assert_eq!((head_nf.get().func)(&[]).unwrap(), Value::Long(99));

    // Second resolution is served from the already-loaded namespace (no
    // rebuild): same value.
    let again = cljrs_env::versioned::resolve_versioned_value(
        &globals,
        "user",
        Some("pinlib"),
        "build-tag",
        &sha_v1,
    )
    .expect("cached pinned resolution");
    let Value::NativeFunction(nf2) = &again else {
        panic!("expected a native fn");
    };
    assert_eq!((nf2.get().func)(&[]).unwrap(), Value::Long(1));
}

/// A `:rust/load :dylib` dependency is brought in by a **plain `require`** of
/// its namespace (no versioned symbol literal): the dep's crate is built at
/// its pinned `:git/sha` and its exports land in the live, unversioned
/// namespace, so `pinlib/build-tag` resolves to the pinned implementation.
///
/// Regression test for the native-deps `require` gap (issue #222).
#[test]
fn native_dep_loaded_by_plain_require() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!("skipping native_dep_loaded_by_plain_require (set CLJRS_DYLIB_E2E=1 to run)");
        return;
    }

    let ws_root = workspace_root();
    let (repo, sha_v1) = make_pinlib_repo(&ws_root);

    // Hermetic dylib/git caches + a pinned workspace for the wrapper deps.
    let cache_home = tempfile::tempdir().unwrap();
    // SAFETY: single gated test invocation; no concurrent env readers.
    unsafe {
        std::env::set_var("HOME", cache_home.path());
        std::env::set_var("CLJRS_WORKSPACE_ROOT", &ws_root);
    }

    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_interp::standard_env_minimal(None, None, None);

    // cljrs.edn equivalent: pinlib is a git dep with :rust/load :dylib, pinned
    // at the v1 commit.  No Clojure source on the path provides this namespace.
    let config = cljrs_deps::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_deps::Dependency::Git(cljrs_deps::GitDep {
                url: Arc::from(repo.path().to_string_lossy().as_ref()),
                sha: Arc::from(sha_v1.as_str()),
                rust_init: Some(Arc::from("pinlib::cljrs_init")),
                rust_crate_dir: None,
                rust_load_dylib: true,
            }),
        )],
        ..Default::default()
    };
    *globals.deps_config.write().unwrap() = Some(Arc::new(config));

    cljrs_dylib::install(&globals);

    // A plain `(require '[pinlib :as pl])` must build + load the native dep.
    let spec = cljrs_env::env::RequireSpec {
        ns: Arc::from("pinlib"),
        version: None,
        alias: Some(Arc::from("pl")),
        refer: cljrs_env::env::RequireRefer::None,
    };
    cljrs_env::loader::load_ns(globals.clone(), &spec, "user")
        .expect("plain require of a native dep should succeed");

    // The unversioned namespace is now loaded and carries the dylib's export.
    assert!(globals.is_loaded("pinlib"));
    let f = globals
        .lookup_in_ns("pinlib", "build-tag")
        .expect("pinlib/build-tag must be registered by the dylib");
    let Value::NativeFunction(nf) = &f else {
        panic!("expected a native fn, got {f:?}");
    };
    assert_eq!(
        (nf.get().func)(&[]).unwrap(),
        Value::Long(1),
        "native dep must be built from the pinned v1 commit"
    );

    // The alias resolves the unversioned namespace.
    assert_eq!(
        globals.resolve_alias("user", "pl").as_deref(),
        Some("pinlib")
    );
}
