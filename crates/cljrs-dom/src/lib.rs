//! `cljrs-dom` — clean DOM interaction API for WASM targets.
//!
//! Registers the `cljrs.dom` namespace with idiomatic Clojure-flavoured
//! wrappers around the browser DOM: kebab-case names, `!`-suffixed
//! mutations, events as Clojure maps, and `core.async` channel support.
#![allow(clippy::type_complexity)]
// `CljChannel` is not `Send + Sync`, but wasm32 is single-threaded so wrapping
// it in `Arc` (as the core.async layer does) is sound here.
#![allow(clippy::arc_with_non_send_sync)]

use std::cell::RefCell;
use std::sync::Arc;

use cljrs_env::env::GlobalEnv;

#[cfg(target_arch = "wasm32")]
pub mod events;
#[cfg(target_arch = "wasm32")]
pub mod fns;
#[cfg(target_arch = "wasm32")]
pub mod node;

thread_local! {
    /// Globals store for DOM event callbacks that fire outside the normal
    /// eval context (i.e., from JS event dispatch, not from Clojure eval).
    pub(crate) static DOM_GLOBALS: RefCell<Option<Arc<GlobalEnv>>> =
        const { RefCell::new(None) };
}

/// Install the `GlobalEnv` for use by DOM event callbacks.
///
/// Must be called from `Repl::new()` before any eval.
pub fn set_globals(globals: Arc<GlobalEnv>) {
    DOM_GLOBALS.with(|g| *g.borrow_mut() = Some(globals));
}

/// Register all `cljrs.dom` native functions into `globals`.
pub fn register(globals: &Arc<GlobalEnv>) {
    #[cfg(target_arch = "wasm32")]
    fns::register(globals);
    #[cfg(not(target_arch = "wasm32"))]
    let _ = globals;
}
