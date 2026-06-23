//! Pinned native packages: build a dependency's Rust crate at a pinned git
//! commit as a cdylib and load it (`:rust/load :dylib` in `cljrs.edn`).
//!
//! By default, a pinned symbol (`mylib/f@<sha>`) that resolves to a native
//! (Rust-backed) function falls back to the **current binary's**
//! implementation, with provenance verification (see
//! `cljrs_env::versioned`).  This crate provides the opt-in alternative:
//! true pinned native code.
//!
//! ## Flow
//!
//! 1. `install(globals)` registers a [`PinnedNativeLoader`] hook on the
//!    environment.  The versioned resolver calls it whenever a pinned
//!    lookup is about to fall back to a native function.
//! 2. The hook checks `cljrs.edn` for a dep covering the namespace with
//!    `:rust/load :dylib` and a `:rust/init` function.
//! 3. The dep's repository is fetched (`cljrs_vcs::fetch_remote`), checked
//!    out at the pinned commit, and wrapped in a generated cdylib crate
//!    that pins the exact same `cljrs-interop` as the host.
//! 4. `cargo build --release` (cached per `(crate, commit, rustc, cljrs
//!    version)`), then `dlopen`.
//! 5. **ABI handshake**: the wrapper exports `cljrs_dylib_abi()` returning
//!    a fingerprint string (cljrs version + `rustc -V`, baked at the
//!    wrapper's build time).  The host refuses to proceed unless it equals
//!    the host's own fingerprint exactly.
//! 6. The wrapper's `cljrs_dylib_init(*mut Registry)` registers the
//!    package's exports through a [`Registry::versioned`] view, so the
//!    pinned implementations land in the immutable `"<ns>@<commit>"`
//!    namespace and never collide with the live ones.
//!
//! ## Safety model (experimental)
//!
//! `cljrs_dylib_init` crosses the boundary with a Rust-ABI `&mut Registry`.
//! This is sound *only* because the handshake guarantees both sides were
//! compiled by the identical compiler against the identical `cljrs-interop`
//! version.  Feature-flag skew between host and wrapper builds is not
//! detected; the whole mechanism is opt-in and documented as experimental.
//! A full C-ABI vtable is the safer long-term design and is deliberately
//! deferred.

// EvalError is large by design across the workspace (same allow as cljrs-env).
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_deps::{Dependency, GitDep};
use cljrs_env::env::GlobalEnv;
use cljrs_env::error::{EvalError, EvalResult};

/// Exported ABI-handshake symbol name.
pub const ABI_SYMBOL: &[u8] = b"cljrs_dylib_abi\0";
/// Exported init symbol name.
pub const INIT_SYMBOL: &[u8] = b"cljrs_dylib_init\0";

/// The host's ABI fingerprint: cljrs workspace version, the rustc that
/// compiled this crate, and the build profile (debug/release — `cljrs-gc`'s
/// object headers have `debug_assertions`-gated fields, so the profiles must
/// match).  A wrapper dylib is only loaded when its baked fingerprint equals
/// this string exactly.
pub fn abi_fingerprint() -> String {
    format!(
        "cljrs {}; {}; {}",
        env!("CARGO_PKG_VERSION"),
        env!("CLJRS_DYLIB_RUSTC"),
        host_profile(),
    )
}

/// The host's build profile, which the wrapper build must match.
fn host_profile() -> &'static str {
    if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    }
}

/// Install the pinned-native package loader on `globals`.
///
/// Idempotent (first writer wins).  Called by the `cljrs` CLI during
/// environment setup; embedders that want `:rust/load :dylib` support call
/// it after constructing their `GlobalEnv`.
pub fn install(globals: &Arc<GlobalEnv>) {
    globals.set_pinned_native_loader(Arc::new(load_pinned));
    globals.set_native_require_loader(Arc::new(load_require));
}

/// The loader hook: returns `Ok(true)` when the package covering `base_ns`
/// was built at `commit` and registered into `"<base_ns>@<commit>"`.
fn load_pinned(globals: &Arc<GlobalEnv>, base_ns: &str, commit: &str) -> EvalResult<bool> {
    let config = globals.deps_config.read().unwrap().clone();
    let Some(config) = config else {
        return Ok(false);
    };
    let Some(git) = find_dylib_dep(&config, base_ns) else {
        return Ok(false);
    };

    let versioned_ns = format!("{base_ns}@{commit}");
    if globals.is_loaded(&versioned_ns) {
        return Ok(true);
    }

    let lib_path = build_pinned_wrapper(&git, commit)
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    load_library(globals, &lib_path, Some(commit))
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    globals.mark_loaded(&versioned_ns);
    eprintln!("[cljrs-dylib] loaded pinned native package {versioned_ns}");
    Ok(true)
}

