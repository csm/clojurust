//! End-to-end AOT → WebAssembly compilation test.
//!
//! Writes a `.cljrs` source file, drives the full front-end +
//! [`cljrs_compiler::aot::compile_file_to_wasm`] over it, and validates the
//! emitted module with `wasmparser` — the CLI's `--target wasm` path, minus the
//! argument parsing.

use std::sync::Mutex;

/// Serialize the tests: each boots a full stdlib environment.
static LOCK: Mutex<()> = Mutex::new(());

/// Compile `source` to a wasm module via the AOT wasm backend and return its
/// bytes.
fn compile_to_wasm(name: &str, source: &str) -> Vec<u8> {
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let dir = std::env::temp_dir().join("cljrs_wasm_tests");
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join(format!("{name}.cljrs"));
    let out_path = dir.join(format!("{name}.wasm"));
    std::fs::write(&src_path, source).unwrap();

    cljrs_compiler::aot::compile_file_to_wasm(&src_path, &out_path, &[])
        .unwrap_or_else(|e| panic!("compile_file_to_wasm failed: {e}"));

    std::fs::read(&out_path).unwrap()
}

/// A file of simple top-level `defn`s compiles to a valid wasm module.
#[test]
fn compiles_simple_defns_to_valid_wasm() {
    let bytes = compile_to_wasm(
        "simple",
        r#"
        (defn add1 [x] (+ x 1))
        (defn pick [c a b] (if c a b))
        "#,
    );
    // Real wasm: magic + version, and structurally valid per wasmparser.
    assert_eq!(&bytes[..4], b"\0asm", "wasm magic");
    wasmparser::Validator::new()
        .validate_all(&bytes)
        .expect("emitted module should validate");
}

/// A loop with unboxed `Long` arithmetic (the `0`-seeded counter) compiles and
/// validates — exercising the typeinfer + checked-`i64.add` path through the CLI
/// entry point, not just the emitter unit tests.
#[test]
fn compiles_unboxed_loop_to_valid_wasm() {
    let bytes = compile_to_wasm(
        "loopsum",
        r#"
        (defn sum-to [n]
          (loop [i 0 acc 0]
            (if (< i n)
              (recur (+ i 1) (+ acc i))
              acc)))
        "#,
    );
    wasmparser::Validator::new()
        .validate_all(&bytes)
        .expect("emitted module should validate");
}
