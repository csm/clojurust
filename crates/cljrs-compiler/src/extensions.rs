//! Extension descriptors, and the compile session that carries them.
//!
//! A clojurust program can pull in namespaces that no compiler needs to know
//! about: `clojure.core.async`, I/O, networking, charset codecs, Base64.  Two
//! things have to happen for each of them during an AOT compile:
//!
//! 1. **At compile time** the extension's namespaces must exist in the
//!    bootstrap environment, or a `(require '[clojure.core.async …])` in the
//!    source fails during macro expansion.
//! 2. **At run time** the generated harness must register the same
//!    namespaces, or the produced binary starts without them.
//!
//! Before this stage `aot.rs` did both by naming the packages directly —
//! `cljrs_async::init(&globals)` and friends at three sites, a hard-coded
//! block of `cljrs_io::init(&globals);` lines in the harness template, and
//! the packages in the compiler's own `[dependencies]`.  That made the
//! compiler backend the thing that decides what the product ships, and it
//! meant a compiler build pulled in the network stack whether or not anything
//! used it.
//!
//! An [`Extension`] instead describes one extension generically: the package
//! to add to the harness, a function pointer to register it here, and the
//! path to that same function for the generated source.  The *host* — the CLI
//! from its enabled features and project configuration, or an embedding
//! application — assembles the [`ExtensionSet`] and hands it to the compiler
//! in a [`CompileSession`].
//!
//! ```rust,ignore
//! let extensions = ExtensionSet::new()
//!     .with(Extension::new("cljrs-async", cljrs_async::init, "cljrs_async::init"))
//!     .with(Extension::new("cljrs-io", cljrs_io::init, "cljrs_io::init"));
//!
//! let session = CompileSession::new(src_dirs, extensions).opacity(policy);
//! cljrs_compiler::aot::compile_file(&entry, &out, &session)?;
//! ```

use std::path::{Path, PathBuf};
use std::sync::Arc;

use cljrs_runtime::env::env::GlobalEnv;

use crate::aot::OpacityPolicy;

/// One optional runtime extension an AOT-compiled program may need.
#[derive(Clone, Copy)]
pub struct Extension {
    /// Cargo package name, added to the generated harness's `[dependencies]`
    /// (e.g. `"cljrs-io"`).
    pub crate_name: &'static str,
    /// Registers the extension's namespaces on a compile-time bootstrap
    /// environment, so `require` and macro expansion resolve them.
    pub register: fn(&Arc<GlobalEnv>),
    /// Path to that same function in generated harness source (e.g.
    /// `"cljrs_io::init"`).  The harness calls it with `&globals`.
    pub init_path: &'static str,
}

impl Extension {
    pub const fn new(
        crate_name: &'static str,
        register: fn(&Arc<GlobalEnv>),
        init_path: &'static str,
    ) -> Self {
        Self {
            crate_name,
            register,
            init_path,
        }
    }
}

impl std::fmt::Debug for Extension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Extension")
            .field("crate_name", &self.crate_name)
            .field("init_path", &self.init_path)
            .finish()
    }
}

/// The extensions one compile should make available, in registration order.
#[derive(Clone, Debug, Default)]
pub struct ExtensionSet {
    extensions: Vec<Extension>,
}

impl ExtensionSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an extension (builder form).  Order is preserved: extensions are
    /// registered in the order added, so one that depends on another comes
    /// second.
    pub fn with(mut self, extension: Extension) -> Self {
        self.extensions.push(extension);
        self
    }

    pub fn push(&mut self, extension: Extension) {
        self.extensions.push(extension);
    }

    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }

    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Extension> {
        self.extensions.iter()
    }

    /// Register every extension's namespaces on a compile-time environment.
    pub fn register_all(&self, globals: &Arc<GlobalEnv>) {
        for extension in &self.extensions {
            (extension.register)(globals);
        }
    }

    /// Package names for the generated harness's `[dependencies]`.
    pub fn crate_names(&self) -> Vec<&'static str> {
        self.extensions.iter().map(|e| e.crate_name).collect()
    }

    /// Rust statements registering every extension, for the generated
    /// harness's `run()`.  Empty when the set is.
    pub fn harness_init_code(&self) -> String {
        let mut out = String::new();
        for extension in &self.extensions {
            out.push_str(&format!("    {}(&globals);\n", extension.init_path));
        }
        out
    }
}

