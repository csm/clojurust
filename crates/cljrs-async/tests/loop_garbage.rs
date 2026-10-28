//! Regression test for csm/clojurust#428: a garbage collection during a
//! `loop*`/`recur` loop must free the dead intermediates of earlier
//! iterations.
//!
//! Two defects kept them alive.  The async evaluator, which drives every
//! top-level form of `cljrs run`/`eval`, gave `loop*` iterations no alloc
//! frame, so each iteration's allocations stayed rooted until the whole form
//! finished.  And a collection that found garbage freed none of it (the sweep
//! keeps unreachable objects for one cycle of grace) and so counted as zero
//! yield: the governor then set the next collection target at twice the heap,
//! past the hard limit, and the collection that would have freed the garbage
//! never ran.
//!
//! Collections here are requested only by the process memory governor, as in
//! the CLI.  This binary owns the governor, so it has one test.

#![cfg(not(feature = "no-gc"))]

use cljrs_gc::HEAP;
use cljrs_gc::governor::{self, MemoryConfig};
use cljrs_reader::{Form, Parser};
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::env::Env;
use cljrs_value::Value;

const MIB: usize = 1024 * 1024;

/// Iterations per loop.  Each `conj` allocates a new vector version that is
/// dead by the next iteration; together they are several times the budget.
const N: i64 = 12_000;

fn forms(src: &str) -> Vec<Form> {
    Parser::new(src.to_owned(), "<loop-garbage>".to_owned())
        .parse_all()
        .expect("parse")
}

fn env(mode: ExecutionMode) -> Env {
    let globals = cljrs_runtime::Runtime::builder()
        .execution_mode(mode)
        .build()
        .expect("runtime")
        .into_globals();
    cljrs_async::init(&globals);
    Env::new(globals, "user")
}

/// Evaluate `src` with `eval` and assert that the loop it runs returns `N`
/// and that collections during it freed at least half of its iterations'
/// vector versions.
fn assert_frees(label: &str, src: &str, mut eval: impl FnMut(&Form) -> Value) {
    let mut forms = forms(src);
    let last = forms.pop().expect("a form");
    for form in &forms {
        eval(form);
    }
    let freed_before = HEAP.total_freed();
    assert_eq!(eval(&last), Value::Long(N), "{label}: loop result");
    let freed = HEAP.total_freed() - freed_before;
    assert!(
        freed as i64 >= N / 2,
        "{label}: collections freed {freed} objects, expected at least {}",
        N / 2
    );
}

#[test]
fn loop_intermediates_are_collected() {
    let g = governor::governor();
    let hard = g.committed_bytes() + 16 * MIB;
    g.configure(MemoryConfig::with_limits(hard - 4 * MIB, hard).unwrap())
        .unwrap();

    let top_level =
        format!("(loop* [acc [] i 0] (if (< i {N}) (recur (conj acc i) (inc i)) (count acc)))");
    let in_fn = format!(
        "(defn build [n] (loop* [acc [] i 0] (if (< i n) (recur (conj acc i) (inc i)) (count acc))))
         (build {N})"
    );

    // Tree-walking interpreter.
    let mut tw = env(ExecutionMode::TreeWalk);
    assert_frees("tree-walk top level", &top_level, |f| {
        cljrs_runtime::tiered::eval(f, &mut tw).expect("eval")
    });
    assert_frees("tree-walk fn", &in_fn, |f| {
        cljrs_runtime::tiered::eval(f, &mut tw).expect("eval")
    });

    // Tiered interpreter, with `build` lowered to IR on definition.
    cljrs_runtime::tiered::force_eager_lowering();
    let mut tiered = env(ExecutionMode::TieredNoJit);
    assert_frees("tiered fn", &in_fn, |f| {
        cljrs_runtime::tiered::eval(f, &mut tiered).expect("eval")
    });

    // Async evaluator, as the CLI drives each top-level form.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let local = tokio::task::LocalSet::new();
    let mut async_env = env(ExecutionMode::TreeWalk);
    assert_frees("async top level", &top_level, |f| {
        local
            .block_on(&rt, cljrs_async::eval_async::eval_async(f, &mut async_env))
            .expect("eval")
    });
}
