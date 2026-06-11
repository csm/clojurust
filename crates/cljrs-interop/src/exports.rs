//! Static export registry — collects every `#[export]`-annotated function and
//! registers them automatically when a [`Registry`] is created.

use cljrs_value::NativeFn;

use crate::registry::Registry;

/// A single entry produced by `#[export]`.
///
/// The `inventory` crate collects all of these at link time.  Every
/// [`Registry`] calls [`register_exports`] in its constructor, so
/// `#[export]`-annotated functions are available without any explicit
/// registration call.
pub struct ExportEntry {
    /// Fully-qualified Clojure name, e.g. `"math/add"`.
    pub qualified: &'static str,
    /// Factory that constructs the `NativeFn` (called once per registration).
    pub make_fn: fn() -> NativeFn,
}

inventory::collect!(ExportEntry);

/// Intern every `#[export]`-annotated function into `registry`, and record
/// every [`ProvenanceEntry`] submitted via
/// [`register_provenance!`][crate::register_provenance].
///
/// Called automatically by [`Registry::new`]; you only need this directly if
/// you are constructing a `Registry` by other means or want to re-register
/// into a second environment.
pub fn register_exports(registry: &Registry) {
    for entry in inventory::iter::<ExportEntry> {
        registry.define(entry.qualified, (entry.make_fn)());
    }
    for entry in inventory::iter::<ProvenanceEntry> {
        registry.set_provenance(entry.ns, entry.commit);
    }
}

/// Provenance of a native package: the git commit `ns` was built from.
///
/// Collected by `inventory` at link time (see
/// [`register_provenance!`][crate::register_provenance]) and recorded into
/// the environment whenever a [`Registry`] is constructed.
pub struct ProvenanceEntry {
    /// The Clojure namespace the native package registers into.
    pub ns: &'static str,
    /// The git commit hash the package was built from.
    pub commit: &'static str,
}

inventory::collect!(ProvenanceEntry);

/// Declare the git commit a native package's namespace was built from.
///
/// Place once per exported namespace, typically with a commit captured by a
/// build script:
///
/// ```rust,ignore
/// // build.rs: println!("cargo:rustc-env=CLJRS_PKG_COMMIT={sha}");
/// cljrs_interop::register_provenance!("math", env!("CLJRS_PKG_COMMIT"));
/// ```
///
/// Pinned lookups (`math/add@<sha>`) that fall back to the native binary
/// then verify the pin against this commit instead of silently resolving to
/// whatever is loaded.
#[macro_export]
macro_rules! register_provenance {
    ($ns:expr, $commit:expr) => {
        $crate::inventory::submit! {
            $crate::ProvenanceEntry { ns: $ns, commit: $commit }
        }
    };
}
