//! Two extension crates must not export the same C-ABI init symbol.
//!
//! This is the only test binary that links more than one plugin crate, which is
//! what the AOT path does when a program pulls in two extensions.

/// A shared `#[no_mangle] cljrs_init` does NOT fail to link. The linker resolves
/// every reference to a single definition, so both Rust paths become the same
/// address and one crate's registration silently replaces the other's: the
/// second namespace is simply never defined, with no diagnostic anywhere.
///
/// Naming each export after its crate is what makes that unrepresentable, and
/// this pins it. Measured before the rename: the two addresses were equal.
#[test]
fn extension_init_symbols_are_unique_per_crate() {
    let base64 = cljrs_base64::cljrs_init_cljrs_base64 as usize;
    let blake3 = cljrs_blake3::cljrs_init_cljrs_blake3 as usize;
    assert_ne!(
        base64, blake3,
        "two extension crates resolved their init to one address, so one \
         plugin's registration silently replaces the other's"
    );
}
