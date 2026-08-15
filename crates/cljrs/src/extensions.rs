//! The extension set this CLI build ships.
//!
//! The compiler backend does not decide what a compiled program contains: it
//! takes a `cljrs_compiler::extensions::ExtensionSet` and registers exactly
//! that, at compile time (so `require` resolves during macro expansion) and in
//! the generated harness (so the produced binary registers the same
//! namespaces).
//!
//! This module is where the decision actually lives, and it is made the same
//! way the interpreter's is — from the CLI's own Cargo features, so
//! `cljrs run` and `cljrs compile` of the same program see the same
//! namespaces:
//!
//! | Feature | Extensions |
//! |---|---|
//! | `async` | `clojure.core.async`, `^:async` dispatch, `await`; file I/O |
//! | `net` | network transports and protocols (implies `async`) |
//! | `charset` | charset codecs and stream adapters |
//! | `base64` | Base64 codecs |
//!
//! An embedding application that links the compiler directly builds its own
//! set the same way.

#[cfg(any(
    feature = "async",
    feature = "net",
    feature = "charset",
    feature = "base64"
))]
use cljrs_compiler::extensions::Extension;
use cljrs_compiler::extensions::ExtensionSet;

/// The extensions available to programs this CLI compiles.
///
/// Mirrors what [`crate::setup_globals`] registers on an interpreted runtime.
pub fn default_set() -> ExtensionSet {
    #[allow(unused_mut)]
    let mut set = ExtensionSet::new();

    #[cfg(feature = "async")]
    {
        set.push(Extension::new(
            "cljrs-async",
            cljrs_async::init,
            "cljrs_async::init",
        ));
        set.push(Extension::new("cljrs-io", cljrs_io::init, "cljrs_io::init"));
    }
    #[cfg(feature = "net")]
    set.push(Extension::new(
        "cljrs-net",
        cljrs_net::init,
        "cljrs_net::init",
    ));
    #[cfg(feature = "charset")]
    set.push(Extension::new(
        "cljrs-charset",
        cljrs_charset::init,
        "cljrs_charset::init",
    ));
    #[cfg(feature = "base64")]
    set.push(Extension::new(
        "cljrs-base64",
        cljrs_base64::init,
        "cljrs_base64::init",
    ));

    set
}
