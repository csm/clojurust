//! End-to-end tests for `cljrs run`'s `-main` invocation, covering both
//! synchronous and `^:async` entry points.
//!
//! An `^:async -main` returns a `Future` immediately, with its body queued on
//! the async `LocalSet`; `run` must await that future so the body runs to
//! completion before the process exits. These tests drive the built binary
//! (via the `CARGO_BIN_EXE_cljrs` path Cargo provides) against temp fixtures.

use std::path::PathBuf;
use std::process::Command;

/// Write `src` to a uniquely named temp `.cljrs` file and return its path.
fn write_fixture(name: &str, src: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "cljrs-run-main-{}-{}.cljrs",
        name,
        std::process::id()
    ));
    std::fs::write(&path, src).expect("write fixture");
    path
}

fn run_file(path: &PathBuf) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("run")
        .arg(path)
        .output()
        .expect("run cljrs binary")
}

#[test]
fn run_invokes_sync_main() {
    let path = write_fixture("sync", r#"(defn -main [& args] (println "sync-main ran"))"#);
    let out = run_file(&path);
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("sync-main ran"), "stdout was: {stdout}");
}

#[test]
fn run_awaits_async_main() {
    // The body runs only if `run` awaits the future returned by the `^:async`
    // `-main`. Before the fix it returned the future immediately and the body
    // never executed, so nothing was printed.
    let path = write_fixture(
        "async",
        r#"
(defn ^:async fetch [x] (* x 2))
(defn ^:async -main [& args]
  (let [v (await (fetch 21))]
    (println "async-main result:" v)))
"#,
    );
    let out = run_file(&path);
    let _ = std::fs::remove_file(&path);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("async-main result: 42"),
        "async -main body did not run to completion; stdout was: {stdout}"
    );
}
