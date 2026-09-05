//! PR #353, second half: a parameter sharing the function's OWN name must
//! shadow it, and must do so identically in every execution tier.
//!
//! The self-reference and the params bind into the same frame; binding the
//! self-reference last overwrote a param that shared the function's name, so
//! `(defn text [text] {:text text})` returned the function as its own `:text`.
//!
//! The ordering fix was applied in two places — `interp::apply::call_cljrs_fn`
//! and `tiered::apply::execute_ir`. Only the first is observable today: in the
//! IR tier a plain parameter is a register, and `Inst::LoadLocal` (which reads
//! the env `execute_ir` populates) is emitted only for closed-over locals, so
//! reverting the tiered half alone changes no result I could construct —
//! destructured, variadic, multi-arity and closure-capturing params included.
//! The tiered half is therefore consistency insurance rather than a live fix:
//! `Inst::LoadLocal` may name a parameter (see `tests/region_phi_uaf.rs`), and
//! if lowering ever emits one, these tests begin discriminating on it.
//!
//! What they pin down now is cross-tier parity: the same script through the CLI
//! under tree-walk and under forced eager IR lowering (`CLJRS_EAGER_LOWER=1`,
//! the repo's established way to reach Tier 1 — see `jit_specialization.rs`,
//! `execution_tier_parity.rs`) must agree, and must agree with Clojure.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const PROGRAM: &str = r#"
(defn text [text] {:text text})
(println (str "defn|" (pr-str (:text (text "hi")))))

(defn f ([] :no-args) ([f] f))
(println (str "arity-0|" (pr-str (f))))
(println (str "arity-1|" (pr-str (f 3))))

(defn xs [& xs] xs)
(println (str "rest|" (pr-str (xs 1 2))))

;; Drive it well past the warm threshold so the eager-IR run really dispatches
;; through the tier, and assert the answer is stable across every call.
(println (str "hot|" (pr-str (distinct (map (fn [i] (:text (text i))) (range 100))))))
"#;

const HOT: &str = "hot|(0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 \
25 26 27 28 29 30 31 32 33 34 35 36 37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 \
55 56 57 58 59 60 61 62 63 64 65 66 67 68 69 70 71 72 73 74 75 76 77 78 79 80 81 82 83 84 \
85 86 87 88 89 90 91 92 93 94 95 96 97 98 99)";

const EXPECTED: &[&str] = &[
    r#"defn|"hi""#,
    "arity-0|:no-args",
    "arity-1|3",
    "rest|(1 2)",
    HOT,
];

/// Run PROGRAM through `cljrs run`, optionally with eager IR lowering forced.
fn run(eager_ir: bool) -> String {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "cljrs_self_shadow_{}_{nanos}_{seq}.cljrs",
        std::process::id()
    ));
    std::fs::write(&path, PROGRAM).expect("write script");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cljrs"));
    cmd.env_remove("CLJRS_NO_IR")
        .env_remove("CLJRS_IR_THRESHOLD")
        .env_remove("CLJRS_JIT_THRESHOLD")
        .args(["--ir-threshold", "0", "--jit-threshold", "0", "run"])
        .arg(&path);
    if eager_ir {
        cmd.env("CLJRS_EAGER_LOWER", "1");
    } else {
        cmd.env_remove("CLJRS_EAGER_LOWER");
    }
    let output = cmd.output().expect("spawn cljrs");
    let _ = std::fs::remove_file(&path);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf8 stdout")
}

fn assert_all(stdout: &str, tier: &str) {
    for expected in EXPECTED {
        assert!(
            stdout.lines().any(|l| l.trim_end() == *expected),
            "{tier}: missing `{expected}`\nfull stdout:\n{stdout}"
        );
    }
}

#[test]
fn param_shadows_the_fns_own_name_tree_walk() {
    assert_all(&run(false), "tree-walk");
}

#[test]
fn param_shadows_the_fns_own_name_eager_ir() {
    assert_all(&run(true), "eager-IR");
}
