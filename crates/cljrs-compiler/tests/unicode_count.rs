//! Native string counts use the interpreter's Unicode scalar indices.
use cljrs_compiler::extensions::{CompileSession, ExtensionSet};

#[test]
fn aot_string_counts_and_clipping_use_characters() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("unicode.cljrs");
    let binary = directory.path().join("unicode");
    std::fs::write(
        &source,
        r#"
      (defn clip [text limit]
        (if (> (count text) limit) (subs text 0 limit) text))
      (println (count ""))
      (println (count "ascii"))
      (println (count "café"))
      (println (count "日本語"))
      (println (count "😀"))
      (println (count "é"))
      (println (clip "😀é" 3))
      (println (clip "😀é" 1))
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
        "0\n5\n4\n3\n1\n2\n😀é\n😀"
    );
}
