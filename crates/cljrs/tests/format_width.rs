//! Regression tests for issue #199: `format` width/flag specifiers like `%-5s`.
//!
//! `builtin_format` is a native function shared by all execution tiers (interpreter,
//! JIT, AOT).  The old code fell through to a literal-emit branch for any specifier
//! that contained flags or a width component, so `%-5s` was emitted as the literal
//! string `%-5s` instead of a padded value, and argument consumption was thrown off.
//!
//! Three test groups verify the fix across each execution path.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// ── helpers ───────────────────────────────────────────────────────────────────

fn unique_path(prefix: &str) -> std::path::PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "cljrs_fmt_{}_{nanos}_{seq}_{}.cljrs",
        prefix,
        std::process::id()
    ))
}

/// Run `src` through the interpreter (`cljrs run`, no JIT promotion).
fn run_interp(src: &str) -> String {
    let path = unique_path("interp");
    std::fs::write(&path, src).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["run"])
        .arg(&path)
        .output()
        .expect("spawn cljrs");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "cljrs exited {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

/// Run `src` under JIT with a very low threshold so hot functions get compiled.
fn run_jit(src: &str) -> String {
    let path = unique_path("jit");
    std::fs::write(&path, src).expect("write script");
    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", "50", "run"])
        .arg(&path)
        .env("CLJRS_EAGER_LOWER", "1")
        .output()
        .expect("spawn cljrs");
    let _ = std::fs::remove_file(&path);
    assert!(
        output.status.success(),
        "cljrs (jit) exited {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

// ── Clojure format script ─────────────────────────────────────────────────────

/// A script that exercises format specifiers from the issue and prints each
/// result on its own line, prefixed by `result:` so assertions are clear.
const FMT_SCRIPT: &str = r#"
(println (format "%-5s" "ab"))
(println (format "%-5s|" "ab"))
(println (format "%-15s%s" "a" "b"))
(println (format "%s %s" "a" "b"))
(println (format "%d" 5))
(println (format "%5d" 42))
(println (format "%-5d" 42))
(println (format "%05d" 42))
(println (format "%+d" 42))
(println (format "%5s" "ab"))
"#;

fn check_output(out: &str, label: &str) {
    let lines: Vec<&str> = out.lines().collect();
    assert_eq!(
        lines.len(),
        10,
        "{label}: expected 10 output lines, got:\n{out}"
    );

    // (format "%-5s" "ab")  → "ab   "
    assert_eq!(lines[0], "ab   ", "{label}: %-5s wrong");
    // (format "%-5s|" "ab") → "ab   |"
    assert_eq!(lines[1], "ab   |", "{label}: %-5s| wrong");
    // (format "%-15s%s" "a" "b") → "a              b"
    assert_eq!(
        lines[2], "a              b",
        "{label}: %-15s%s wrong — got {:?}",
        lines[2]
    );
    // bare %s still works
    assert_eq!(lines[3], "a b", "{label}: %s %s wrong");
    // bare %d still works
    assert_eq!(lines[4], "5", "{label}: %d wrong");
    // right-justify integer
    assert_eq!(lines[5], "   42", "{label}: %5d wrong");
    // left-justify integer
    assert_eq!(lines[6], "42   ", "{label}: %-5d wrong");
    // zero-pad integer
    assert_eq!(lines[7], "00042", "{label}: %05d wrong");
    // plus flag
    assert_eq!(lines[8], "+42", "{label}: %+d wrong");
    // right-justify string
    assert_eq!(lines[9], "   ab", "{label}: %5s wrong");
}

// ── Interpreter tests ─────────────────────────────────────────────────────────

#[test]
fn format_width_flags_interpreter() {
    let out = run_interp(FMT_SCRIPT);
    check_output(&out, "interpreter");
}

// ── JIT tests ─────────────────────────────────────────────────────────────────

/// Call format from inside a hot loop so the JIT compiles the surrounding
/// function and exercises the builtin call seam under native code.
#[test]
fn format_width_flags_jit() {
    // Run the same checks once (confirming builtins work from JIT context),
    // then run the hot loop to confirm promotion doesn't break anything.
    let src = r#"
(defn check-format [i]
  (let [r1 (format "%-5s" "ab")
        r2 (format "%-5s|" "ab")
        r3 (format "%-15s%s" "a" "b")
        r4 (format "%05d" 42)
        r5 (format "%5s" "ab")]
    (when (or (not= r1 "ab   ")
              (not= r2 "ab   |")
              (not= r3 "a              b")
              (not= r4 "00042")
              (not= r5 "   ab"))
      (println "FAIL at i=" i
               "r1=" r1 "r2=" r2 "r3=" r3 "r4=" r4 "r5=" r5))))

(dotimes [i 10000]
  (check-format i))
(println "done")
"#;
    let out = run_jit(src);
    assert!(
        !out.contains("FAIL"),
        "format width/flags produced wrong results under JIT:\n{out}"
    );
    assert!(
        out.trim_end().ends_with("done"),
        "expected 'done' at end of JIT output:\n{out}"
    );
}

// AOT path: the same `builtin_format` native function is embedded in AOT
// binaries.  An equivalent test lives in
// `crates/cljrs-compiler/tests/aot_e2e.rs` behind the `aot_full_test` feature.
