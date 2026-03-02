//! Rust ↔ Clojure interop layer for clojurust.
//!
//! Phase 9 will implement:
//! - `#[cljx::export]` proc-macro to expose Rust `fn` as native functions
//! - Type marshalling: `Value` ↔ Rust primitives and structs
//! - `NativeObject` variant for opaque Rust structs behind GC
//! - Error bridging: Rust `Result`/`panic` → Clojure exception
//! - `cljx.rust` namespace with `rust/cast`, `rust/unsafe`, etc.
//! - Dynamic linking of compiled Rust `.so`/`.dylib` at runtime
