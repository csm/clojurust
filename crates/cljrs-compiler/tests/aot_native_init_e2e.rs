//! AOT-compiling a program whose `cljrs.edn` declares a `:rust :init` hook.
//!
//! This is the path `cljrs compile` takes for any project with a native
//! extension, and until this file existed nothing exercised it end to end.
//! Three separate defects reached the tree through that gap in a single
//! session: the emitted init call was not wrapped in `unsafe` (so an extension
//! declaring the honest `unsafe extern "C" fn` could not build its own
//! harness), two extensions exported the same `cljrs_init` symbol, and the
//! harness emitted the crate IDENTIFIER where Cargo wanted the PACKAGE name.
//! Every one was found by reading, not by CI.
//!
//! What makes this a real test rather than a compile check: it asserts the
//! produced binary RUNS and that the function the init hook registered is
//! CALLABLE from the compiled program. A registration that silently does
//! nothing still compiles — that was the failure mode of the init-symbol bug,
//! where two crates folded onto one definition and the loser's namespace
//! quietly went missing.
//!
//! The fixture package is deliberately HYPHENATED (`aot-init-fixture`, crate
//! identifier `aot_init_fixture`). Hyphens are the Rust community norm and
//! what both in-tree extensions use, and a Cargo dependency key is read as the
//! package name unless `package = "..."` redirects it. The older `pinlib`
//! fixture is a single unhyphenated word, which is precisely why it never
//! caught this.
//!
//! Cost: each test builds a real binary through `cargo build`. The
//! `unsafe extern "C" fn` case — the spelling both in-tree extensions use, and
//! the one that reproduces all three defects — runs in ordinary CI on purpose;
//! gating the whole file would re-open the coverage gap in a new form. The
//! safe `extern "C" fn` spelling is the cheaper variation and is gated behind
//! `aot_full_test` with the rest of the AOT suite:
//!
//! ```sh
//! cargo test -p cljrs-compiler --features aot_full_test --test aot_native_init_e2e
//! ```

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use cljrs_project::config::RustConfig;

mod common;

/// Serialize these tests: each runs `cargo build` in a harness project, and
/// concurrent cargo processes fight over the same lock.
static AOT_LOCK: Mutex<()> = Mutex::new(());

/// Locate the clojurust workspace root (this crate is `<root>/crates/cljrs-compiler`).
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// How the fixture crate spells its entry point.
///
/// `SafeExternC` exists only under `aot_full_test`: the ungated build never
/// constructs it, and an ungated variant would read as dead code.
#[derive(Clone, Copy)]
enum Spelling {
    /// What both in-tree extensions declare: the honest signature for a
    /// function that dereferences a raw pointer.
    UnsafeExternC,
    /// What `docs/book/src/rust-interop/project-setup.md` teaches.
    #[cfg(feature = "aot_full_test")]
    SafeExternC,
}

impl Spelling {
    fn init_body(self) -> &'static str {
        match self {
            Spelling::UnsafeExternC => {
                "/// # Safety\n\
                 /// `registry` must be a valid, uniquely-borrowed `*mut Registry`.\n\
                 #[unsafe(no_mangle)]\n\
                 pub unsafe extern \"C\" fn cljrs_init_fixture(registry: *mut Registry) {\n    \
                     register(unsafe { &mut *registry });\n\
                 }\n"
            }
            #[cfg(feature = "aot_full_test")]
            Spelling::SafeExternC => {
                "#[unsafe(no_mangle)]\n\
                 pub extern \"C\" fn cljrs_init_fixture(registry: *mut Registry) {\n    \
                     register(unsafe { &mut *registry });\n\
                 }\n"
            }
        }
    }
}

/// Write the fixture extension crate and return its directory.
///
/// The crate registers `fixture/answer`, which the compiled program calls. The
/// empty `[workspace]` table keeps Cargo from treating it as a member of a
/// parent workspace, and `cljrs-interop` is pinned by absolute path into this
/// checkout so the fixture and the harness unify on one crate instance.
fn write_fixture_crate(root: &Path, spelling: Spelling) {
    std::fs::create_dir_all(root.join("src")).unwrap();

    let cargo_toml = format!(
        r#"[package]
name = "aot-init-fixture"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[lib]
crate-type = ["rlib"]

[dependencies]
cljrs-interop = {{ path = "{}" }}
"#,
        workspace_root().join("crates/cljrs-interop").display()
    );
    std::fs::write(root.join("Cargo.toml"), cargo_toml).unwrap();

    let lib_rs = format!(
        r#"use cljrs_interop::{{Registry, wrap_fn0}};

pub fn register(registry: &mut Registry) {{
    registry.define(
        "fixture/answer",
        wrap_fn0("answer", || Ok::<i64, String>(42)),
    );
}}

{}"#,
        spelling.init_body()
    );
    std::fs::write(root.join("src/lib.rs"), lib_rs).unwrap();
}

/// Compile `source` with the fixture crate wired in as `:rust :init`, run the
/// binary, and return its stdout.
#[allow(clippy::result_large_err)]
fn compile_with_native_init(name: &str, spelling: Spelling, source: &str) -> String {
    let _guard = AOT_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = tempfile::tempdir().expect("tempdir");
    let crate_dir = dir.path().join("fixture");
    write_fixture_crate(&crate_dir, spelling);

    let src_path = dir.path().join(format!("{name}.cljrs"));
    let bin_path = dir.path().join(format!("{name}_bin"));
    std::fs::write(&src_path, source).unwrap();

    let rust_config = RustConfig {
        crate_dir: crate_dir.clone(),
        init_fn: Some("aot_init_fixture::cljrs_init_fixture".into()),
    };
    let session = common::session(vec![]).rust_config(Some(rust_config));

    // compile_file needs a large stack for Clojure eval.
    let result = std::thread::Builder::new()
        .stack_size(8 * 1024 * 1024)
        .spawn({
            let src = src_path.clone();
            let bin = bin_path.clone();
            move || cljrs_compiler::aot::compile_file(&src, &bin, &session)
        })
        .unwrap()
        .join()
        .unwrap();

    result.unwrap_or_else(|e| panic!("compilation failed for {name}: {e:?}"));

    let output = Command::new(&bin_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run {name} binary: {e}"));
    assert!(
        output.status.success(),
        "{name} binary exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// The whole point: a hyphen-named extension crate AOT-compiles, and the
/// function its init hook registered answers in the compiled program.
///
/// Before the package-name fix this failed in cargo, not in an assertion:
///
/// ```text
/// error: no matching package named `aot_init_fixture` found
/// help: packages with similar names: aot-init-fixture
/// ```
#[test]
fn a_hyphen_named_extension_registers_a_callable_fn() {
    let out = compile_with_native_init(
        "native_init_unsafe",
        Spelling::UnsafeExternC,
        "(println (fixture/answer))\n",
    );
    assert_eq!(
        out.trim(),
        "42",
        "the init hook's namespace was not callable from the compiled program"
    );
}

/// The safe spelling the docs teach compiles and runs identically — the
/// `#[allow(unused_unsafe)]` in the emitted harness is what keeps it warning
/// free.
#[cfg(feature = "aot_full_test")]
#[test]
fn a_safe_extern_c_entry_point_works_too() {
    let out = compile_with_native_init(
        "native_init_safe",
        Spelling::SafeExternC,
        "(println (fixture/answer))\n",
    );
    assert_eq!(out.trim(), "42");
}
