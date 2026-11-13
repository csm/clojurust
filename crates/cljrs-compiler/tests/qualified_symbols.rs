//! Qualified symbol identity must survive native compilation.
use cljrs_compiler::extensions::{CompileSession, ExtensionSet};

#[test]
fn aot_qualified_symbol_namespace_name_and_map_lookup() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("qualified.cljrs");
    let binary = directory.path().join("qualified");
    std::fs::write(
        &source,
        r#"
      (println (namespace 'bulwark.commands/register!))
      (println (name 'bulwark.commands/register!))
      (println (= 'bulwark.commands/register! (symbol "bulwark.commands" "register!")))
      (println (get {'bulwark.commands/register! :found} (symbol "bulwark.commands" "register!")))
      (println (namespace '/))
      (println (name 'clojure.core//))
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
        "bulwark.commands\nregister!\ntrue\n:found\nnil\n/"
    );
}