impl<'a> IntoIterator for &'a ExtensionSet {
    type Item = &'a Extension;
    type IntoIter = std::slice::Iter<'a, Extension>;
    fn into_iter(self) -> Self::IntoIter {
        self.extensions.iter()
    }
}

/// Everything one AOT compile needs beyond its input and output paths.
///
/// Prepared by the host (the CLI, an embedding application) and handed to
/// [`aot::compile_file`](crate::aot::compile_file) or
/// [`aot::compile_file_to_wasm`](crate::aot::compile_file_to_wasm).
#[derive(Clone, Debug, Default)]
pub struct CompileSession {
    src_dirs: Vec<PathBuf>,
    extensions: ExtensionSet,
    rust_config: Option<cljrs_deps::RustConfig>,
    verify_commit_signatures: bool,
    opacity: OpacityPolicy,
}

impl CompileSession {
    /// A session over `src_dirs` (searched when `require` resolves a
    /// namespace to a file) with `extensions` available to the compiled
    /// program.
    pub fn new(src_dirs: Vec<PathBuf>, extensions: ExtensionSet) -> Self {
        Self {
            src_dirs,
            extensions,
            rust_config: None,
            verify_commit_signatures: false,
            opacity: OpacityPolicy::default(),
        }
    }

    /// Depend on the user's Rust crate and call its `cljrs_init` hook before
    /// loading any Clojure code (`:rust` in `cljrs.edn`).
    pub fn rust_config(mut self, config: Option<cljrs_deps::RustConfig>) -> Self {
        self.rust_config = config;
        self
    }

    /// Verify the commit signature of every versioned pin resolved during
    /// compilation.  The produced binary trusts its embedded sources, so
    /// verification happens here, at compile time.
    pub fn verify_commit_signatures(mut self, enabled: bool) -> Self {
        self.verify_commit_signatures = enabled;
        self
    }

    /// What to do when the produced artifact would still carry readable
    /// Clojure source.
    pub fn opacity(mut self, policy: OpacityPolicy) -> Self {
        self.opacity = policy;
        self
    }

    pub fn src_dirs(&self) -> &[PathBuf] {
        &self.src_dirs
    }

    pub fn extensions(&self) -> &ExtensionSet {
        &self.extensions
    }

    pub fn rust_config_ref(&self) -> Option<&cljrs_deps::RustConfig> {
        self.rust_config.as_ref()
    }

    pub fn verifies_commit_signatures(&self) -> bool {
        self.verify_commit_signatures
    }

    pub fn opacity_policy(&self) -> OpacityPolicy {
        self.opacity
    }

    /// Append a source directory.
    pub fn add_src_dir(&mut self, dir: impl AsRef<Path>) {
        self.src_dirs.push(dir.as_ref().to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &Arc<GlobalEnv>) {}

    #[test]
    fn harness_init_code_is_one_call_per_extension_in_order() {
        let set = ExtensionSet::new()
            .with(Extension::new("cljrs-async", noop, "cljrs_async::init"))
            .with(Extension::new("cljrs-io", noop, "cljrs_io::init"));
        assert_eq!(
            set.harness_init_code(),
            "    cljrs_async::init(&globals);\n    cljrs_io::init(&globals);\n"
        );
        assert_eq!(set.crate_names(), vec!["cljrs-async", "cljrs-io"]);
    }

    #[test]
    fn empty_set_emits_nothing() {
        let set = ExtensionSet::new();
        assert!(set.is_empty());
        assert_eq!(set.harness_init_code(), "");
        assert!(set.crate_names().is_empty());
    }
}
