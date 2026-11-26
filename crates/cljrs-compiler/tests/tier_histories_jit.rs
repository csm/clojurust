//! The execution-history matrix (issue #510) with the native JIT attached:
//! [`Op::Promote`](history::Op::Promote) drives each function through
//! background lowering *and* native compilation, settling on both workers,
//! so every history composes rebinding and retained values with published
//! machine code.
//!
//! The histories themselves live in `cljrs-runtime/tests/history/mod.rs`;
//! this binary only adds the configuration that needs `cljrs-compiler`.

#![cfg(not(target_arch = "wasm32"))]

#[path = "../../cljrs-runtime/tests/history/mod.rs"]
mod history;

use history::*;

fn attach_jit(runtime: &cljrs_runtime::Runtime) {
    cljrs_compiler::jit::install(runtime);
}

const NATIVE_JIT: Config = Config {
    name: "native-jit",
    mode: cljrs_runtime::ExecutionMode::Tiered,
    background: true,
    reach: Reach::Native,
    setup: attach_jit,
};

#[test]
fn every_history_with_native_jit() {
    init_thresholds();
    for &h in ALL {
        run(h, &[TREE_WALK, NATIVE_JIT]);
    }
}
