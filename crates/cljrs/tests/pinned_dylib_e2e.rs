//! End-to-end test for pinned native packages (`:rust/load :dylib`).
//!
//! Heavyweight: builds a wrapper cdylib with cargo (compiling
//! `cljrs-interop` and its dependency tree in release mode), so it only
//! runs when `CLJRS_DYLIB_E2E=1` is set:
//!
//! ```sh
//! CLJRS_DYLIB_E2E=1 cargo test -p cljrs --test pinned_dylib_e2e
//! ```
//!
//! Fixtures, all built around a tiny native crate (`pinlib`) whose
//! `cljrs_init` defines `pinlib/build-tag`:
//!
//! - a git repository where commit v1 returns 1 and HEAD returns 2, for the
//!   pinned (`@<sha>`) and plain-`require` paths;
//! - a single-crate working tree, for `:local/root`;
//! - a two-crate working tree where the tag comes from a *sibling* crate the
//!   dylib crate depends on by path, for `:local/root` + `:rust/crate`.
//!
//! Every test in this binary sets `HOME` and `CLJRS_WORKSPACE_ROOT` for the
//! whole process, so they are serialized on [`ENV_LOCK`] (see [`TestEnv`]).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, MutexGuard};

use cljrs_value::Value;

/// Serializes the tests in this binary.
///
/// Each of them points `HOME` at a private cache directory and pins
/// `CLJRS_WORKSPACE_ROOT`, and `std::env::set_var` is undefined behaviour
/// while any other thread may be reading the environment: libtest runs these
/// tests on parallel threads, and a `Command` spawn reads the environment.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// The process environment for one test: holds [`ENV_LOCK`] for the test's
/// whole body, points `HOME` at a private cache directory (so the dylib and
/// git caches are hermetic) and pins the workspace the wrapper's
/// `cljrs-interop` is taken from.
struct TestEnv {
    _guard: MutexGuard<'static, ()>,
    home: tempfile::TempDir,
}

impl TestEnv {
    fn new(ws_root: &Path) -> TestEnv {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let home = tempfile::tempdir().unwrap();
        // SAFETY: `ENV_LOCK` is held for the rest of the test, and every test
        // in this binary takes it before touching the environment or spawning
        // a process, so no other thread reads or writes it concurrently.
        unsafe {
            std::env::set_var("HOME", home.path());
            std::env::set_var("CLJRS_WORKSPACE_ROOT", ws_root);
        }
        TestEnv {
            _guard: guard,
            home,
        }
    }

    /// The private `HOME` this test runs under.
    fn home(&self) -> &Path {
        self.home.path()
    }
}

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

