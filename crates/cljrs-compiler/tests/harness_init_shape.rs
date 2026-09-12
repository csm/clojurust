//! The shape the AOT harness emits for a project's `:rust :init` hook.
//!
//! `aot::native_init_code` emits:
//!
//! ```ignore
//! let mut __registry = cljrs_interop::Registry::new(globals.clone());
//! #[allow(unused_unsafe)]
//! unsafe {
//!     my_project::cljrs_init(&mut __registry);
//! }
//! ```
//!
//! This file *is* that shape, compiled against both spellings an extension may
//! use for its entry point. It is a compile-time test: if the emitted shape
//! stops accepting either spelling, this file stops building.
//!
//! Before the `unsafe` wrapper, the emitted call was bare, and an extension
//! declaring the honest `unsafe extern "C" fn` could not be AOT-compiled at
//! all:
//!
//! ```text
//! error[E0308]: mismatched types
//!   expected safe fn, found unsafe fn
//! ```
//!
//! Nothing caught it because no test compiles a program with a `:rust :init`.

/// Stands in for `cljrs_interop::Registry`; only the call shape is under test.
pub struct Registry;

/// The honest signature for a function that dereferences a raw pointer, and
/// what the extension crates in this workspace declare.
unsafe extern "C" fn unsafe_init(registry: *mut Registry) {
    let _ = unsafe { &mut *registry };
}

/// What `docs/book/src/rust-interop/project-setup.md` teaches.
extern "C" fn safe_init(registry: *mut Registry) {
    let _ = unsafe { &mut *registry };
}

#[test]
fn the_emitted_shape_accepts_an_unsafe_entry_point() {
    let mut __registry = Registry;
    #[allow(unused_unsafe)]
    unsafe {
        unsafe_init(&mut __registry);
    }
}

/// The `allow(unused_unsafe)` earns its place here: a safe entry point inside
/// `unsafe` is redundant, and the generated crate would warn without it.
#[test]
fn the_emitted_shape_accepts_a_safe_entry_point() {
    let mut __registry = Registry;
    #[allow(unused_unsafe)]
    unsafe {
        safe_init(&mut __registry);
    }
}
