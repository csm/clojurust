use cljrs_compiler::extensions::{CompileSession, ExtensionSet};

#[test]
fn compiled_swap_retries_a_conflicting_update() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("swap.cljrs");
    let binary = directory.path().join("swap");
    std::fs::write(
        &source,
        r#"
      (def state (atom {:left 0}))
      (def first-call (atom true))
      (def calls (atom 0))
      (defn update [old]
        (swap! calls inc)
        (when (compare-and-set! first-call true false)
          (reset! state {:left 0 :right 1}))
        (assoc old :left 1))
      (swap! state update)
      (println (= {:left 1 :right 1} @state))
      (println @calls)
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
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "true\n2");
}
