//! End-to-end test for `spawn-isolate` under `cljrs run`: the CLI session
//! installs the isolate factory, so a script can start isolates that load
//! their entry namespace from the same `--src-path`.

use std::process::Command;

#[test]
fn run_spawns_isolates_that_load_from_the_source_path() {
    let root = std::env::temp_dir().join(format!("cljrs-spawn-isolate-{}", std::process::id()));
    let src = root.join("src").join("demo");
    std::fs::create_dir_all(&src).expect("create fixture dirs");
    std::fs::write(
        src.join("work.cljrs"),
        r#"(ns demo.work
  (:require [clojure.core.async :refer [spawn-isolate]]))
(defn square [n] (* n n))
(defn nested [n] @(spawn-isolate 'demo.work/square n))
"#,
    )
    .expect("write worker namespace");
    let main = root.join("main.cljrs");
    std::fs::write(
        &main,
        r#"(require '[clojure.core.async :refer [spawn-isolate]])
(defn local [] 1)
(println (mapv deref (mapv #(spawn-isolate 'demo.work/square %) [2 3 4])))
(println @(spawn-isolate 'demo.work/nested 5))
(println (try @(spawn-isolate 'user/local) (catch Exception _ :not-loadable)))
"#,
    )
    .expect("write main script");

    let out = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .arg("run")
        .arg("--src-path")
        .arg(root.join("src"))
        .arg(&main)
        .output()
        .expect("run cljrs binary");
    let _ = std::fs::remove_dir_all(&root);

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(stdout, "[4 9 16]\n25\n:not-loadable\n");
}
