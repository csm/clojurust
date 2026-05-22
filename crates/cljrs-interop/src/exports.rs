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

/// Intern every `#[export]`-annotated function into `registry`.
///
/// Called automatically by [`Registry::new`]; you only need this directly if
/// you are constructing a `Registry` by other means or want to re-register
/// into a second environment.
pub fn register_exports(registry: &Registry) {
    for entry in inventory::iter::<ExportEntry> {
        registry.define(entry.qualified, (entry.make_fn)());
    }
}
