//! Native packages: build a dependency's Rust crate as a cdylib and load it
//! (`:rust/load :dylib` in `cljrs.edn`).
//!
//! Two dependency forms reach this module, and they differ only in where the
//! crate's source comes from:
//!
//! - `:git/url` + `:git/sha` — an immutable commit. Serves both `require` and
//!   versioned (`mylib/f@<sha>`) resolution; its artifact is cached forever.
//! - `:local/root` — a working tree, built as it currently stands. Serves
//!   `require` only (there is no commit to resolve `@<sha>` against), and is
//!   versioned by a digest of the tree, so an edit is a different build.
//!
//! By default, a pinned symbol (`mylib/f@<sha>`) that resolves to a native
//! (Rust-backed) function falls back to the **current binary's**
//! implementation, with provenance verification (see
//! `cljrs_runtime::env::versioned`).  This module provides the opt-in
//! alternative: true out-of-binary native code.
//!
//! ## Flow
//!
//! 1. `install(globals)` registers the loader hooks on the environment.  The
//!    versioned resolver calls one whenever a pinned lookup is about to fall
//!    back to a native function; `require` calls the other for a namespace no
//!    source path provides.
//! 2. The hook checks `cljrs.edn` for a dep covering the namespace with
//!    `:rust/load :dylib` and a `:rust/init` function, then passes the trust
//!    gate ([`super::check`]) for the project that `cljrs.edn` belongs to.
//!    The config opting in is not enough: an untrusted project builds and
//!    loads nothing.
//! 3. The dep's source is materialized — fetched and checked out at the
//!    pinned commit (`cljrs_project::vcs`), or taken as-is from
//!    `:local/root` — and wrapped in a generated cdylib crate that pins the
//!    exact same `cljrs-interop` as the host.
//! 4. `cargo build` in the host's profile (cached per `(crate, source, rustc,
//!    cljrs version)`), then `dlopen`.
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

// EvalError is large by design across the workspace (same allow as cljrs-runtime).
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_project::config::Dependency;
use cljrs_runtime::env::error::{EvalError, EvalResult};
use cljrs_runtime::tiered::GlobalEnv;

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
/// Idempotent (first writer wins).  Called by [`crate::session::setup_globals`]
/// during environment setup.
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
    let Some(decl) = find_dylib_dep(&config, base_ns) else {
        return Ok(false);
    };
    // A working tree has no commit, so it cannot answer for `ns/f@sha`.
    // Declined *before* validation: a misconfigured local dep must leave
    // versioned resolution exactly as it was, not turn it into an error.
    if !decl.is_pinned() {
        return Ok(false);
    }
    // The trust gate, before anything is fetched, built or loaded.  Declining
    // leaves versioned resolution on its default path: the current binary's
    // implementation, with provenance verification.
    if let Err(refusal) = super::check(config.root.as_deref()) {
        super::report_refusal(&refusal);
        return Ok(false);
    }
    let dep = decl
        .validate()
        .map_err(|e| EvalError::Runtime(format!("pinned native {base_ns}: {e}")))?;

    let versioned_ns = format!("{base_ns}@{commit}");
    if globals.is_loaded(&versioned_ns) {
        return Ok(true);
    }

    let lib_path = build_wrapper(&dep, Some(commit))
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    load_library(globals, &lib_path, Some(commit))
        .map_err(|e| EvalError::Runtime(format!("pinned native {versioned_ns}: {e}")))?;

    globals.mark_loaded(&versioned_ns);
    eprintln!("[cljrs] loaded pinned native package {versioned_ns}");
    Ok(true)
}

/// The `require`-path loader hook: returns `Ok(true)` when a `:rust/load
/// :dylib` dep covering `ns` was built and its exports were registered into
/// the **unversioned** namespace, making a plain `(require '[ns :as …])` of a
/// pure-native package succeed.
///
/// Unlike [`load_pinned`] (which serves versioned-symbol resolution and lands
/// the package in the immutable `"<ns>@<commit>"` namespace), this registers
/// into the live namespace so unversioned references resolve normally.  The
/// caller (`cljrs-runtime`'s unversioned loader) marks `ns` loaded on success.
///
/// Both dependency forms reach this path: a git dep builds at its pinned
/// `:git/sha`, a `:local/root` dep builds the working tree as it stands.
fn load_require(globals: &Arc<GlobalEnv>, ns: &str) -> EvalResult<bool> {
    let config = globals.deps_config.read().unwrap().clone();
    let Some(config) = config else {
        return Ok(false);
    };
    let Some(decl) = find_dylib_dep(&config, ns) else {
        return Ok(false);
    };
    // The trust gate, before anything is fetched, built or loaded.  This is an
    // error rather than a decline: the namespace exists only as native code,
    // so declining would surface as a misleading "namespace not found".
    super::check(config.root.as_deref())
        .map_err(|refusal| EvalError::Runtime(format!("native dep {ns}: {refusal}")))?;
    let dep = decl
        .validate()
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    // Already brought in (e.g. an earlier require of a sibling namespace
    // provided by the same dylib loaded the whole package).
    if globals.is_loaded(ns) {
        return Ok(true);
    }

    let lib_path = build_wrapper(&dep, None)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    load_library(globals, &lib_path, None)
        .map_err(|e| EvalError::Runtime(format!("native dep {ns}: {e}")))?;

    eprintln!("[cljrs] loaded native dep {ns} ({})", dep.provenance());
    Ok(true)
}

// ── Domain: what a native dependency IS ───────────────────────────────────────

/// Where a `:rust/load :dylib` dep's crate source comes from.
///
/// A closed set: every source a `cljrs.edn` dep can name is one of these.
#[derive(Debug, Clone)]
enum NativeSource {
    /// A git repository, checked out at a pinned commit.
    Pinned { url: Arc<str>, sha: Arc<str> },
    /// A directory on disk, built as it currently stands.
    WorkingTree(PathBuf),
}

impl NativeSource {
    /// The commit this source is fixed at, or `None` for a working tree.
    fn commit(&self) -> Option<&str> {
        match self {
            NativeSource::Pinned { sha, .. } => Some(sha.as_ref()),
            NativeSource::WorkingTree(_) => None,
        }
    }
}

