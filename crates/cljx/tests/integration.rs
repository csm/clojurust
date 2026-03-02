//! Source-level integration tests.
//!
//! Fixture `.cljx` files live in `<workspace-root>/tests/fixtures/`.
//! Tests here are `#[ignore]`d until the reader and evaluator land in
//! Phase 2 and Phase 4 respectively.

use std::path::PathBuf;

fn fixtures_dir() -> PathBuf {
    // crates/cljx is two levels below the workspace root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
}

#[test]
#[ignore = "requires Phase 2 reader"]
fn run_fixture_files() {
    let dir = fixtures_dir();
    let entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("fixtures dir should exist")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x == "cljx" || x == "cljc")
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !entries.is_empty(),
        "no .cljx/.cljc fixture files found in {}",
        dir.display()
    );

    for entry in entries {
        let path = entry.path();
        // TODO(Phase 4): feed each file through the interpreter and assert
        // it produces the expected result (encoded in a `; => value` comment).
        println!("fixture: {}", path.display());
    }
}
