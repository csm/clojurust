fn main() {
    // Tell Cargo not to re-run this script unless build.rs itself changes.
    // Without this, Cargo re-runs any build script that emits no
    // rerun-if-changed directives on every single build.
    println!("cargo::rerun-if-changed=build.rs");
}
