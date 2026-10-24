//! Strict managed-memory admission (governor Phase 4) seen from the
//! interpreter.
//!
//! This binary owns the process memory governor, so it has one test.

#![cfg(not(feature = "no-gc"))]

use cljrs_gc::governor::{self, MemoryConfig};
use cljrs_reader::Parser;
use cljrs_runtime::ExecutionMode;
use cljrs_runtime::env::env::Env;
use cljrs_runtime::env::error::EvalError;
use cljrs_value::{Keyword, Value};

const MIB: usize = 1024 * 1024;

#[allow(clippy::result_large_err)]
fn eval(src: &str, env: &mut Env) -> Result<Value, EvalError> {
    let form = Parser::new(src.to_owned(), "<memory-limit>".to_owned())
        .parse_all()
        .expect("parse")
        .remove(0);
    cljrs_runtime::tiered::eval(&form, env)
}

fn env(mode: ExecutionMode) -> Env {
    Env::new(
        cljrs_runtime::Runtime::builder()
            .execution_mode(mode)
            .build()
            .expect("runtime")
            .into_globals(),
        "user",
    )
}

/// Garbage that a collection reclaims does not fail an evaluation; live
/// data past the hard limit does, without aborting the process, and
/// `try`/`catch` cannot intercept it.  The runtime stays usable afterwards.
fn exercise(mode: ExecutionMode) {
    let mut env = env(mode);
    let g = governor::governor();
    let hard = g.committed_bytes() + 8 * MIB;
    let mut config = MemoryConfig::with_limits(hard / 2, hard).unwrap();
    config.critical_reserve = 8 * MIB;
    config.strict = true;
    g.configure(config).unwrap();

    // Several times the 8 MiB budget in short-lived strings.  Builtins
    // only: loading `clojure.core` is slow in debug builds.
    let garbage = format!(
        "(loop* [i 0] (if (< i 40000) (do (str i \"{}\") (recur (inc i))) :done))",
        "0123456789".repeat(100)
    );
    assert_eq!(
        eval(&garbage, &mut env).expect("garbage is collected"),
        Value::keyword(Keyword::simple("done"))
    );

    // `cons` cells: a persistent vector's accounted size counts every element
    // for each version, which would make each `conj` a large allocation.
    let retained = "(loop* [acc nil i 0] \
                      (if (< i 20000000) (recur (cons (str i) acc) (inc i)) i))";
    match eval(retained, &mut env) {
        Err(EvalError::MemoryLimitExceeded(fault)) => {
            assert_eq!(fault.hard_limit, hard);
            assert!(fault.to_string().contains("managed memory limit exceeded"));
        }
        other => panic!("{mode:?}: expected MemoryLimitExceeded, got {other:?}"),
    }

    let caught = format!("(try {retained} (catch :default e :caught))");
    assert!(
        matches!(
            eval(&caught, &mut env),
            Err(EvalError::MemoryLimitExceeded(_))
        ),
        "{mode:?}: the fault is not catchable"
    );
    assert!(governor::pending_runtime_fault().is_none());

    assert_eq!(eval("(+ 1 2)", &mut env).unwrap(), Value::Long(3));
    let snap = g.snapshot();
    assert!(snap.runtime_faults >= 2, "{snap}");
    assert!(snap.reserve_overruns == 0, "{snap}");

    g.configure(MemoryConfig::with_limits(256 * MIB, 512 * MIB).unwrap())
        .unwrap();
}

#[test]
fn strict_mode_reports_memory_limit_in_both_interpreters() {
    std::thread::Builder::new()
        .stack_size(64 * MIB)
        .spawn(|| {
            exercise(ExecutionMode::TreeWalk);
            exercise(ExecutionMode::TieredNoJit);
        })
        .unwrap()
        .join()
        .unwrap();
}