/// Locate the clojurust workspace root (this crate is `<root>/crates/cljrs`).
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
    let _env = TestEnv::new(&ws_root);
    let (repo, sha_v1) = make_pinlib_repo(&ws_root);

    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();

    // Host's own (HEAD) implementation — must remain untouched.
    let head_fn = cljrs_value::NativeFn {
        blocking_func: None,
        captures: Vec::new(),
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
    let config = cljrs_project::config::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_project::config::Dependency::Git(cljrs_project::config::GitDep {
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

    cljrs::native::pinned::install(&globals);

    // Resolve the pinned symbol: must build + load the v1 dylib.
    let resolved = cljrs_runtime::env::versioned::resolve_versioned_value(
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
    let again = cljrs_runtime::env::versioned::resolve_versioned_value(
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
    let _env = TestEnv::new(&ws_root);
    let (repo, sha_v1) = make_pinlib_repo(&ws_root);

    let _mutator = cljrs_gc::register_mutator();
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();

    // cljrs.edn equivalent: pinlib is a git dep with :rust/load :dylib, pinned
    // at the v1 commit.  No Clojure source on the path provides this namespace.
    let config = cljrs_project::config::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_project::config::Dependency::Git(cljrs_project::config::GitDep {
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

    cljrs::native::pinned::install(&globals);

    // A plain `(require '[pinlib :as pl])` must build + load the native dep.
    let spec = cljrs_runtime::env::env::RequireSpec {
        ns: Arc::from("pinlib"),
        version: None,
        alias: Some(Arc::from("pl")),
        refer: cljrs_runtime::env::env::RequireRefer::None,
    };
    cljrs_runtime::env::loader::load_ns(globals.clone(), &spec, "user")
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

// ── :local/root native deps ──────────────────────────────────────────────────

/// Write the `pinlib` fixture crate into `root` as a plain directory — no git,
/// no commit, just a working tree as a developer would have it.
fn write_pinlib_tree(root: &Path, ws_root: &Path, tag: i64) {
    std::fs::create_dir_all(root.join("src")).unwrap();
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
    std::fs::write(root.join("src/lib.rs"), pinlib_source(tag)).unwrap();
}

/// A runtime whose `cljrs.edn` declares `pinlib` as a `:local/root` native
/// dep, with `crate_subdir` as its `:rust/crate`.
fn globals_with_local_dep(
    root: &Path,
    crate_subdir: Option<&str>,
) -> Arc<cljrs_runtime::tiered::GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();

    let config = cljrs_project::config::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_project::config::Dependency::Local {
                root: root.to_path_buf(),
                rust_init: Some(Arc::from("pinlib::cljrs_init")),
                rust_crate_dir: crate_subdir.map(Arc::from),
                rust_load_dylib: true,
            },
        )],
        ..Default::default()
    };
    *globals.deps_config.write().unwrap() = Some(Arc::new(config));
    cljrs::native::pinned::install(&globals);
    globals
}

fn require_pinlib(globals: &Arc<cljrs_runtime::tiered::GlobalEnv>) -> i64 {
    let spec = cljrs_runtime::env::env::RequireSpec {
        ns: Arc::from("pinlib"),
        version: None,
        alias: None,
        refer: cljrs_runtime::env::env::RequireRefer::None,
    };
    cljrs_runtime::env::loader::load_ns(globals.clone(), &spec, "user")
        .expect("require of a :local/root native dep should succeed");

    let f = globals
        .lookup_in_ns("pinlib", "build-tag")
        .expect("pinlib/build-tag must be registered by the dylib");
    let Value::NativeFunction(nf) = &f else {
        panic!("expected a native fn, got {f:?}");
    };
    match (nf.get().func)(&[]).unwrap() {
        Value::Long(n) => n,
        other => panic!("expected a long, got {other:?}"),
    }
}

/// A `:local/root` dep with `:rust/load :dylib` is built from the working tree
/// and loaded by a plain `require` — the case a developer hits while writing
/// the extension, before any of it is committed or pushed anywhere.
///
/// Editing the tree and requiring again must pick the edit up: a working tree
/// has no commit to key a cache on, so its artifact is never assumed fresh.
#[test]
fn local_root_native_dep_is_built_from_the_working_tree() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!(
            "skipping local_root_native_dep_is_built_from_the_working_tree \
             (set CLJRS_DYLIB_E2E=1 to run)"
        );
        return;
    }

    let ws_root = workspace_root();
    let _env = TestEnv::new(&ws_root);
    let tree = tempfile::tempdir().unwrap();
    write_pinlib_tree(tree.path(), &ws_root, 7);

    let _mutator = cljrs_gc::register_mutator();

    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), None)),
        7,
        "the dylib must be built from the working tree as it stands"
    );

    // Edit the tree the way a developer would, and require again in a fresh
    // runtime: the rebuilt dylib must carry the edit.
    std::fs::write(tree.path().join("src/lib.rs"), pinlib_source(8)).unwrap();
    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), None)),
        8,
        "an edited working tree must be rebuilt, not served from cache"
    );
}

