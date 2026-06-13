fn main() {
    // Capture the host rustc version for the ABI handshake: a pinned-package
    // wrapper dylib is only loaded when it was built by the exact same
    // compiler as this binary's runtime crates.
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = std::process::Command::new(rustc)
        .arg("-V")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=CLJRS_DYLIB_RUSTC={version}");
    println!("cargo:rerun-if-changed=build.rs");
}
