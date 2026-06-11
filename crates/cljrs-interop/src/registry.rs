//! `Registry` — entry point for native Rust code to register Clojure-visible
//! functions and values.
//!
//! # Usage
//!
//! User crates that embed Rust code alongside Clojure sources implement a
//! `cljrs_init` function matching the [`InitFn`] signature:
//!
//! ```rust,ignore
//! use cljrs_interop::{Registry, wrap_fn1};
//!
//! pub fn cljrs_init(registry: &mut Registry) {
//!     registry.define("my.project/greet", wrap_fn1("greet", |name: String| {
//!         Ok::<String, String>(format!("Hello, {name}!"))
//!     }));
//! }
//! ```
//!
//! The `cljrs compile` toolchain reads the `:rust :init` key from `cljrs.edn`
//! (e.g. `"my_project::cljrs_init"`) and emits a generated `main.rs` that calls
//! it before loading any Clojure source.

use std::sync::Arc;

use cljrs_env::env::GlobalEnv;
use cljrs_gc::GcPtr;
use cljrs_value::{NativeFn, Value};

use crate::exports::register_exports;

// ── InitFn ────────────────────────────────────────────────────────────────────

/// The expected signature of a Rust-side hook-registration function.
///
/// Name your function `cljrs_init` and list it under `:rust :init` in
/// `cljrs.edn` so the build toolchain can wire it up automatically:
///
/// ```edn
/// :rust {:crate "."
///        :init  "my_crate::cljrs_init"}
/// ```
pub type InitFn = fn(&mut Registry);

// ── Registry ──────────────────────────────────────────────────────────────────

/// A handle passed to [`InitFn`] implementations so they can register native
/// functions and values into the Clojure namespace table.
///
/// `Registry` wraps an `Arc<GlobalEnv>` and is intentionally thin: it exposes
/// only the operations needed for safe interop, leaving advanced `GlobalEnv`
/// access available via [`Registry::env`].
pub struct Registry {
    env: Arc<GlobalEnv>,
}

impl Registry {
    /// Create a `Registry` backed by `env`.
    ///
    /// Automatically registers every `#[export]`-annotated function found in
    /// the binary (via `inventory`).  Called by the generated `main.rs`; user
    /// code receives `&mut Registry` and never constructs one directly.
    pub fn new(env: Arc<GlobalEnv>) -> Self {
        let r = Self { env };
        register_exports(&r);
        r
    }

    /// Register `f` under the fully-qualified Clojure name `"my.ns/my-fn"`.
    ///
    /// The namespace is created if it does not yet exist. The unqualified
    /// symbol after `/` becomes the var name visible to Clojure code.
    ///
    /// Panics if `qualified` contains no `/`; use [`define_in`][Self::define_in]
    /// for plain names.
    pub fn define(&self, qualified: &str, f: NativeFn) {
        let (ns, sym) = split_qualified(qualified)
            .unwrap_or_else(|| panic!("Registry::define: {qualified:?} has no '/' separator"));
        self.env
            .intern(ns, Arc::from(sym), Value::NativeFunction(GcPtr::new(f)));
    }

    /// Register `f` into an explicit namespace under a plain (unqualified) name.
    ///
    /// Equivalent to `define("ns/name", f)` but avoids string formatting when
    /// the parts are already separate.
    pub fn define_in(&self, ns: &str, name: &str, f: NativeFn) {
        self.env
            .intern(ns, Arc::from(name), Value::NativeFunction(GcPtr::new(f)));
    }

    /// Record the git commit the native package providing namespace `ns` was
    /// built from.
    ///
    /// The versioned resolver consults this when a pinned symbol
    /// (`ns/name@<sha>`) falls back to a native function: a matching
    /// provenance is silent; a mismatch warns (or errors under
    /// `--enforce-native-versions`).  Typically driven by the
    /// [`register_provenance!`][crate::register_provenance] macro with a
    /// build-script-provided commit, e.g. `env!("CLJRS_PKG_COMMIT")`.
    pub fn set_provenance(&self, ns: &str, commit: &str) {
        self.env.set_native_provenance(ns, commit);
    }

    /// Access the underlying `GlobalEnv` for operations beyond simple `define`
    /// (e.g. registering builtin namespace sources, setting aliases).
    pub fn env(&self) -> &Arc<GlobalEnv> {
        &self.env
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Split `"my.ns/sym"` into `Some(("my.ns", "sym"))`, or `None` if no `/`.
fn split_qualified(s: &str) -> Option<(&str, &str)> {
    s.rfind('/').map(|idx| (&s[..idx], &s[idx + 1..]))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::split_qualified;

    #[test]
    fn split_simple() {
        assert_eq!(split_qualified("my.ns/my-fn"), Some(("my.ns", "my-fn")));
    }

    #[test]
    fn split_nested_slash() {
        // rfind: last '/' wins — shouldn't arise in practice but must be deterministic.
        assert_eq!(split_qualified("a/b/c"), Some(("a/b", "c")));
    }

    #[test]
    fn split_no_slash() {
        assert_eq!(split_qualified("plain"), None);
    }
}