/// Write a two-crate working tree: the dylib crate at `crates/pinlib`, and the
/// sibling `crates/pintag` it takes the build tag from by path dependency.
/// This is the layout `:rust/crate` exists for.
fn write_multi_crate_tree(root: &Path, ws_root: &Path, tag: i64) {
    std::fs::create_dir_all(root.join("crates/pinlib/src")).unwrap();
    std::fs::create_dir_all(root.join("crates/pintag/src")).unwrap();

    std::fs::write(
        root.join("crates/pintag/Cargo.toml"),
        r#"[package]
name = "pintag"
version = "0.1.0"
edition = "2024"

[workspace]
"#,
    )
    .unwrap();
    write_pintag_source(root, tag);

    let pinlib_toml = format!(
        r#"[package]
name = "pinlib"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
cljrs-interop = {{ path = "{}" }}
pintag = {{ path = "../pintag" }}
"#,
        ws_root.join("crates/cljrs-interop").display()
    );
    std::fs::write(root.join("crates/pinlib/Cargo.toml"), pinlib_toml).unwrap();
    std::fs::write(
        root.join("crates/pinlib/src/lib.rs"),
        r#"use cljrs_interop::{Registry, wrap_fn0};

pub fn cljrs_init(registry: &mut Registry) {
    registry.define(
        "pinlib/build-tag",
        wrap_fn0("build-tag", || Ok::<i64, String>(pintag::TAG)),
    );
}
"#,
    )
    .unwrap();
}

/// Rewrite only the sibling crate's source, leaving `crates/pinlib` untouched.
fn write_pintag_source(root: &Path, tag: i64) {
    std::fs::write(
        root.join("crates/pintag/src/lib.rs"),
        format!("pub const TAG: i64 = {tag};\n"),
    )
    .unwrap();
}

/// A `:local/root` dep with `:rust/crate` pointing at one crate of a
/// multi-crate tree is versioned by the whole tree, not by that crate: the
/// crate's path dependencies are part of what gets built.
///
/// Editing only the sibling leaves `crates/pinlib` byte-identical.  Digesting
/// the `:rust/crate` directory alone therefore yields the same version, the
/// cached artifact is returned before cargo is ever invoked, and the stale
/// library is `dlopen`ed.
#[test]
fn local_root_native_dep_picks_up_a_sibling_crate_edit() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!(
            "skipping local_root_native_dep_picks_up_a_sibling_crate_edit \
             (set CLJRS_DYLIB_E2E=1 to run)"
        );
        return;
    }

    let ws_root = workspace_root();
    let _env = TestEnv::new(&ws_root);
    let tree = tempfile::tempdir().unwrap();
    write_multi_crate_tree(tree.path(), &ws_root, 11);

    let _mutator = cljrs_gc::register_mutator();
    let crate_subdir = Some("crates/pinlib");

    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), crate_subdir)),
        11,
        "the dylib must be built from the crate :rust/crate names"
    );

    // The edit is entirely outside `:rust/crate`.
    write_pintag_source(tree.path(), 12);
    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), crate_subdir)),
        12,
        "an edit to a sibling crate must rebuild, not serve the cached artifact"
    );
}

/// An edit that does not advance the file's mtime (`cp -p`, `rsync --times`,
/// a timestamp-preserving archive) changes the tree's digest but not cargo's
/// view of it: cargo decides freshness by mtime, and the target directory is
/// shared across versions.  The build must still carry the edit rather than
/// relink the previous version's code and publish it under the new version.
///
/// Also checks that the superseded working-tree artifact is evicted, so a tree
/// edited many times leaves one library behind rather than one per edit.
#[test]
fn local_root_native_dep_picks_up_an_edit_that_keeps_its_mtime() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!(
            "skipping local_root_native_dep_picks_up_an_edit_that_keeps_its_mtime \
             (set CLJRS_DYLIB_E2E=1 to run)"
        );
        return;
    }

    let ws_root = workspace_root();
    let env = TestEnv::new(&ws_root);
    let tree = tempfile::tempdir().unwrap();
    write_multi_crate_tree(tree.path(), &ws_root, 21);

    let tag_file = tree.path().join("crates/pintag/src/lib.rs");
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(946_684_800);
    let set_old_mtime = || {
        std::fs::File::options()
            .write(true)
            .open(&tag_file)
            .unwrap()
            .set_modified(old)
            .unwrap();
    };
    set_old_mtime();

    let _mutator = cljrs_gc::register_mutator();
    let crate_subdir = Some("crates/pinlib");

    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), crate_subdir)),
        21
    );

    write_pintag_source(tree.path(), 22);
    set_old_mtime();
    assert_eq!(
        require_pinlib(&globals_with_local_dep(tree.path(), crate_subdir)),
        22,
        "an edit that keeps its mtime must still be compiled, not relinked stale"
    );

    let artifacts: Vec<_> = std::fs::read_dir(env.home().join(".cljrs/cache/dylibs"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("pinlib@local-"))
        .collect();
    assert_eq!(
        artifacts.len(),
        1,
        "the superseded artifact must be evicted, found {artifacts:?}"
    );
}

