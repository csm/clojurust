//! Differential correctness harness for every native execution tier.
//!
//! The source program is byte-for-byte identical in all four runs. Only the
//! execution policy changes: tree-walk, eager IR, eager JIT, or native AOT.
//! Each case emits a small machine-readable value/error record, which keeps
//! tier-specific diagnostics and process formatting out of the comparison.

use std::collections::BTreeMap;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const PROGRAM: &str = r#"
(defn parity-mod [a b] (mod a b))
(defn parity-literals [] [#"[a-z]+" 1/3 1.25M])
(defn parity-fib [n]
  (if (<= n 1)
    n
    (+ (parity-fib (- n 1)) (parity-fib (- n 2)))))
(defn parity-seq-loop [coll]
  (loop [s (seq coll) acc []]
    (if s
      (recur (next s) (conj acc (first s)))
      acc)))
(defn parity-add [a b] (+ a b))
(defn parity-sub [a b] (- a b))
(defn parity-mul [a b] (* a b))
(defn parity-kwargs [& {:keys [a b] :or {b 5}}] [a b])
(defn parity-kwargs-fixed [x & {:keys [a]}] [x a])
(defn parity-seq-rest [& [a b]] [a b])
(defn parity-kwargs-odd []
  (try
    (str "value|" (pr-str (parity-kwargs :a 1 :b)))
    (catch Exception e
      (str "error|" (ex-message e)))))
(defn parity-kwargs-meta-map []
  (parity-kwargs (with-meta {:a 6} {:tag true})))
(defn parity-kwargs-meta-mixed []
  (parity-kwargs :a 1 (with-meta {:b 9} {:tag true})))
(def parity-or-default-count (atom 0))
(defn parity-or-default
  ;; An `:or` default is an ordinary argument of `(get m :k default)`, so it is
  ;; evaluated on every call — including calls that supply the key.  The atom
  ;; counts how many times it actually ran (issue #363).
  [{:keys [x] :or {x (do (swap! parity-or-default-count inc) 1)}}]
  x)
(def parity-input [1 2 3 4])
(def parity-literal-sink nil)
(def parity-seq-sink nil)

;; Drive every function well beyond the low JIT threshold used by the harness.
;; The other policies execute these exact calls in their selected tier too.
(dotimes [_ 50]
  (parity-mod 10 3)
  (parity-fib 8)
  (parity-add 1 2)
  (parity-sub 2 1)
  (parity-mul 2 3)
  (parity-kwargs :a 1)
  (parity-kwargs-fixed 0 :a 3)
  (parity-kwargs {:a 2 :b 4})
  (parity-seq-rest 1 2)
  ;; 50 calls that all supply `:x`; each still evaluates the default.
  (parity-or-default {:x 7})
  ;; Keeping allocation-heavy results reachable also prevents AOT's region
  ;; optimizer from turning the JIT warm-up into an allocation stress test.
  (def parity-literal-sink (parity-literals))
  (def parity-seq-sink (parity-seq-loop parity-input)))

(println (str "mod|value|" (pr-str (parity-mod -10 3))))
;; Keep a direct literal call alongside the hot function call. This locks down
;; ANF binary-fold normalization and the compiler's boxed Mod path separately.
(println (str "mod-literal|value|" (pr-str (mod -10 3))))
(println (str "literals|value|" (pr-str (parity-literals))))
(println (str "recursive-aot|value|" (pr-str (parity-fib 10))))
(println (str "seq-loop|value|" (pr-str (parity-seq-loop parity-input))))
(println (str "overflow-add|value|"
              (pr-str (parity-add 9223372036854775807 1))))
(println (str "overflow-sub-low|value|"
              (pr-str (parity-sub -9223372036854775808 1))))
(println (str "overflow-sub-high|value|"
              (pr-str (parity-sub 9223372036854775807 -1))))
(println (str "overflow-mul|value|"
              (pr-str (parity-mul 9223372036854775807 2))))
;; Hit and miss, in that order, so the effect count below is 50 + 2 either way.
(println (str "or-default-hit|value|" (pr-str (parity-or-default {:x 7}))))
(println (str "or-default-miss|value|" (pr-str (parity-or-default {}))))
(println (str "or-default-effects|value|" (pr-str @parity-or-default-count)))
(println (str "kwargs|value|" (pr-str (parity-kwargs :a 1))))
(println (str "kwargs-fixed|value|" (pr-str (parity-kwargs-fixed 0 :a 3))))
(println (str "kwargs-map|value|" (pr-str (parity-kwargs {:a 2 :b 4}))))
(println (str "seq-rest|value|" (pr-str (parity-seq-rest 1 2))))
(println (str "kwargs-nil|value|" (pr-str (parity-kwargs nil))))
(println (str "kwargs-meta-map|value|"
              (pr-str (parity-kwargs-meta-map))))
(println (str "kwargs-meta-mixed|value|"
              (pr-str (parity-kwargs-meta-mixed))))
(println
  (str "kwargs-odd|" (parity-kwargs-odd)))
(println
  (str "arithmetic-error|"
       (try
         (str "value|" (pr-str (mod 1 0)))
         (catch Exception e
           (str "error|" (ex-message e))))))
"#;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Outcome {
    Value(String),
    Error(String),
}

#[derive(Clone, Copy, Debug)]
enum Tier {
    TreeWalk,
    EagerIr,
    Jit,
    Aot,
}

impl Tier {
    fn name(self) -> &'static str {
        match self {
            Self::TreeWalk => "tree-walk",
            Self::EagerIr => "eager-ir",
            Self::Jit => "jit",
            Self::Aot => "aot",
        }
    }
}

fn wait_with_timeout(mut command: Command, tier: Tier, timeout: Duration) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .unwrap_or_else(|e| panic!("failed to spawn {}: {e}", tier.name()));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                return child
                    .wait_with_output()
                    .unwrap_or_else(|e| panic!("failed to collect {}: {e}", tier.name()));
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let output = child.wait_with_output().expect("collect timed-out child");
                panic!(
                    "{} exceeded {:?} (possible non-terminating compiled loop)\nstdout:\n{}\nstderr:\n{}",
                    tier.name(),
                    timeout,
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr),
                );
            }
            Err(e) => panic!("failed to poll {}: {e}", tier.name()),
        }
    }
}