/// What exactly was built — the identity that names one artifact.
///
/// A commit names it for a git dep; a working tree has no commit, so its
/// content does. Either way an artifact under this version is immutable, which
/// is what lets the cache be trusted and what keeps a rebuilt library from
/// landing on the path an already-`dlopen`ed one occupies.
struct SourceVersion {
    /// The cache-directory suffix, e.g. `@<sha>` or `@local-<hash>`.
    slug: String,
    /// The full key mixed into the ABI-fingerprint hash.
    key: String,
}

/// A `:rust/load :dylib` dependency as `cljrs.edn` declares it, before
/// validation.
///
/// The source alone settles which resolution paths the dep can serve, so that
/// question is answerable here; only actually building it needs `:rust/init`.
#[derive(Debug, Clone)]
struct DylibDecl {
    source: NativeSource,
    init_fn: Option<Arc<str>>,
    crate_subdir: Option<Arc<str>>,
}

impl DylibDecl {
    /// Whether this dep names an immutable commit, and so can serve versioned
    /// (`ns/f@sha`) resolution.
    fn is_pinned(&self) -> bool {
        self.source.commit().is_some()
    }

    /// Into a loadable dep, or `Err` when the dep opted into `:dylib` without
    /// naming the `:rust/init` function that would load it.
    fn validate(self) -> Result<NativeDep, String> {
        let init_fn = self
            .init_fn
            .ok_or("dep has :rust/load :dylib but no :rust/init function")?;
        Ok(NativeDep {
            source: self.source,
            init_fn,
            crate_subdir: self.crate_subdir,
        })
    }
}

/// A dependency that opts into loading its Rust code as a cdylib.
///
/// Validation happens in [`DylibDecl::validate`]: a `NativeDep` always names
/// an init function, so no later stage has to handle its absence.
#[derive(Debug, Clone)]
struct NativeDep {
    source: NativeSource,
    init_fn: Arc<str>,
    crate_subdir: Option<Arc<str>>,
}

impl NativeDep {
    /// The `extern crate` identifier the `:rust/init` path names: its first
    /// segment.  A Rust identifier, *not* the Cargo package name, which comes
    /// from [`package_name_of`]; the two differ whenever the package name
    /// contains `-`, which an identifier cannot.
    fn pkg_ident(&self) -> &str {
        self.init_fn.split("::").next().unwrap_or(&self.init_fn)
    }

    /// The init path with its crate segment stripped (`a::b::c` -> `b::c`).
    fn init_tail(&self) -> &str {
        self.init_fn
            .split_once("::")
            .map(|(_, rest)| rest)
            .unwrap_or(&self.init_fn)
    }

    /// Human-readable provenance, for the load log line.
    fn provenance(&self) -> String {
        match &self.source {
            NativeSource::Pinned { sha, .. } => format!("pinned {sha}"),
            NativeSource::WorkingTree(root) => format!("local {}", root.display()),
        }
    }
}

/// Read the `:rust/*` keys of a `:deps` entry as a dylib declaration, or
/// `None` when the entry did not opt into `:rust/load :dylib`.
fn as_dylib_decl(dep: &Dependency) -> Option<DylibDecl> {
    match dep {
        Dependency::Git(git) if git.rust_load_dylib => Some(DylibDecl {
            source: NativeSource::Pinned {
                url: git.url.clone(),
                sha: git.sha.clone(),
            },
            init_fn: git.rust_init.clone(),
            crate_subdir: git.rust_crate_dir.clone(),
        }),
        Dependency::Local {
            root,
            rust_init,
            rust_crate_dir,
            rust_load_dylib: true,
        } => Some(DylibDecl {
            source: NativeSource::WorkingTree(root.clone()),
            init_fn: rust_init.clone(),
            crate_subdir: rust_crate_dir.clone(),
        }),
        _ => None,
    }
}

/// Whether a dep named `dep_name` provides namespace `ns` — exact match, or a
/// dotted prefix (dep `my.lib` covers `my.lib.util`).
fn covers_namespace(dep_name: &str, ns: &str) -> bool {
    dep_name == ns
        || ns
            .strip_prefix(dep_name)
            .is_some_and(|rest| rest.starts_with('.'))
}

/// Find the `:rust/load :dylib` declaration covering `base_ns`.
fn find_dylib_dep(config: &cljrs_project::config::DepsConfig, base_ns: &str) -> Option<DylibDecl> {
    config
        .deps
        .iter()
        .filter(|(name, _)| covers_namespace(name, base_ns))
        .find_map(|(_, dep)| as_dylib_decl(dep))
}

// ── Wrapper build ─────────────────────────────────────────────────────────────

/// Where the wrapper for one `(dep, version)` is generated and what it produces.
struct WrapperPlan {
    /// The root of the dep's materialized source: the checkout for a pinned
    /// dep, the `:local/root` for a working tree.
    source_root: PathBuf,
    /// The dep's own crate, already on disk.
    crate_dir: PathBuf,
    /// The generated wrapper crate's directory.  Deliberately *not* keyed by
    /// version: cargo tracks the dep's own sources itself, so one wrapper
    /// crate and one target directory per `(dep crate, ABI)` makes an edit
    /// cost an incremental rebuild instead of a fresh one, and leaves one
    /// target directory on disk instead of one per edit.
    wrapper_dir: PathBuf,
    /// The cargo target directory for `wrapper_dir`, set explicitly so the
    /// artifact path below is predictable whatever the ambient
    /// `CARGO_TARGET_DIR` says.
    target_dir: PathBuf,
    /// The cdylib cargo produces inside [`Self::target_dir`].
    build_output: PathBuf,
    /// The version-unique path the cdylib is published to and `dlopen`ed
    /// from.  A rebuilt library reusing the path an already-loaded one
    /// occupies would not be picked up, so this cannot be the shared
    /// [`Self::build_output`].
    artifact: PathBuf,
    /// [`SourceVersion::key`] of the version being built.
    version_key: String,
    /// The [`BuildRecord`] naming the version [`Self::target_dir`] last built.
    record: PathBuf,
    /// Cache identity, for log lines.
    label: String,
}

