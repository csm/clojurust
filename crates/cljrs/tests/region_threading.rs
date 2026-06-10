//! Phase 10.5 — context-driven bump allocation across the JIT call boundary.
//!
//! `make-pair` is deliberately non-inlineable (inner `fn`), so the only way
//! its `[a b]` allocation reaches a region is stage-4 cross-function
//! promotion: the caller's lowering clones a region-parameterised variant and
//! rewrites the call to `CallWithRegion`, threading its region handle as a
//! hidden trailing argument once the caller is JIT-compiled.
//!
//! In the script/REPL flow each `defn` lowers separately, so this only works
//! through the cross-defn registry (`cljrs_eval::defn_registry`): `make-pair`
//! is registered when defined, and `use-pair`'s lowering consumes it as an
//! external.  That cloning makes redefinition correctness the critical
//! property — the second test redefines the callee mid-run and asserts the
//! caller picks up the new definition.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `src` through `cljrs run` with the JIT forced on at a low threshold.
/// Returns `(stdout, gc_stats)`.
fn run_jit(src: &str) -> (String, String) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "cljrs_region_threading_{}_{nanos}_{seq}",
        std::process::id()
    ));
    let script = base.with_extension("cljrs");
    let stats = base.with_extension("gcstats");
    std::fs::write(&script, src).expect("write script");

    let output = Command::new(env!("CARGO_BIN_EXE_cljrs"))
        .args(["--jit-threshold", "50", "--gc-stats"])
        .arg(&stats)
        .arg("run")
        .arg(&script)
        .env("CLJRS_EAGER_LOWER", "1")
        .output()
        .expect("spawn cljrs");

    let stats_out = std::fs::read_to_string(&stats).unwrap_or_default();
    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_file(&stats);

    assert!(
        output.status.success(),
        "cljrs exited with {:?}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    (
        String::from_utf8(output.stdout).expect("utf8 stdout"),
        stats_out,
    )
}

/// Parse `"  Region (bump) allocs:  N (M bytes)"` from a GC stats dump.
fn region_alloc_count(stats: &str) -> u64 {
    stats
        .lines()
        .find(|l| l.contains("Region (bump) allocs:"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn cross_defn_calls_bump_allocate_into_the_callers_region() {
    let src = r#"
        (defn make-pair [a b]
          (let [f (fn [x] x)]
            [a b]))
        (defn use-pair [x] (count (make-pair x x)))
        (dotimes [i 20000]
          (let [c (use-pair i)]
            (when (not= c 2)
              (println "WRONG at" i ":" c))
            (when (= i 19999)
              (println "count:" c))))
    "#;

    let (out, stats) = run_jit(src);
    assert!(
        !out.contains("WRONG at"),
        "cross-defn region promotion broke call results; got:\n{out}"
    );
    assert!(out.contains("count: 2"), "got:\n{out}");

    // The callee's vector must actually land in the caller's bump region —
    // tens of thousands of region allocations, not a handful.
    let regions = region_alloc_count(&stats);
    assert!(
        regions >= 10_000,
        "expected the hot cross-defn call to bump-allocate per iteration; \
         region allocs = {regions}\nstats:\n{stats}"
    );
}

#[test]
fn redefining_a_region_promoted_callee_invalidates_its_callers() {
    // Stage 4 clones the callee's body into the caller, so without
    // invalidation the caller would keep returning the OLD definition's
    // result after the redefinition (even at Tier 1, and worse once native).
    let src = r#"
        (defn make-pair [a b]
          (let [f (fn [x] x)]
            [a b]))
        (defn use-pair [x] (count (make-pair x x)))
        (dotimes [i 20000]
          (when (not= (use-pair i) 2)
            (println "WRONG-BEFORE at" i)))
        (defn make-pair [a b]
          (let [f (fn [x] x)]
            [a b a]))
        (dotimes [i 2000]
          (when (not= (use-pair i) 3)
            (println "STALE at" i ":" (use-pair i))))
        (println "after-redef:" (use-pair 1))
    "#;

    let (out, _stats) = run_jit(src);
    assert!(
        !out.contains("WRONG-BEFORE"),
        "pre-redefinition results wrong; got:\n{out}"
    );
    assert!(
        !out.contains("STALE at"),
        "caller kept executing the old callee body after redefinition; got:\n{out}"
    );
    assert!(out.contains("after-redef: 3"), "got:\n{out}");
}