fn cli_command(script: &std::path::Path, tier: Tier) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cljrs"));
    command
        .env_remove("CLJRS_NO_IR")
        .env_remove("CLJRS_NO_JIT")
        .env_remove("CLJRS_EAGER_LOWER")
        .env_remove("CLJRS_IR_THRESHOLD")
        .env_remove("CLJRS_JIT_THRESHOLD")
        .env_remove("CLJRS_OSR_THRESHOLD");

    match tier {
        Tier::TreeWalk => {
            command.args(["--ir-threshold", "0", "--jit-threshold", "0", "run"]);
        }
        Tier::EagerIr => {
            command
                .args(["--ir-threshold", "0", "--jit-threshold", "0", "run"])
                .env("CLJRS_EAGER_LOWER", "1");
        }
        Tier::Jit => {
            command
                .args([
                    "-X",
                    "debug:jit",
                    "--ir-threshold",
                    "0",
                    "--jit-threshold",
                    "5",
                    "run",
                ])
                .env("CLJRS_EAGER_LOWER", "1");
        }
        Tier::Aot => unreachable!("AOT uses its compiled binary"),
    }
    command.arg(script);
    command
}

fn run_cli(script: &std::path::Path, tier: Tier) -> Output {
    let output = wait_with_timeout(cli_command(script, tier), tier, Duration::from_secs(30));
    assert!(
        output.status.success(),
        "{} failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        tier.name(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    if matches!(tier, Tier::Jit) {
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("compiled  arity_id="),
            "JIT run never published native code; stderr:\n{stderr}"
        );
    }
    output
}