/// A `:local/root` dep that opted into `:dylib` but named no `:rust/init` is
/// still *declined* by versioned resolution, not turned into an error: whether
/// a dep can answer `ns/f@<sha>` follows from its source having no commit, and
/// that is settled before `:rust/init` is required.
///
/// Not gated on `CLJRS_DYLIB_E2E`: the dep declines before anything is
/// materialized or built, which is the whole point, so nothing here runs cargo.
#[test]
fn a_local_dep_without_rust_init_declines_versioned_resolution() {
    let ws_root = workspace_root();
    let _env = TestEnv::new(&ws_root);
    let _mutator = cljrs_gc::register_mutator();

    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .build()
        .expect("runtime")
        .into_globals();

    // The host's own implementation, which the fallback must reach.
    let head_fn = cljrs_value::NativeFn {
        blocking_func: None,
        captures: Vec::new(),
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

    // `:rust/load :dylib` with no `:rust/init`, on a root that does not exist:
    // reaching either the validation error or the filesystem would be a bug.
    let config = cljrs_project::config::DepsConfig {
        deps: vec![(
            Arc::from("pinlib"),
            cljrs_project::config::Dependency::Local {
                root: PathBuf::from("/nonexistent/local/root"),
                rust_init: None,
                rust_crate_dir: None,
                rust_load_dylib: true,
            },
        )],
        ..Default::default()
    };
    *globals.deps_config.write().unwrap() = Some(Arc::new(config));
    cljrs::native::pinned::install(&globals);

    let sha = "0123456789abcdef0123456789abcdef01234567";
    let resolved = cljrs_runtime::env::versioned::resolve_versioned_value(
        &globals,
        "user",
        Some("pinlib"),
        "build-tag",
        sha,
    )
    .expect("a misconfigured local dep must decline versioned resolution, not error it");
    let Value::NativeFunction(nf) = &resolved else {
        panic!("expected a native fn, got {resolved:?}");
    };
    assert_eq!((nf.get().func)(&[]).unwrap(), Value::Long(99));
}

/// A `:local/root` dep has no commit, so it cannot answer a versioned symbol
/// (`pinlib/build-tag@<sha>`). The resolver must fall back to the host's own
/// native binding rather than erroring or silently serving the local build.
#[test]
fn local_root_native_dep_does_not_serve_versioned_resolution() {
    if std::env::var("CLJRS_DYLIB_E2E").is_err() {
        eprintln!(
            "skipping local_root_native_dep_does_not_serve_versioned_resolution \
             (set CLJRS_DYLIB_E2E=1 to run)"
        );
        return;
    }

    let ws_root = workspace_root();
    let _env = TestEnv::new(&ws_root);
    let tree = tempfile::tempdir().unwrap();
    write_pinlib_tree(tree.path(), &ws_root, 7);

    let _mutator = cljrs_gc::register_mutator();
    let globals = globals_with_local_dep(tree.path(), None);

    // The host's own implementation, which the fallback must reach.
    let head_fn = cljrs_value::NativeFn {
        blocking_func: None,
        captures: Vec::new(),
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

    let sha = "0123456789abcdef0123456789abcdef01234567";
    let resolved = cljrs_runtime::env::versioned::resolve_versioned_value(
        &globals,
        "user",
        Some("pinlib"),
        "build-tag",
        sha,
    )
    .expect("versioned resolution should fall back to the host's native fn");
    let Value::NativeFunction(nf) = &resolved else {
        panic!("expected a native fn, got {resolved:?}");
    };
    assert_eq!(
        (nf.get().func)(&[]).unwrap(),
        Value::Long(99),
        "a local dep must not answer for a commit it does not have"
    );
    assert!(
        !globals.is_loaded(&format!("pinlib@{sha}")),
        "no versioned namespace should have been created for a local dep"
    );
}