/// Which version the shared build directory last built, and where it was
/// published.
///
/// Cargo decides what to recompile from mtimes, so after an edit that did not
/// advance one (`cp -p`, `rsync --times`, a timestamp-preserving archive) it
/// would call a changed crate fresh and relink the previous version's code.
/// The record is what lets [`build_wrapper`] notice that the target directory
/// holds another version's build and force the source crates to be rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BuildRecord {
    /// [`SourceVersion::key`] of the build.
    key: String,
    /// Where that build was published.
    artifact: PathBuf,
}

impl BuildRecord {
    /// The record at `path`; `None` when it is missing or unreadable.
    fn read(path: &Path) -> Option<BuildRecord> {
        let text = std::fs::read_to_string(path).ok()?;
        let json: serde_json::Value = serde_json::from_str(&text).ok()?;
        Some(BuildRecord {
            key: json.get("key")?.as_str()?.to_string(),
            artifact: PathBuf::from(json.get("artifact")?.as_str()?),
        })
    }

    fn write(&self, path: &Path) -> Result<(), String> {
        let json = serde_json::json!({
            "key": self.key,
            "artifact": self.artifact.to_string_lossy(),
        });
        std::fs::write(path, json.to_string())
            .map_err(|e| format!("writing {}: {e}", path.display()))
    }
}

/// Whether the target directory may hold compiled source crates from a version
/// other than `key`, so cargo's mtime-based freshness cannot be trusted.
///
/// A target directory with no record has unknown provenance: an interrupted
/// build removes the record before starting (see [`build_wrapper`]), and so
/// does any build that predates it.
fn target_is_foreign(previous: Option<&BuildRecord>, key: &str, target_exists: bool) -> bool {
    match previous {
        Some(record) => record.key != key,
        None => target_exists,
    }
}

/// Build (or reuse from cache) the wrapper cdylib for `dep`, returning the
/// path to the built library.
///
/// `commit_override` builds a pinned dep at a commit other than its declared
/// `:git/sha` — the versioned resolver asks for the commit named in the symbol.
/// It must be `None` for a working-tree dep, which has no commit at all.
fn build_wrapper(dep: &NativeDep, commit_override: Option<&str>) -> Result<PathBuf, String> {
    let source_root = materialize_source(&dep.source, commit_override)?;
    let crate_dir = crate_dir_of(dep, &source_root);
    if !crate_dir.join("Cargo.toml").exists() {
        return Err(format!(
            "no Cargo.toml at {} (set :rust/crate if the crate lives in a subdirectory)",
            crate_dir.display()
        ));
    }

    let version = version_of(dep, &source_root, commit_override)?;
    let plan = plan_wrapper(dep, source_root, crate_dir, &version);
    if plan.artifact.exists() {
        return Ok(plan.artifact);
    }

    write_wrapper_crate(&plan.wrapper_dir, &plan.crate_dir, dep)?;

    let previous = BuildRecord::read(&plan.record);
    if target_is_foreign(
        previous.as_ref(),
        &plan.version_key,
        plan.target_dir.exists(),
    ) {
        clean_source_crates(&plan)?;
    }
    // Until this build is published the target directory is in an unknown
    // state: a failure part-way through may leave some crates compiled from
    // this version, which a later build of the recorded one would take as
    // fresh.
    if plan.record.exists() {
        std::fs::remove_file(&plan.record)
            .map_err(|e| format!("removing {}: {e}", plan.record.display()))?;
    }
    cargo_build(&plan)?;

    if !plan.build_output.exists() {
        return Err(format!(
            "built wrapper not found at {}",
            plan.build_output.display()
        ));
    }
    publish_artifact(&plan)?;
    BuildRecord {
        key: plan.version_key.clone(),
        artifact: plan.artifact.clone(),
    }
    .write(&plan.record)?;

    if let (NativeSource::WorkingTree(_), Some(previous)) = (&dep.source, previous) {
        evict_artifact(&previous.artifact, &plan.artifact, &dylib_cache_root());
    }
    Ok(plan.artifact)
}

/// Remove a superseded working-tree artifact, so a tree edited over a long
/// time does not leave one library per save behind.
///
/// Only the working-tree form calls this: a pinned commit's artifact stays
/// valid forever.  `previous` comes from a file on disk, so nothing outside
/// `cache_root` is touched, whatever it says.  Failure is ignored — a library
/// still mapped by a running process cannot be removed on every platform, and
/// a leftover directory costs only space.
fn evict_artifact(previous: &Path, current: &Path, cache_root: &Path) {
    if previous == current {
        return;
    }
    let Some(fp_dir) = previous.parent() else {
        return;
    };
    let Some(label_dir) = fp_dir.parent() else {
        return;
    };
    if label_dir.parent() != Some(cache_root) || current.starts_with(fp_dir) {
        return;
    }
    let _ = std::fs::remove_dir_all(fp_dir);
    // Holds other ABI fingerprints' builds of the same version, if any.
    let _ = std::fs::remove_dir(label_dir);
}