fn run_aot(script: &std::path::Path, binary: &std::path::Path) -> Output {
    let src = script.to_path_buf();
    let bin = binary.to_path_buf();
    thread::Builder::new()
        .name("tier-parity-aot-build".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(move || {
            cljrs_compiler::aot::compile_file(
                &src,
                &bin,
                &cljrs_compiler::extensions::CompileSession::default(),
            )
        })
        .expect("spawn AOT compiler")
        .join()
        .expect("AOT compiler panicked")
        .expect("AOT compilation failed");

    let command = Command::new(binary);
    let output = wait_with_timeout(command, Tier::Aot, Duration::from_secs(30));
    assert!(
        output.status.success(),
        "AOT binary failed with {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    output
}

fn parse_records(tier: Tier, output: &Output) -> BTreeMap<String, Outcome> {
    let stdout = String::from_utf8(output.stdout.clone()).expect("UTF-8 stdout");
    let mut records = BTreeMap::new();
    for line in stdout.lines() {
        let mut fields = line.splitn(3, '|');
        let (Some(case), Some(kind), Some(payload)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let outcome = match kind {
            "value" => Outcome::Value(payload.to_string()),
            "error" => Outcome::Error(payload.to_string()),
            _ => continue,
        };
        assert!(
            records.insert(case.to_string(), outcome).is_none(),
            "{} emitted duplicate record for {case}",
            tier.name()
        );
    }
    assert_eq!(records.len(), 21, "{} stdout:\n{stdout}", tier.name());
    records
}

#[test]
fn values_and_errors_match_across_all_execution_tiers() {
    let temp = tempfile::tempdir().expect("temporary harness directory");
    let script = temp.path().join("tier_parity.cljrs");
    let binary = temp.path().join("tier_parity_aot");
    std::fs::write(&script, PROGRAM).expect("write parity program");

    let tree = parse_records(Tier::TreeWalk, &run_cli(&script, Tier::TreeWalk));
    let eager_ir = parse_records(Tier::EagerIr, &run_cli(&script, Tier::EagerIr));
    let jit = parse_records(Tier::Jit, &run_cli(&script, Tier::Jit));
    let aot = parse_records(Tier::Aot, &run_aot(&script, &binary));

    assert_eq!(eager_ir, tree, "eager IR diverged from tree-walk");
    assert_eq!(jit, tree, "JIT diverged from tree-walk");
    assert_eq!(aot, tree, "AOT diverged from tree-walk");

    assert_eq!(tree["mod"], Outcome::Value("2".to_string()));
    assert_eq!(tree["mod-literal"], Outcome::Value("2".to_string()));
    assert_eq!(
        tree["literals"],
        Outcome::Value("[#\"[a-z]+\" 1/3 1.25M]".to_string())
    );
    assert_eq!(tree["recursive-aot"], Outcome::Value("55".to_string()));
    // The key is present, so the default loses — but it still ran, once per
    // call: 50 warm-up calls plus the hit and the miss below.
    assert_eq!(tree["or-default-hit"], Outcome::Value("7".to_string()));
    assert_eq!(tree["or-default-miss"], Outcome::Value("1".to_string()));
    assert_eq!(tree["or-default-effects"], Outcome::Value("52".to_string()));
    assert_eq!(tree["seq-loop"], Outcome::Value("[1 2 3 4]".to_string()));
    assert_eq!(tree["kwargs"], Outcome::Value("[1 5]".to_string()));
    assert_eq!(tree["kwargs-fixed"], Outcome::Value("[0 3]".to_string()));
    assert_eq!(tree["kwargs-map"], Outcome::Value("[2 4]".to_string()));
    assert_eq!(tree["seq-rest"], Outcome::Value("[1 2]".to_string()));
    assert_eq!(tree["kwargs-nil"], Outcome::Value("[nil 5]".to_string()));
    assert_eq!(tree["kwargs-meta-map"], Outcome::Value("[6 5]".to_string()));
    assert_eq!(
        tree["kwargs-meta-mixed"],
        Outcome::Value("[1 9]".to_string())
    );
    assert_eq!(
        tree["kwargs-odd"],
        Outcome::Error("No value supplied for key: :b".to_string())
    );
    assert_eq!(
        tree["overflow-add"],
        Outcome::Value("9223372036854775808N".to_string())
    );
    assert_eq!(
        tree["overflow-sub-low"],
        Outcome::Value("-9223372036854775809N".to_string())
    );
    assert_eq!(
        tree["overflow-sub-high"],
        Outcome::Value("9223372036854775808N".to_string())
    );
    assert_eq!(
        tree["overflow-mul"],
        Outcome::Value("18446744073709551614N".to_string())
    );
    assert_eq!(
        tree["arithmetic-error"],
        Outcome::Error("mod by zero".to_string())
    );
}