/// The `require`-path loader hook: returns `Ok(true)` when a `:rust/load
/// :dylib` dep covering `ns` was built at its pinned `:git/sha` and its
/// exports were registered into the **unversioned** namespace, making a plain
/// `(require '[ns :as …])` of a pure-native package succeed.
///
/// Unlike [`load_pinned`] (which serves versioned-symbol resolution and lands
/// the package in the immutable `"<ns>@<commit>"` namespace), this registers
/// into the live namespace so unversioned references resolve normally.  The
/// caller (`cljrs-env`'s unversioned loader) marks `ns` loaded on success.
fn load_require(globals: &Arc<GlobalEnv>, ns: &str) -> EvalResult<bool> {
    let config = globals.deps_config.read().unwrap().clone();
    let Some(config) = config else {
        return Ok(false);
    };
    let Some(git) = find_dylib_dep(&config, ns) else {
        return Ok(false);
    };

    // Already brought in (e.g. an earlier require of a sibling namespace
    // provided by the same dylib loaded the whole package).
    if globals.is_loaded(ns) {
        return Ok(true);
    }

    let commit = git.sha.clone();
    let lib_path = build_pinned_wrapper(&git, &commit)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    load_library(globals, &lib_path, None)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    eprintln!("[cljrs-dylib] loaded native dep {ns} (pinned {commit})");
    Ok(true)
}

/// Find a `:rust/load :dylib` git dep whose name covers `base_ns` (exact
/// match or dotted prefix: dep `my.lib` covers `my.lib.util`).
fn find_dylib_dep(config: &cljrs_deps::DepsConfig, base_ns: &str) -> Option<GitDep> {
    for (name, dep) in &config.deps {
        if let Dependency::Git(git) = dep
            && git.rust_load_dylib
            && (name.as_ref() == base_ns
                || base_ns
                    .strip_prefix(name.as_ref())
                    .is_some_and(|rest| rest.starts_with('.')))
        {
            return Some(git.clone());
        }
    }
    None
}

// ── Wrapper build ─────────────────────────────────────────────────────────────

/// Build (or reuse from cache) the wrapper cdylib for `git` at `commit`,
/// returning the path to the built library.
fn build_pinned_wrapper(git: &GitDep, commit: &str) -> Result<PathBuf, String> {
    let init_fn = git
        .rust_init
        .as_deref()
        .ok_or("dep has :rust/load :dylib but no :rust/init function")?;
    let crate_name = init_fn.split("::").next().unwrap_or(init_fn);
    // Cargo package names use '-', Rust paths use '_': accept either by
    // depending on the package directory and renaming is unnecessary —
    // we depend by path, and Cargo reads the real package name from the
    // checkout's Cargo.toml.  The `extern crate` ident is the init path's
    // first segment, which must match the package's lib name.
    let pkg_ident = crate_name.replace('-', "_");

    // 1. Fetch + checkout at the pinned commit.
    let bare = cljrs_vcs::fetch_remote(&git.url, commit).map_err(|e| e.to_string())?;
    let checkout = checkout_at(&bare, crate_name, commit)?;
    let crate_dir = match git.rust_crate_dir.as_deref() {
        Some(sub) => checkout.join(sub),
        None => checkout.clone(),
    };
    if !crate_dir.join("Cargo.toml").exists() {
        return Err(format!(
            "no Cargo.toml at {} (set :rust/crate if the crate lives in a subdirectory)",
            crate_dir.display()
        ));
    }

    // 2. Cache key: same wrapper inputs → same artifact.
    let fingerprint = abi_fingerprint();
    let fp_hash = stable_hash(&format!("{fingerprint}|{}|{commit}", git.url));
    let wrapper_dir = dylib_cache_root()
        .join(format!("{pkg_ident}@{commit}"))
        .join(format!("fp-{fp_hash}"));
    let artifact = wrapper_artifact_path(&wrapper_dir);
    if artifact.exists() {
        return Ok(artifact);
    }

    // 3. Generate the wrapper crate.
    write_wrapper_crate(&wrapper_dir, &crate_dir, crate_name, &pkg_ident, init_fn)?;

    // 4. Build, matching the host's profile (see `abi_fingerprint`).
    let offline = find_workspace_root().is_some();
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg("build").current_dir(&wrapper_dir);
    if host_profile() == "release" {
        cmd.arg("--release");
    }
    if offline {
        cmd.arg("--offline");
    }
    eprintln!("[cljrs-dylib] building pinned native package {pkg_ident}@{commit}…");
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build of pinned wrapper failed (see output above; wrapper at {})",
            wrapper_dir.display()
        ));
    }

    if !artifact.exists() {
        return Err(format!("built wrapper not found at {}", artifact.display()));
    }
    Ok(artifact)
}

