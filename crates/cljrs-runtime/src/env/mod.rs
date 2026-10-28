#![allow(clippy::result_large_err)]
#![allow(clippy::arc_with_non_send_sync)]

pub mod apply;
pub mod async_hook;
pub mod callback;
pub mod depth;
pub mod dynamics;
// The `env` module keeps its name so the `cljrs_runtime::env::env`
// path matches the pre-merge `cljrs_runtime::env::env` path.
#[allow(clippy::module_inception)]
pub mod env;
pub mod error;
pub mod gas;
pub mod gc_roots;
pub mod loader;
pub mod monitor;
pub mod policy;
pub mod taps;
pub mod vcs;
#[cfg(not(target_arch = "wasm32"))]
pub mod versioned;

pub use async_hook::AsyncRuntime;
