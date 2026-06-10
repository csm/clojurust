//! Phase 10.6 — specialization & inline caches, end to end.
//!
//! Four properties, one test each:
//!
//! 1. **Primitive unboxing on stable type profiles** — a hot numeric loop
//!    whose parameter profiled monomorphically `Long` compiles to native
//!    code that keeps `i64`s in registers: the boxed arithmetic bridge
//!    counters (`rt_add`/`rt_lt`/…) stay nearly flat compared to the same
//!    workload with specialization disabled (`CLJRS_JIT_NO_SPEC=1`).
//!
//! 2. **Deoptimization** — calling the specialized function with a `Double`
//!    fails the entry type guard, returns the deopt sentinel, and re-runs at
//!    Tier 1 with the correct result; repeated violations discard the
//!    specialization and the function keeps answering correctly.
//!
//! 3. **Keyword-lookup inline cache** — `(:k m)` sites cache the interned
//!    keyword in a per-site slot; the fill path runs once per site, not once
//!    per iteration (previously every execution heap-allocated a fresh
//!    keyword).
//!
//! 4. **Protocol-dispatch inline cache** — protocol method calls from native
//!    code hit the per-site `(callee, type-tag, generation) → impl` cache,
//!    and re-extending the protocol (generation bump) invalidates it.
//!
//! Each script runs far past the JIT threshold so background compilation
//! reliably publishes native code mid-run; per-iteration correctness is
//! asserted regardless, so a missed promotion can only weaken the stats
//! assertions, never produce wrong results.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Run `src` through `cljrs run` with the JIT forced on at a low threshold.
/// Returns `(stdout, jit_stats)`.
fn run_jit(src: &str, extra_env: &[(&str, &str)]) -> (String, String) {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let base = std::env::temp_dir().join(format!(
        "cljrs_jit_spec_{}_{nanos}_{seq}",
        std::process::id()
    ));
    let script = base.with_extension("cljrs");
    let stats = base.with_extension("jitstats");
    std::fs::write(&script, src).expect("write script");

    let mut cmd = Command::new(env!("CARGO_BIN_EXE_cljrs"));
    cmd.args(["--jit-threshold", "50", "--jit-stats"])
        .arg(&stats)
        .arg("run")
        .arg(&script)
        .env("CLJRS_EAGER_LOWER", "1");
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("spawn cljrs");

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

/// Parse `"  <label>:  N"` from a JIT stats dump.
fn stat(stats: &str, label: &str) -> u64 {
    stats
        .lines()
        .find(|l| l.contains(label))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

#[test]
fn monomorphic_long_profile_unboxes_hot_loop_arithmetic() {
    // hot-sum(200) does ~600 arithmetic/comparison ops per call.  Without
    // specialization the JIT'd code calls the boxed bridges for every one of
    // them (~12M over the run); with the parameter specialized to Long the
    // loop runs entirely in registers.
    let src = r#"
        (defn hot-sum [n]
          (loop [i 0 acc 0]
            (if (< i n)
              (recur (+ i 1) (+ acc i))
              acc)))
        (dotimes [i 20000]
          (when (not= (hot-sum 200) 19900)
            (println "WRONG at" i)))
        (println "sum:" (hot-sum 200))
    "#;

    let (out_spec, stats_spec) = run_jit(src, &[]);
    assert!(!out_spec.contains("WRONG at"), "got:\n{out_spec}");
    assert!(out_spec.contains("sum: 19900"), "got:\n{out_spec}");

    let (out_nospec, stats_nospec) = run_jit(src, &[("CLJRS_JIT_NO_SPEC", "1")]);
    assert!(!out_nospec.contains("WRONG at"), "got:\n{out_nospec}");

    let boxed_spec = stat(&stats_spec, "Boxed arith calls");
    let boxed_nospec = stat(&stats_nospec, "Boxed arith calls");
    assert!(
        boxed_spec * 5 < boxed_nospec,
        "specialization should eliminate the vast majority of boxed \
         arithmetic bridge calls; specialized = {boxed_spec}, \
         unspecialized = {boxed_nospec}\nspec stats:\n{stats_spec}\n\
         nospec stats:\n{stats_nospec}"
    );
}

#[test]
fn type_guard_violation_deopts_to_tier1_with_correct_results() {
    // hot-sum promotes with a pure-Long profile, then gets hammered with
    // Doubles: every such call must fail the entry guard, re-run at Tier 1,
    // and produce the exact Tier-1 answer.  Past the deopt limit (default
    // 10) the specialization is discarded — and Long calls must keep
    // answering correctly afterwards (interpreted or generically
    // recompiled).
    let src = r#"
        (defn hot-sum [n]
          (loop [i 0 acc 0]
            (if (< i n)
              (recur (+ i 1) (+ acc i))
              acc)))
        (dotimes [i 5000]
          (when (not= (hot-sum 100) 4950)
            (println "WRONG-LONG at" i)))
        (dotimes [i 50]
          (when (not= (hot-sum 10.5) 55)
            (println "WRONG-DOUBLE at" i)))
        (dotimes [i 5000]
          (when (not= (hot-sum 100) 4950)
            (println "WRONG-AFTER at" i)))
        (println "long:" (hot-sum 100) "double:" (hot-sum 10.5))
    "#;

    let (out, stats) = run_jit(src, &[]);
    assert!(
        !out.contains("WRONG-LONG at"),
        "pre-deopt Long results wrong; got:\n{out}"
    );
    assert!(
        !out.contains("WRONG-DOUBLE at"),
        "guard-failing Double calls must produce Tier-1 results; got:\n{out}"
    );
    assert!(
        !out.contains("WRONG-AFTER at"),
        "Long calls after the specialization was discarded went wrong; got:\n{out}"
    );
    assert!(out.contains("long: 4950 double: 55"), "got:\n{out}");

    // The Double calls hit the guard until the limit discarded the
    // specialization; at least the limit's worth of deopts must register.
    // (If promotion never landed, deopts are 0 — but then the correctness
    // asserts above already covered Tier 1; require the promotion here so
    // the deopt path is genuinely exercised.)
    let deopts = stat(&stats, "Guard deopts");
    assert!(
        deopts >= 10,
        "expected the Double calls to fail the entry guard at least up to \
         the deopt limit; deopts = {deopts}\nstats:\n{stats}"
    );
}

#[test]
fn keyword_constants_fill_their_inline_cache_once_per_site() {
    // (:x m) / (:y m) compile to per-site keyword IC slots: the fill path
    // (which interns + stores the keyword) runs once per call site, while
    // the remaining ~tens-of-thousands of executions take the inline
    // load+branch.  Before Phase 10.6 every execution called
    // rt_const_keyword and heap-allocated a fresh keyword.
    let src = r#"
        (defn pick [m]
          (+ (:x m) (:y m)))
        (def point {:x 3 :y 4})
        (dotimes [i 20000]
          (when (not= (pick point) 7)
            (println "WRONG at" i)))
        (println "picked:" (pick point))
    "#;

    let (out, stats) = run_jit(src, &[]);
    assert!(!out.contains("WRONG at"), "got:\n{out}");
    assert!(out.contains("picked: 7"), "got:\n{out}");

    let fills = stat(&stats, "Keyword IC fills");
    assert!(
        fills >= 1,
        "native code never took the keyword IC path (did promotion land?); \
         stats:\n{stats}"
    );
    assert!(
        fills <= 64,
        "keyword IC must fill once per call site, not per iteration; \
         fills = {fills}\nstats:\n{stats}"
    );
}

#[test]
fn protocol_dispatch_hits_inline_cache_and_reextension_invalidates_it() {
    // `describe` dispatches on the argument type through a ProtocolFn.  The
    // hot caller compiles to native code whose call site caches the
    // (callee, type-tag, generation) → impl resolution.  Re-extending the
    // protocol bumps the global generation: the stale cache entry must be
    // invalidated and the new impl picked up at the same call site.
    let src = r#"
        (defprotocol Describe
          (describe [x]))
        (extend-type Long
          Describe
          (describe [x] (* x 2)))
        (defn poke [x] (describe x))
        (dotimes [i 20000]
          (when (not= (poke 21) 42)
            (println "WRONG at" i)))
        (println "before:" (poke 21))
        (extend-type Long
          Describe
          (describe [x] (* x 10)))
        (dotimes [i 2000]
          (when (not= (poke 21) 210)
            (println "STALE at" i ":" (poke 21))))
        (println "after:" (poke 21))
    "#;

    let (out, stats) = run_jit(src, &[]);
    assert!(!out.contains("WRONG at"), "got:\n{out}");
    assert!(out.contains("before: 42"), "got:\n{out}");
    assert!(
        !out.contains("STALE at"),
        "protocol IC kept dispatching the old impl after re-extension; got:\n{out}"
    );
    assert!(out.contains("after: 210"), "got:\n{out}");

    let hits = stat(&stats, "Protocol IC hits");
    assert!(
        hits >= 100,
        "hot protocol dispatch should hit the inline cache; hits = {hits}\n\
         stats:\n{stats}"
    );
}