/// Materialize the pinned commit's tree into a working checkout (no `.git`).
fn checkout_at(bare: &Path, crate_name: &str, commit: &str) -> Result<PathBuf, String> {
    let dest = dylib_cache_root()
        .join("checkouts")
        .join(format!("{crate_name}@{commit}"));
    // Sentinel marking a previously completed checkout (the worktree has no
    // `.git`, so we can't probe for one).
    let sentinel = dest.join(".cljrs-checkout-complete");
    if sentinel.exists() {
        return Ok(dest);
    }
    std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;

    // Resolve the pinned commit to its tree and check that tree out into `dest`
    // with gitoxide — just the files needed to build the crate.
    let repo = gix::open(bare).map_err(|e| format!("open {}: {e}", bare.display()))?;
    let tree = repo
        .rev_parse_single(commit)
        .map_err(|e| format!("resolve {commit}: {e}"))?
        .object()
        .map_err(|e| e.to_string())?
        .peel_to_tree()
        .map_err(|e| format!("peel {commit} to tree: {e}"))?;
    let mut index = repo
        .index_from_tree(&tree.id)
        .map_err(|e| format!("index from tree: {e}"))?;
    let opts = repo
        .checkout_options(gix::worktree::stack::state::attributes::Source::IdMapping)
        .map_err(|e| e.to_string())?;
    let odb = repo.objects.clone().into_arc().map_err(|e| e.to_string())?;
    let should_interrupt = std::sync::atomic::AtomicBool::new(false);
    gix::worktree::state::checkout(
        &mut index,
        &dest,
        odb,
        &gix::progress::Discard,
        &gix::progress::Discard,
        &should_interrupt,
        opts,
    )
    .map_err(|e| format!("checkout {commit} into {}: {e}", dest.display()))?;

    std::fs::write(&sentinel, commit.as_bytes()).map_err(|e| e.to_string())?;
    Ok(dest)
}

/// Write the generated wrapper crate (Cargo.toml, build.rs, src/lib.rs).
fn write_wrapper_crate(
    wrapper_dir: &Path,
    crate_dir: &Path,
    crate_name: &str,
    pkg_ident: &str,
    init_fn: &str,
) -> Result<(), String> {
    std::fs::create_dir_all(wrapper_dir.join("src")).map_err(|e| e.to_string())?;

    // Pin cljrs-interop exactly like the AOT harness pins runtime crates:
    // a local checkout when one is found (offline), the published version
    // otherwise.  The handshake catches any residual mismatch.
    let interop_dep = match find_workspace_root() {
        Some(root) => format!(
            "cljrs-interop = {{ path = \"{}\" }}",
            root.join("crates/cljrs-interop").display()
        ),
        None => format!("cljrs-interop = \"={}\"", env!("CARGO_PKG_VERSION")),
    };

    let cargo_toml = format!(
        r#"[package]
name = "cljrs-pinned-wrapper"
version = "{version}"
edition = "2024"

[workspace]

[lib]
crate-type = ["cdylib"]

[dependencies]
{interop_dep}
{crate_name} = {{ path = "{crate_dir}" }}

[profile.release]
panic = "unwind"
"#,
        version = env!("CARGO_PKG_VERSION"),
        crate_dir = crate_dir.display(),
    );
    std::fs::write(wrapper_dir.join("Cargo.toml"), cargo_toml).map_err(|e| e.to_string())?;

    let build_rs = r#"fn main() {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=CLJRS_WRAPPER_RUSTC={version}");
}
"#;
    std::fs::write(wrapper_dir.join("build.rs"), build_rs).map_err(|e| e.to_string())?;

    let lib_rs = format!(
        r#"//! Auto-generated pinned-package wrapper (cljrs-dylib).

/// ABI fingerprint baked at build time; must equal the host's
/// `cljrs_dylib::abi_fingerprint()` exactly (including the build profile —
/// cljrs-gc object headers differ between debug and release).
#[cfg(debug_assertions)]
static ABI: &str = concat!(
    "cljrs ",
    env!("CARGO_PKG_VERSION"),
    "; ",
    env!("CLJRS_WRAPPER_RUSTC"),
    "; debug\0"
);
#[cfg(not(debug_assertions))]
static ABI: &str = concat!(
    "cljrs ",
    env!("CARGO_PKG_VERSION"),
    "; ",
    env!("CLJRS_WRAPPER_RUSTC"),
    "; release\0"
);

#[unsafe(no_mangle)]
pub extern "C" fn cljrs_dylib_abi() -> *const std::os::raw::c_char {{
    ABI.as_ptr() as *const std::os::raw::c_char
}}

/// Register the pinned package's exports into the host-provided registry.
///
/// # Safety
/// `registry` must be a valid `*mut cljrs_interop::Registry` from a host
/// whose ABI fingerprint matched `cljrs_dylib_abi()`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn cljrs_dylib_init(registry: *mut cljrs_interop::Registry) {{
    let registry = unsafe {{ &mut *registry }};
    // This dylib's own #[export] inventory (separate from the host's).
    cljrs_interop::register_exports(registry);
    {pkg_ident}::{init_tail}(registry);
}}
"#,
        init_tail = init_fn
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or("cljrs_init"),
    );
    std::fs::write(wrapper_dir.join("src/lib.rs"), lib_rs).map_err(|e| e.to_string())?;
    Ok(())
}

