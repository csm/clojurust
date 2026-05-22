//! Static export registry — collects every `#[export]`-annotated function and
//! lets a `cljrs_init` implementation register them all in one call.

use cljrs_value::NativeFn;

use crate::registry::Registry;

/// A single entry produced by `#[export]`.
///
/// The `inventory` crate collects all of these at link time; call
/// [`register_exports`] to intern them into a [`Registry`].
pub struct ExportEntry {
    /// Fully-qualified Clojure name, e.g. `"math/add"`.
    pub qualified: &'static str,
    /// Factory that constructs the `NativeFn` (called once per registration).
    pub make_fn: fn() -> NativeFn,
}

inventory::collect!(ExportEntry);

/// Register every `#[export]`-annotated function into `registry`.
///
/// Call this inside your `cljrs_init` function to avoid listing each function
/// individually:
///
/// ```rust,ignore
/// pub fn cljrs_init(registry: &mut Registry) {
///     cljrs_interop::register_exports(registry);
/// }
/// ```
pub fn register_exports(registry: &mut Registry) {
    for entry in inventory::iter::<ExportEntry> {
        registry.define(entry.qualified, (entry.make_fn)());
    }
}