/// Copy the freshly built cdylib to its version-unique path.
///
/// `dlopen` keys on the path, so a rebuilt library has to arrive somewhere the
/// previous one never occupied; the shared build directory cannot provide
/// that, and the copy is what buys it back.
///
/// The copy lands on a temporary sibling and is renamed into place.
/// [`build_wrapper`] trusts any file at the artifact path, so a copy cut short
/// (interrupt, full disk) must never be visible there; a rename within one
/// directory is atomic, which also makes two processes publishing the same
/// version at once safe.
fn publish_artifact(plan: &WrapperPlan) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    let dir = plan
        .artifact
        .parent()
        .ok_or("wrapper artifact path has no parent directory")?;
    let file_name = plan
        .artifact
        .file_name()
        .ok_or("wrapper artifact path has no file name")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let temp = dir.join(format!(
        ".{}.{}-{}.tmp",
        file_name.to_string_lossy(),
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::copy(&plan.build_output, &temp) {
        let _ = std::fs::remove_file(&temp);
        return Err(format!(
            "copying {} to {}: {e}",
            plan.build_output.display(),
            temp.display()
        ));
    }
    if let Err(e) = std::fs::rename(&temp, &plan.artifact) {
        let _ = std::fs::remove_file(&temp);
        // Where a rename cannot replace an existing file, a concurrent
        // publisher of the same version having got there first is success.
        if plan.artifact.exists() {
            return Ok(());
        }
        return Err(format!(
            "renaming {} to {}: {e}",
            temp.display(),
            plan.artifact.display()
        ));
    }
    Ok(())
}

/// Identify what is about to be built, given the dep's materialized
/// `source_root`.
///
/// A pinned dep is named by its commit. A working tree is named by a digest of
/// its source files, so an edited tree is a different version, which is what
/// gets it rebuilt and gets the result its own path to be loaded from.
///
/// The digest covers the whole `source_root`, not just the crate `:rust/crate`
/// names: a crate in a multi-crate tree normally has path dependencies on its
/// siblings, so an edit outside it still changes what gets built.  The cost is
/// a spurious rebuild when something the build ignores changes; the
/// alternative is loading a stale library, silently.
fn version_of(
    dep: &NativeDep,
    source_root: &Path,
    commit_override: Option<&str>,
) -> Result<SourceVersion, String> {
    match &dep.source {
        NativeSource::Pinned { url, sha } => {
            let commit = commit_override.unwrap_or(sha.as_ref());
            Ok(SourceVersion {
                slug: format!("@{commit}"),
                key: format!("{url}|{commit}"),
            })
        }
        NativeSource::WorkingTree(_) => {
            let digest = digest_source_tree(source_root)?;
            Ok(SourceVersion {
                slug: format!("@local-{digest}"),
                key: format!("{}|{digest}", source_root.display()),
            })
        }
    }
}

/// Directories [`digest_source_tree`] never descends into: build output, VCS
/// data and other toolchains' dependency trees, none of which cargo reads as
/// source.
const NON_SOURCE_DIRS: &[&str] = &["target", ".git", ".hg", ".svn", ".jj", "node_modules"];

/// Files larger than this are identified by length and mtime rather than read
/// in full.  Rust sources are far smaller; what gets this large is fixtures and
/// data, which would otherwise be read on every `require`.
const CONTENT_DIGEST_LIMIT: u64 = 1 << 20;

/// Digest every source file under `dir`, ignoring build output and VCS data.
///
/// Path and contents both feed the hash, so a rename is as much a change as an
/// edit. Walk order is sorted, making the digest independent of readdir order.
///
/// Symlinks are not followed into directories: a cycle would never end, and a
/// link out of the tree would digest whatever it points at.  A symlinked
/// directory contributes its link target; a symlinked file, its contents.
fn digest_source_tree(dir: &Path) -> Result<String, String> {
    let mut acc = String::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let mut entries: Vec<_> = std::fs::read_dir(&current)
            .map_err(|e| format!("reading {}: {e}", current.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("reading {}: {e}", current.display()))?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            // `DirEntry::file_type` describes the entry itself, never a
            // symlink's target.
            let file_type = entry
                .file_type()
                .map_err(|e| format!("reading {}: {e}", path.display()))?;
            if file_type.is_dir() {
                let name = entry.file_name();
                if !NON_SOURCE_DIRS.contains(&name.to_string_lossy().as_ref()) {
                    stack.push(path);
                }
                continue;
            }
            let fingerprint = if file_type.is_symlink() {
                symlink_fingerprint(&path)?
            } else {
                let meta = entry
                    .metadata()
                    .map_err(|e| format!("reading {}: {e}", path.display()))?;
                file_fingerprint(&path, &meta)?
            };
            let rel = path.strip_prefix(dir).unwrap_or(&path);
            acc.push_str(&format!("{}|{fingerprint}\n", rel.display()));
        }
    }
    Ok(stable_hash(&acc))
}

