//! Reader conditionals must resolve the same way in both evaluators.
//!
//! `eval_async` re-implements the container and binding-vector arms that
//! `eval` has, so every position that resolves `#?`/`#?@` in one has to
//! resolve it in the other. These tests pin that parity: for each source, the
//! async evaluator must produce exactly what the sync evaluator produces, and
//! both must match the hand-inlined spelling.
//!
//! Since virtual-threads Phase 5 these cases run on fibers (`fiber::eval_form`);
//! the colored `eval_async` evaluator they first targeted is wasm32-only.

use std::sync::Arc;

use cljrs_async::fiber::eval_form;
use cljrs_reader::Parser;
use cljrs_runtime::env::env::{Env, GlobalEnv};

fn async_env() -> Arc<GlobalEnv> {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(cljrs_runtime::ExecutionMode::TreeWalk)
        .eager_clojure_test(true)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    globals
}

fn parse_one(src: &str) -> Result<cljrs_reader::Form, String> {
    let mut p = Parser::new(src.to_string(), "<test>".to_string());
    p.parse_one()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no form".to_string())
}

fn block_on_local<F: std::future::Future>(f: F) -> F::Output {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("build runtime");
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, f)
}

/// Evaluate `src` with the sync tree-walker and with `eval_async`, returning
/// both outcomes as printable results so errors compare as values.
fn eval_both(src: &str) -> (Result<String, String>, Result<String, String>) {
    let form = match parse_one(src) {
        Ok(f) => f,
        // A read-time rejection is shared by both evaluators.
        Err(e) => return (Err(e.clone()), Err(e)),
    };

    let globals = async_env();
    let mut sync_env = Env::new(globals.clone(), "user");
    let sync = cljrs_runtime::interp::eval::eval(&form, &mut sync_env)
        .map(|v| format!("{v}"))
        .map_err(|e| e.to_string());

    let asynced = block_on_local(async move {
        let mut env = Env::new(globals, "user");
        eval_form(&form, &mut env)
            .await
            .map(|v| format!("{v}"))
            .map_err(|e| e.to_string())
    });

    (sync, asynced)
}

fn assert_parity(spliced: &str, inlined: &str) {
    let (sync_spliced, async_spliced) = eval_both(spliced);
    let (sync_inlined, _) = eval_both(inlined);
    assert_eq!(
        sync_spliced, async_spliced,
        "sync/async disagree on {spliced}"
    );
    assert_eq!(
        sync_spliced, sync_inlined,
        "{spliced} does not match inlined {inlined}"
    );
}

// ── examples ──────────────────────────────────────────────────────────────

#[test]
fn async_let_binding_vector_expands_splices() {
    assert_parity(
        "(let* [#?@(:rust [x 1 y 2])] [x y])",
        "(let* [x 1 y 2] [x y])",
    );
}

#[test]
fn async_loop_binding_vector_expands_splices() {
    assert_parity("(loop* [#?@(:rust [x 3])] x)", "(loop* [x 3] x)");
}

#[test]
fn async_map_literal_splices_into_key_value_positions() {
    assert_parity("{:a 1 #?@(:rust [:b 2]) :c 3}", "{:a 1 :b 2 :c 3}");
}

#[test]
fn async_map_literal_with_odd_expansion_is_rejected_by_both() {
    let (sync, asynced) = eval_both("{:a 1 #?@(:rust [:b]) :c 3}");
    assert!(sync.is_err(), "sync accepted an odd map: {sync:?}");
    assert_eq!(sync, asynced);
}

#[test]
fn async_unmatched_conditional_in_map_reads_as_empty() {
    assert_parity("{#?(:clj :a)}", "{}");
}

// ── properties ────────────────────────────────────────────────────────────

/// Render a splice of `n` elements starting at `start`, plus the inlined
/// spelling of the same elements. `matches` picks the branch key, so a
/// non-matching splice must contribute nothing.
fn splice(start: usize, n: usize, matches: bool) -> (String, String) {
    let ks: Vec<String> = (start..start + n).map(|i| format!(":k{i}")).collect();
    let key = if matches { "rust" } else { "clj" };
    let inlined = if matches { ks.join(" ") } else { String::new() };
    (format!("#?@(:{key} [{}])", ks.join(" ")), inlined)
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(24))]

    /// In every container the async evaluator handles, a splice at an arbitrary
    /// position contributes exactly what inlining it contributes - and agrees
    /// with the sync evaluator. Element counts are even so map literals stay
    /// legal in the same generated shape as the other containers.
    #[test]
    fn prop_async_container_splice_matches_sync_and_inline(
        pre in 0usize..3, mid in 0usize..3, suf in 0usize..3, matches in proptest::bool::ANY,
    ) {
        let (pre, mid, suf) = (pre * 2, mid * 2, suf * 2);
        let head: Vec<String> = (0..pre).map(|i| format!(":k{i}")).collect();
        let (spliced_mid, inlined_mid) = splice(pre, mid, matches);
        let tail: Vec<String> = (pre + mid..pre + mid + suf).map(|i| format!(":k{i}")).collect();

        for (open, close) in [("[", "]"), ("#{", "}"), ("{", "}")] {
            let spliced = format!(
                "{open}{} {spliced_mid} {}{close}", head.join(" "), tail.join(" ")
            );
            let inlined = format!(
                "{open}{} {inlined_mid} {}{close}", head.join(" "), tail.join(" ")
            );
            assert_parity(&spliced, &inlined);
        }
    }

    /// The same law for `let*` and `loop*` binding vectors, which the async
    /// evaluator resolves on its own path.
    #[test]
    fn prop_async_binding_vector_splice_matches_sync_and_inline(
        pre in 0usize..2, mid in 0usize..3, matches in proptest::bool::ANY,
    ) {
        let head: Vec<String> = (0..pre).flat_map(|i| [format!("x{i}"), format!("{i}")]).collect();
        let names: Vec<String> = (0..pre).map(|i| format!("x{i}")).collect();
        let mid_pairs: Vec<String> = (pre..pre + mid)
            .flat_map(|i| [format!("x{i}"), format!("{i}")])
            .collect();
        let mid_names: Vec<String> = (pre..pre + mid).map(|i| format!("x{i}")).collect();
        let key = if matches { "rust" } else { "clj" };

        let bound: Vec<String> = if matches {
            names.iter().chain(mid_names.iter()).cloned().collect()
        } else {
            names.clone()
        };
        let body = format!("[{}]", bound.join(" "));

        for head_form in ["let*", "loop*"] {
            let spliced = format!(
                "({head_form} [{} #?@(:{key} [{}])] {body})", head.join(" "), mid_pairs.join(" ")
            );
            let inlined_bindings = if matches {
                format!("{} {}", head.join(" "), mid_pairs.join(" "))
            } else {
                head.join(" ")
            };
            let inlined = format!("({head_form} [{inlined_bindings}] {body})");
            assert_parity(&spliced, &inlined);
        }
    }
}