/// The built artifact path for a wrapper crate dir (host-profile build).
fn wrapper_artifact_path(wrapper_dir: &Path) -> PathBuf {
    let stem = "cljrs_pinned_wrapper";
    #[cfg(target_os = "macos")]
    let file = format!("lib{stem}.dylib");
    #[cfg(target_os = "windows")]
    let file = format!("{stem}.dll");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let file = format!("lib{stem}.so");
    wrapper_dir.join("target").join(host_profile()).join(file)
}

// ── Loading ───────────────────────────────────────────────────────────────────

/// dlopen the wrapper, verify the ABI handshake, and run its init.
///
/// `version` selects the `Registry` view: `Some(commit)` registers the
/// package's exports into the immutable `"<ns>@<commit>"` namespace (pinned
/// versioned resolution); `None` registers into the live, unversioned
/// namespaces (the plain-`require` path).
fn load_library(
    globals: &Arc<GlobalEnv>,
    path: &Path,
    version: Option<&str>,
) -> Result<(), String> {
    // SAFETY: the library is generated by `write_wrapper_crate` and built by
    // us; the Rust-ABI init call is guarded by the fingerprint handshake.
    unsafe {
        let lib = libloading::Library::new(path).map_err(|e| e.to_string())?;

        let abi: libloading::Symbol<unsafe extern "C" fn() -> *const std::os::raw::c_char> =
            lib.get(ABI_SYMBOL).map_err(|e| e.to_string())?;
        let got = std::ffi::CStr::from_ptr(abi())
            .to_string_lossy()
            .to_string();
        let expected = abi_fingerprint();
        if got != expected {
            return Err(format!(
                "ABI fingerprint mismatch: wrapper was built as `{got}` but this binary \
                 expects `{expected}`; rebuild with the matching toolchain/cljrs version"
            ));
        }

        let init: libloading::Symbol<unsafe extern "C" fn(*mut cljrs_interop::Registry)> =
            lib.get(INIT_SYMBOL).map_err(|e| e.to_string())?;
        let mut registry = match version {
            Some(commit) => cljrs_interop::Registry::versioned(globals.clone(), commit),
            None => cljrs_interop::Registry::for_require(globals.clone()),
        };
        init(&mut registry as *mut _);

        // The dylib's code must stay mapped as long as any registered
        // NativeFn closure exists (same contract as the CLI's
        // load_native_lib).
        std::mem::forget(lib);
    }
    Ok(())
}

// ── Paths & misc ──────────────────────────────────────────────────────────────

/// `~/.cljrs/cache/dylibs`.
fn dylib_cache_root() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".cljrs").join("cache").join("dylibs")
}

/// Locate a local clojurust checkout for path-pinned wrapper deps:
/// `CLJRS_WORKSPACE_ROOT` override first, then this crate's compile-time
/// manifest location (`<workspace>/crates/cljrs-dylib`).
fn find_workspace_root() -> Option<PathBuf> {
    let validate = |p: PathBuf| -> Option<PathBuf> {
        (p.join("Cargo.toml").exists() && p.join("crates/cljrs-interop/Cargo.toml").exists())
            .then_some(p)
    };
    if let Some(root) = std::env::var_os("CLJRS_WORKSPACE_ROOT") {
        return validate(PathBuf::from(root));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    validate(manifest_dir.parent()?.parent()?.to_path_buf())
}

/// Short stable hex hash for cache directory names.
fn stable_hash(s: &str) -> String {
    use std::hash::{DefaultHasher, Hash as _, Hasher as _};
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}