/// A regular file's contribution to [`digest_source_tree`].
fn file_fingerprint(path: &Path, meta: &std::fs::Metadata) -> Result<String, String> {
    if meta.len() > CONTENT_DIGEST_LIMIT {
        return Ok(format!(
            "len={} mtime={:?}",
            meta.len(),
            meta.modified().ok()
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(stable_hash_bytes(&bytes))
}

/// A symlink's contribution to [`digest_source_tree`]: the file it resolves to,
/// or — for a link to a directory, or a dangling one — its target path.
fn symlink_fingerprint(path: &Path) -> Result<String, String> {
    if let Ok(meta) = std::fs::metadata(path)
        && meta.is_file()
    {
        return file_fingerprint(path, &meta);
    }
    let target =
        std::fs::read_link(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(format!("link={}", target.display()))
}

/// The dep's crate directory within its materialized source.
fn crate_dir_of(dep: &NativeDep, source_root: &Path) -> PathBuf {
    match dep.crate_subdir.as_deref() {
        Some(sub) => source_root.join(sub),
        None => source_root.to_path_buf(),
    }
}

/// Put the dep's source on disk and return its root.
///
/// For a pinned dep this fetches (network only when the commit is missing) and
/// materializes a files-only checkout; a working tree is already on disk.
fn materialize_source(
    source: &NativeSource,
    commit_override: Option<&str>,
) -> Result<PathBuf, String> {
    match source {
        NativeSource::Pinned { url, sha } => {
            let commit = commit_override.unwrap_or(sha.as_ref());
            cljrs_project::vcs::fetch_remote(url, commit).map_err(|e| e.to_string())?;
            cljrs_project::vcs::worktree_at_commit(url, commit).map_err(|e| e.to_string())
        }
        NativeSource::WorkingTree(root) => {
            if commit_override.is_some() {
                return Err("a :local/root dep has no commit to build at".into());
            }
            if !root.is_dir() {
                return Err(format!("local dep root not found at {}", root.display()));
            }
            Ok(root.canonicalize().unwrap_or_else(|_| root.clone()))
        }
    }
}

/// Derive every path the build needs. Pure: the source is already resolved.
fn plan_wrapper(
    dep: &NativeDep,
    source_root: PathBuf,
    crate_dir: PathBuf,
    version: &SourceVersion,
) -> WrapperPlan {
    let abi = abi_fingerprint();
    let label = format!("{}{}", dep.pkg_ident(), version.slug);
    let fp_hash = stable_hash(&format!("{abi}|{}", version.key));
    let build_hash = stable_hash(&format!("{abi}|{}", crate_dir.display()));
    let wrapper_dir = dylib_cache_root()
        .join("build")
        .join(format!("{}-{build_hash}", dep.pkg_ident()));
    let target_dir = wrapper_dir.join("target");
    let lib_file = wrapper_lib_filename();
    let build_output = target_dir.join(host_profile()).join(&lib_file);
    let artifact = dylib_cache_root()
        .join(&label)
        .join(format!("fp-{fp_hash}"))
        .join(&lib_file);
    WrapperPlan {
        source_root,
        crate_dir,
        record: wrapper_dir.join("last-build.json"),
        wrapper_dir,
        target_dir,
        build_output,
        artifact,
        version_key: version.key.clone(),
        label,
    }
}

/// Whether a wrapper build may reach the network to resolve dependencies.
///
/// Cargo resolves the *dependency's own* crates here, not just the
/// `cljrs-interop` pin, so online is the default: an extension that pulls an
/// uncached crates.io crate has to be buildable.  `CLJRS_DYLIB_OFFLINE`
/// selects `--offline` for a machine that already has everything vendored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NetworkPolicy {
    Online,
    Offline,
}

impl NetworkPolicy {
    /// Read the policy from a `CLJRS_DYLIB_OFFLINE` value.  Unset, empty and
    /// `0` mean online; anything else opts into `--offline`.
    fn from_env_value(value: Option<&str>) -> NetworkPolicy {
        match value {
            None => NetworkPolicy::Online,
            Some(v) if v.is_empty() || v == "0" => NetworkPolicy::Online,
            Some(_) => NetworkPolicy::Offline,
        }
    }

    /// The policy this process runs under.
    fn from_env() -> NetworkPolicy {
        NetworkPolicy::from_env_value(std::env::var("CLJRS_DYLIB_OFFLINE").ok().as_deref())
    }
}

/// A `cargo <subcommand>` run on the generated wrapper, in its target
/// directory and under the configured [`NetworkPolicy`].
fn wrapper_cargo(plan: &WrapperPlan, subcommand: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new("cargo");
    cmd.arg(subcommand)
        .current_dir(&plan.wrapper_dir)
        .env("CARGO_TARGET_DIR", &plan.target_dir);
    if NetworkPolicy::from_env() == NetworkPolicy::Offline {
        cmd.arg("--offline");
    }
    cmd
}

/// Run `cargo build` on the generated wrapper, matching the host's profile
/// (see [`abi_fingerprint`]).
fn cargo_build(plan: &WrapperPlan) -> Result<(), String> {
    let mut cmd = wrapper_cargo(plan, "build");
    if host_profile() == "release" {
        cmd.arg("--release");
    }
    eprintln!("[cljrs] building native package {}…", plan.label);
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo build of native wrapper failed (see output above; wrapper at {})",
            plan.wrapper_dir.display()
        ));
    }
    Ok(())
}

/// Discard the compiled output of every crate under [`WrapperPlan::source_root`]
/// so the next build compiles them from what is on disk now.
///
/// Called when the target directory holds another version's build (see
/// [`BuildRecord`]).  Only the dep's own tree is cleaned: its registry and git
/// dependencies, and `cljrs-interop`, are keyed by version and stay
/// incremental.
fn clean_source_crates(plan: &WrapperPlan) -> Result<(), String> {
    let out = wrapper_cargo(plan, "metadata")
        .args(["--format-version", "1"])
        .stderr(std::process::Stdio::inherit())
        .output()
        .map_err(|e| format!("cargo: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata of native wrapper failed (see output above; wrapper at {})",
            plan.wrapper_dir.display()
        ));
    }
    let metadata: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("parsing cargo metadata output: {e}"))?;
    let names = source_crate_names(&metadata, &plan.source_root);
    if names.is_empty() {
        return Ok(());
    }

    let mut cmd = wrapper_cargo(plan, "clean");
    if host_profile() == "release" {
        cmd.arg("--release");
    }
    for name in &names {
        cmd.arg("-p").arg(name);
    }
    let status = cmd.status().map_err(|e| format!("cargo: {e}"))?;
    if !status.success() {
        return Err(format!(
            "cargo clean of native wrapper failed (see output above; wrapper at {})",
            plan.wrapper_dir.display()
        ));
    }
    Ok(())
}

