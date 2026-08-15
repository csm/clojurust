//! Shared test scaffolding: the extension set a host supplies to the compiler.
//!
//! The compiler no longer names the optional extension packages itself, so a
//! test that compiles a program using `clojure.core.async`, I/O, networking,
//! charset codecs, or Base64 must supply them exactly as the CLI does.  These
//! packages are dev-dependencies here for that reason.

use std::path::PathBuf;

use cljrs_compiler::extensions::{CompileSession, Extension, ExtensionSet};

/// Every extension the default CLI build ships, in the same order.
pub fn extension_set() -> ExtensionSet {
    ExtensionSet::new()
        .with(Extension::new(
            "cljrs-async",
            cljrs_async::init,
            "cljrs_async::init",
        ))
        .with(Extension::new("cljrs-io", cljrs_io::init, "cljrs_io::init"))
        .with(Extension::new(
            "cljrs-net",
            cljrs_net::init,
            "cljrs_net::init",
        ))
        .with(Extension::new(
            "cljrs-charset",
            cljrs_charset::init,
            "cljrs_charset::init",
        ))
        .with(Extension::new(
            "cljrs-base64",
            cljrs_base64::init,
            "cljrs_base64::init",
        ))
}

/// A compile session over `src_dirs` with the full extension set.
pub fn session(src_dirs: Vec<PathBuf>) -> CompileSession {
    CompileSession::new(src_dirs, extension_set())
}
