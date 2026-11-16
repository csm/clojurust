use cljrs_compiler::extensions::{CompileSession, ExtensionSet};

/// Issue #515: a compiled `compare-and-set!` must run the atom's validator and
/// notify its watches, exactly like `reset!` and `swap!`.
#[test]
fn compiled_compare_and_set_validates_and_notifies() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("cas.cljrs");
    let binary = directory.path().join("cas");
    std::fs::write(
        &source,
        r#"
      (def a (atom 1 :validator pos?))
      (def hits (atom []))
      (add-watch a :w (fn [_ _ old new] (swap! hits conj [old new])))
      (println (try (compare-and-set! a 1 -1) (catch Exception e :rejected)))
      (println @a)
      (println (compare-and-set! a 5 2))
      (println (compare-and-set! a 1 2))
      (println (apply compare-and-set! [a 2 3]))
      (println @a)
      (println @hits)
    "#,
    )
    .unwrap();
    cljrs_compiler::aot::compile_file(
        &source,
        &binary,
        &CompileSession::new(vec![], ExtensionSet::new()),
    )
    .unwrap();
    let output = std::process::Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        ":rejected\n1\nfalse\ntrue\ntrue\n3\n[[1 2] [2 3]]"
    );
}