/// Names of the path packages in `cargo metadata` output whose manifest lives
/// under `source_root`, sorted and deduplicated.
///
/// Names rather than package ids: `cargo clean -p` ignores an id's URL and
/// version qualifiers and cleans by name regardless.
fn source_crate_names(metadata: &serde_json::Value, source_root: &Path) -> Vec<String> {
    let mut names: Vec<String> = metadata
        .get("packages")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter(|pkg| pkg.get("source").is_none_or(|s| s.is_null()))
        .filter(|pkg| {
            pkg.get("manifest_path")
                .and_then(|m| m.as_str())
                .is_some_and(|m| Path::new(m).starts_with(source_root))
        })
        .filter_map(|pkg| pkg.get("name")?.as_str().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Write the generated wrapper crate (Cargo.toml, build.rs, src/lib.rs).
///
/// The dependency is declared under the Rust identifier the `:rust/init` path
/// uses, renamed to the package's real name — those differ whenever a package
/// name contains `-`, which Rust identifiers cannot.
fn write_wrapper_crate(
    wrapper_dir: &Path,
    crate_dir: &Path,
    dep: &NativeDep,
) -> Result<(), String> {
    let pkg_ident = dep.pkg_ident();
    let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml"))
        .map_err(|e| format!("reading {}: {e}", crate_dir.join("Cargo.toml").display()))?;
    let package_name = package_name_of(&manifest).ok_or_else(|| {
        format!(
            "no [package] name in {}",
            crate_dir.join("Cargo.toml").display()
        )
    })?;
    let dep_line = format!(
        "{pkg_ident} = {{ path = {}, package = {} }}",
        toml_basic_string(&crate_dir.display().to_string()),
        toml_basic_string(&package_name),
    );
    std::fs::create_dir_all(wrapper_dir.join("src")).map_err(|e| e.to_string())?;

    // Pin cljrs-interop exactly like the AOT harness pins runtime crates:
    // a local checkout when one is found (offline), the published version
    // otherwise.  The handshake catches any residual mismatch.
    let interop_dep = match find_workspace_root() {
        Some(root) => format!(
            "cljrs-interop = {{ path = {} }}",
            toml_basic_string(&root.join("crates/cljrs-interop").display().to_string())
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
{dep_line}

[profile.release]
panic = "unwind"
"#,
        version = env!("CARGO_PKG_VERSION"),
    );
    write_if_changed(&wrapper_dir.join("Cargo.toml"), &cargo_toml)?;

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
    write_if_changed(&wrapper_dir.join("build.rs"), build_rs)?;

    let lib_rs = format!(
        r#"//! Auto-generated pinned-package wrapper (cljrs).

/// ABI fingerprint baked at build time; must equal the host's
/// `cljrs::native::pinned::abi_fingerprint()` exactly (including the build profile —
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
        init_tail = dep.init_tail(),
    );
    write_if_changed(&wrapper_dir.join("src/lib.rs"), &lib_rs)?;
    Ok(())
}

/// Write `contents` to `path` unless it already holds exactly that.
///
/// The wrapper crate is regenerated on every artifact miss; rewriting identical
/// bytes would still advance the mtimes and make cargo recompile the wrapper
/// and rerun its `build.rs` when only the dependency changed.
fn write_if_changed(path: &Path, contents: &str) -> Result<(), String> {
    if std::fs::read(path).is_ok_and(|old| old == contents.as_bytes()) {
        return Ok(());
    }
    std::fs::write(path, contents).map_err(|e| format!("writing {}: {e}", path.display()))
}

/// `s` rendered as a TOML basic string, quotes included.
///
/// The generated manifest carries filesystem paths: a Windows separator is an
/// invalid escape inside a basic string, and a `"` would close it early.
fn toml_basic_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str(r"\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The `[package] name` declared in a Cargo manifest.
///
/// Only that one key is read, so the full TOML grammar is not needed: track
/// which table each line belongs to and take `name` from `[package]`.
fn package_name_of(manifest: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or(line).trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "name" {
            continue;
        }
        let value = value.trim().trim_matches(['"', '\''].as_slice());
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

/// The cdylib file name cargo produces for the generated wrapper crate.
fn wrapper_lib_filename() -> String {
    let stem = "cljrs_pinned_wrapper";
    #[cfg(target_os = "macos")]
    let file = format!("lib{stem}.dylib");
    #[cfg(target_os = "windows")]
    let file = format!("{stem}.dll");
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let file = format!("lib{stem}.so");
    file
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
        // NativeFn closure exists (same contract as `super::load_project_lib`).
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
/// manifest location (`<workspace>/crates/cljrs`).
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
    stable_hash_bytes(s.as_bytes())
}

/// [`stable_hash`] over raw bytes.
///
/// File contents are hashed as they lie: a lossy UTF-8 conversion maps every
/// invalid byte to one replacement character, which would digest two files
/// that differ only in invalid UTF-8 identically.
fn stable_hash_bytes(bytes: &[u8]) -> String {
    use std::hash::{DefaultHasher, Hasher as _};
    let mut h = DefaultHasher::new();
    h.write(bytes);
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_package_name() {
        let manifest = r#"
[package]
name = "cljrs-raster"
version = "0.1.0"
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("cljrs-raster"));
    }

    #[test]
    fn ignores_a_name_in_another_table() {
        // `[lib] name` and a dependency's `package` key are both `name`-ish
        // and both wrong; only `[package]` decides.
        let manifest = r#"
[lib]
name = "wrong_lib_name"

[package]
name = "right-one"

[dependencies.serde]
name = "also-wrong"
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("right-one"));
    }

    #[test]
    fn tolerates_comments_and_spacing() {
        let manifest = r#"
# leading comment
[package]   # the package table
   name   =   'spaced-out'   # trailing comment
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("spaced-out"));
    }

    #[test]
    fn a_manifest_without_a_package_name_yields_none() {
        assert_eq!(package_name_of("[workspace]\nmembers = []\n"), None);
        assert_eq!(package_name_of(""), None);
    }

    #[test]
    fn a_workspace_inherited_version_does_not_confuse_it() {
        let manifest = r#"
[package]
name = "cljrs-ffmpeg"
version.workspace = true
edition.workspace = true
"#;
        assert_eq!(package_name_of(manifest).as_deref(), Some("cljrs-ffmpeg"));
    }

    /// A working-tree dep is versioned by a digest of its whole root, not of
    /// the crate `:rust/crate` selects: the crate being built normally has
    /// path dependencies on its siblings, so an edit to one of those changes
    /// what cargo would produce.  Digesting the crate directory alone leaves
    /// the version unchanged, and `build_wrapper` then returns the cached
    /// artifact without ever asking cargo to rebuild.
    #[test]
    fn a_working_tree_version_covers_the_whole_root() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("crates/thing/src")).unwrap();
        std::fs::create_dir_all(root.join("crates/sibling/src")).unwrap();
        std::fs::write(
            root.join("crates/thing/Cargo.toml"),
            "[package]\nname = \"thing\"\n",
        )
        .unwrap();
        std::fs::write(root.join("crates/thing/src/lib.rs"), "pub fn f() {}\n").unwrap();
        std::fs::write(
            root.join("crates/sibling/src/lib.rs"),
            "pub const N: i64 = 1;\n",
        )
        .unwrap();

        let dep = NativeDep {
            source: NativeSource::WorkingTree(root.to_path_buf()),
            init_fn: Arc::from("thing::cljrs_init"),
            crate_subdir: Some(Arc::from("crates/thing")),
        };

        let before = version_of(&dep, root, None).unwrap();
        // The edit is outside `:rust/crate`.
        std::fs::write(
            root.join("crates/sibling/src/lib.rs"),
            "pub const N: i64 = 2;\n",
        )
        .unwrap();
        let after = version_of(&dep, root, None).unwrap();

        assert_ne!(
            before.slug, after.slug,
            "an edit to a sibling crate must produce a different version"
        );
        assert_ne!(before.key, after.key);
    }

    /// A pinned dep is named by its commit, never by a digest: its checkout is
    /// immutable, and `commit_override` is what the versioned resolver asks
    /// for.
    #[test]
    fn a_pinned_version_is_the_commit() {
        let dep = NativeDep {
            source: NativeSource::Pinned {
                url: Arc::from("https://example.invalid/lib"),
                sha: Arc::from("aaaa"),
            },
            init_fn: Arc::from("lib::cljrs_init"),
            crate_subdir: None,
        };
        assert_eq!(
            version_of(&dep, Path::new("/nonexistent"), None)
                .unwrap()
                .slug,
            "@aaaa"
        );
        assert_eq!(
            version_of(&dep, Path::new("/nonexistent"), Some("bbbb"))
                .unwrap()
                .slug,
            "@bbbb"
        );
    }

    /// Two files differing only in invalid UTF-8 must not digest identically:
    /// `String::from_utf8_lossy` collapses every invalid byte onto the same
    /// replacement character, so the bytes have to be hashed as they lie.
    #[test]
    fn a_digest_distinguishes_invalid_utf8() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        std::fs::write(a.path().join("x.bin"), [0xffu8]).unwrap();
        std::fs::write(b.path().join("x.bin"), [0xfeu8]).unwrap();
        assert_ne!(
            digest_source_tree(a.path()).unwrap(),
            digest_source_tree(b.path()).unwrap()
        );
    }

    /// The `:rust/init` path's first segment is a Rust identifier, which is
    /// what the generated wrapper's `extern crate` reference needs; the Cargo
    /// package name it renames is `package_name_of`'s job.
    #[test]
    fn the_package_identifier_is_the_init_paths_first_segment() {
        let dep = NativeDep {
            source: NativeSource::WorkingTree(PathBuf::from("/x")),
            init_fn: Arc::from("my_lib::deep::cljrs_init"),
            crate_subdir: None,
        };
        assert_eq!(dep.pkg_ident(), "my_lib");
        assert_eq!(dep.init_tail(), "deep::cljrs_init");
    }

    /// A `:local/root` dep declines versioned resolution before validation, so
    /// a missing `:rust/init` cannot turn a fallback into an error.
    #[test]
    fn a_working_tree_declaration_is_not_pinned() {
        let local = DylibDecl {
            source: NativeSource::WorkingTree(PathBuf::from("/x")),
            init_fn: None,
            crate_subdir: None,
        };
        assert!(!local.is_pinned());
        assert!(local.validate().is_err());

        let pinned = DylibDecl {
            source: NativeSource::Pinned {
                url: Arc::from("u"),
                sha: Arc::from("s"),
            },
            init_fn: None,
            crate_subdir: None,
        };
        assert!(pinned.is_pinned());
    }

    /// Paths reach the generated manifest as TOML basic strings: a Windows
    /// separator is an invalid escape there and a quote would close the string.
    #[test]
    fn a_basic_string_escapes_backslashes_and_quotes() {
        assert_eq!(
            toml_basic_string(r"C:\src\my crate"),
            r#""C:\\src\\my crate""#
        );
        assert_eq!(toml_basic_string(r#"a"b"#), r#""a\"b""#);
        assert_eq!(toml_basic_string("plain/path"), r#""plain/path""#);
    }

    /// `--offline` is a policy the operator sets, not a fact derived from
    /// whether a cljrs checkout happens to be around: the flag constrains the
    /// dependency's own resolution, which that checkout says nothing about.
    #[test]
    fn the_network_policy_comes_from_its_own_setting() {
        assert_eq!(NetworkPolicy::from_env_value(None), NetworkPolicy::Online);
        assert_eq!(
            NetworkPolicy::from_env_value(Some("")),
            NetworkPolicy::Online
        );
        assert_eq!(
            NetworkPolicy::from_env_value(Some("0")),
            NetworkPolicy::Online
        );
        assert_eq!(
            NetworkPolicy::from_env_value(Some("1")),
            NetworkPolicy::Offline
        );
    }

    /// The built cdylib is published to a version-unique path: `dlopen` keys
    /// on the path, so a rebuild landing where a loaded library already sits
    /// would not be picked up.  The build directory it comes from is shared
    /// across versions, which is what keeps rebuilds incremental.
    #[test]
    fn the_artifact_path_is_version_unique_but_the_build_dir_is_not() {
        let dep = NativeDep {
            source: NativeSource::WorkingTree(PathBuf::from("/x")),
            init_fn: Arc::from("thing::cljrs_init"),
            crate_subdir: None,
        };
        let crate_dir = PathBuf::from("/x");
        let v1 = SourceVersion {
            slug: "@local-1111".into(),
            key: "/x|1111".into(),
        };
        let v2 = SourceVersion {
            slug: "@local-2222".into(),
            key: "/x|2222".into(),
        };
        let a = plan_wrapper(&dep, crate_dir.clone(), crate_dir.clone(), &v1);
        let b = plan_wrapper(&dep, crate_dir.clone(), crate_dir, &v2);

        assert_ne!(a.artifact, b.artifact);
        assert_eq!(a.wrapper_dir, b.wrapper_dir);
        assert_eq!(a.target_dir, b.target_dir);
        assert_eq!(a.build_output, b.build_output);
        assert_eq!(a.record, b.record);
        assert!(!a.artifact.starts_with(&a.target_dir));
    }

    /// Cargo decides freshness by mtime, so a content change that does not
    /// advance one is invisible to it.  The shared target directory is
    /// therefore distrusted whenever it last built some other version, or its
    /// provenance is unknown.
    #[test]
    fn a_target_dir_built_from_another_version_is_foreign() {
        let record = BuildRecord {
            key: "/x|1111".into(),
            artifact: PathBuf::from("/a"),
        };
        assert!(!target_is_foreign(Some(&record), "/x|1111", true));
        assert!(target_is_foreign(Some(&record), "/x|2222", true));
        assert!(target_is_foreign(None, "/x|2222", true));
        assert!(!target_is_foreign(None, "/x|2222", false));
    }

    #[test]
    fn a_build_record_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("last-build.json");
        assert_eq!(BuildRecord::read(&path), None);
        let record = BuildRecord {
            key: "/x|1111".into(),
            artifact: PathBuf::from("/cache/pkg@local-1111/fp-ab/lib.so"),
        };
        record.write(&path).unwrap();
        assert_eq!(BuildRecord::read(&path), Some(record));
        std::fs::write(&path, "not json").unwrap();
        assert_eq!(BuildRecord::read(&path), None);
    }

    /// Only path packages inside the dep's own tree are cleaned: registry
    /// crates and `cljrs-interop` (a path dep outside it) stay incremental.
    #[test]
    fn only_crates_under_the_source_root_are_cleaned() {
        let metadata = serde_json::json!({
            "packages": [
                {"name": "thing", "source": null,
                 "manifest_path": "/tree/crates/thing/Cargo.toml"},
                {"name": "sibling", "source": null,
                 "manifest_path": "/tree/crates/sibling/Cargo.toml"},
                {"name": "cljrs-interop", "source": null,
                 "manifest_path": "/cljrs/crates/cljrs-interop/Cargo.toml"},
                {"name": "cljrs-pinned-wrapper", "source": null,
                 "manifest_path": "/home/.cljrs/cache/dylibs/build/thing-1/Cargo.toml"},
                {"name": "serde",
                 "source": "registry+https://github.com/rust-lang/crates.io-index",
                 "manifest_path": "/tree/vendor/serde/Cargo.toml"},
            ]
        });
        assert_eq!(
            source_crate_names(&metadata, Path::new("/tree")),
            vec!["sibling".to_string(), "thing".to_string()]
        );
    }

    /// Publishing goes through a temporary sibling and a rename, so the
    /// artifact path only ever holds a complete library, and no temporary is
    /// left behind.
    #[test]
    fn publishing_replaces_the_artifact_whole() {
        let tmp = tempfile::tempdir().unwrap();
        let build_output = tmp.path().join("out.so");
        std::fs::write(&build_output, b"new library").unwrap();
        let artifact = tmp.path().join("pkg@local-1/fp-2/lib.so");
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, b"trunc").unwrap();

        let plan = WrapperPlan {
            source_root: tmp.path().into(),
            crate_dir: tmp.path().into(),
            wrapper_dir: tmp.path().into(),
            target_dir: tmp.path().join("target"),
            build_output,
            artifact: artifact.clone(),
            version_key: "k".into(),
            record: tmp.path().join("last-build.json"),
            label: "pkg@local-1".into(),
        };
        publish_artifact(&plan).unwrap();

        assert_eq!(std::fs::read(&artifact).unwrap(), b"new library");
        let names: Vec<_> = std::fs::read_dir(artifact.parent().unwrap())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("lib.so")]);
    }

    #[test]
    fn a_superseded_artifact_is_evicted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let old = root.join("pkg@local-1/fp-a/lib.so");
        let new = root.join("pkg@local-2/fp-a/lib.so");
        for p in [&old, &new] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"lib").unwrap();
        }
        evict_artifact(&old, &new, root);
        assert!(!root.join("pkg@local-1").exists());
        assert!(new.exists());

        // The current artifact is never removed.
        evict_artifact(&new, &new, root);
        assert!(new.exists());
    }

    /// The record is a file on disk; a path it names outside the cache root is
    /// left alone.
    #[test]
    fn eviction_stays_inside_the_cache_root() {
        let cache = tempfile::tempdir().unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let outside = elsewhere.path().join("a/b/lib.so");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"keep").unwrap();
        evict_artifact(&outside, &cache.path().join("p@1/fp/lib.so"), cache.path());
        assert!(outside.exists());
    }

    /// Unchanged wrapper sources keep their mtimes, so cargo does not rebuild
    /// the wrapper or rerun its `build.rs` when only the dependency changed.
    #[test]
    fn rewriting_identical_contents_leaves_the_file_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("build.rs");
        write_if_changed(&path, "fn main() {}\n").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(old)
            .unwrap();

        write_if_changed(&path, "fn main() {}\n").unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), old);

        write_if_changed(&path, "fn main() { }\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn main() { }\n");
    }

    /// A symlink cycle under the root must terminate: directory links are
    /// digested by their target, never descended into.
    #[cfg(unix)]
    #[test]
    fn a_symlink_cycle_does_not_hang_the_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("a")).unwrap();
        std::fs::write(root.join("a/lib.rs"), "x").unwrap();
        std::os::unix::fs::symlink(root, root.join("a/loop")).unwrap();
        digest_source_tree(root).unwrap();
    }

    /// A symlinked file is digested by what it resolves to, so editing the
    /// target changes the version.
    #[cfg(unix)]
    #[test]
    fn a_symlinked_file_is_digested_by_contents() {
        let tree = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("shared.rs");
        std::fs::write(&target, "1").unwrap();
        std::os::unix::fs::symlink(&target, tree.path().join("shared.rs")).unwrap();
        let before = digest_source_tree(tree.path()).unwrap();
        std::fs::write(&target, "2").unwrap();
        assert_ne!(before, digest_source_tree(tree.path()).unwrap());
    }

    /// Build output, VCS data and other toolchains' dependency trees are not
    /// inputs, at any depth.
    #[test]
    fn non_source_directories_do_not_affect_the_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "x").unwrap();
        let before = digest_source_tree(root).unwrap();
        for dir in NON_SOURCE_DIRS {
            std::fs::create_dir_all(root.join("src").join(dir)).unwrap();
            std::fs::write(root.join("src").join(dir).join("f"), "junk").unwrap();
        }
        assert_eq!(before, digest_source_tree(root).unwrap());
    }

    /// A file over the size limit is identified by length and mtime, so it
    /// still changes the version when it grows.
    #[test]
    fn a_large_file_still_changes_the_digest() {
        let tmp = tempfile::tempdir().unwrap();
        let big = tmp.path().join("fixture.bin");
        std::fs::write(&big, vec![0u8; CONTENT_DIGEST_LIMIT as usize + 1]).unwrap();
        let before = digest_source_tree(tmp.path()).unwrap();
        std::fs::write(&big, vec![0u8; CONTENT_DIGEST_LIMIT as usize + 2]).unwrap();
        assert_ne!(before, digest_source_tree(tmp.path()).unwrap());
    }
}
