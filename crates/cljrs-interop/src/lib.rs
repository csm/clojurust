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
//! - **`wrap_native_fn`** — helpers to register Rust functions with automatic
//!   argument marshalling

pub mod error;
pub mod marshal;

// Re-export the core interop traits from cljrs-value so downstream crates
// only need to depend on cljrs-interop.
pub use cljrs_gc::{GcPtr, MarkVisitor, Trace};
pub use cljrs_value::native_object::{NativeObject, NativeObjectBox, gc_native_object};
pub use cljrs_value::{Value, ValueError, ValueResult};

pub use error::wrap_result;
pub use marshal::{FromValue, IntoValue};
