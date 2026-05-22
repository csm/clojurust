//! Rust ↔ Clojure interop layer for clojurust.
//!
//! This crate provides the building blocks for Rust code to interact with
//! the clojurust runtime:
//!
//! - **`NativeObject`** re-export from `cljrs-value` — trait for opaque Rust
//!   structs wrapped as Clojure values
//! - **`FromValue` / `IntoValue`** — type-safe conversion between `Value` and
//!   Rust types
//! - **`wrap_result`** — convert `Result<T, E>` to `ValueResult<Value>`
//! - **`wrap_fn*`** — helpers to register Rust functions with automatic
//!   argument marshalling
//! - **`#[export]`** — proc-macro for automatic function registration
//! - **`register_exports`** — register all `#[export]`-annotated functions at once

pub mod error;
pub mod exports;
pub mod marshal;
pub mod register;
pub mod registry;

// Re-export the core interop traits from cljrs-value so downstream crates
// only need to depend on cljrs-interop.
pub use cljrs_gc::{GcPtr, MarkVisitor, Trace};
pub use cljrs_value::native_object::{NativeObject, NativeObjectBox, gc_native_object};
pub use cljrs_value::{Arity, NativeFn, Value, ValueError, ValueResult};

pub use error::wrap_result;
pub use exports::{ExportEntry, register_exports};
pub use marshal::{FromValue, IntoValue};
pub use register::{wrap_fn_variadic, wrap_fn0, wrap_fn1, wrap_fn2, wrap_fn3};
pub use registry::{InitFn, Registry};

// Re-export the proc-macro so users write `#[cljrs_interop::export(...)]`.
pub use cljrs_export_macro::export;

// Re-export inventory so the generated `::cljrs_interop::inventory::submit!`
// path resolves correctly inside user crates.
#[doc(hidden)]
pub use inventory;
